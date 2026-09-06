//! End-to-end industrial orchestration scenarios.
//!
//! Covers:
//! 1. Multi-tier diamond DAG execution with fan-out and fan-in.
//! 2. Content-addressable storage (CAS) deduplication, integrity, and GC.
//! 3. High-concurrency tenant throttling with backoff recovery.
//! 4. Static simulation and wave decomposition on multi-branch workflows.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use runvane::clock::ManualClock;
use runvane::domain::ids::{HandlerId, RunId, WorkflowId};
use runvane::domain::retry_policy::RetryPolicy;
use runvane::domain::run::Run;
use runvane::domain::status::{Priority, RunStatus, TaskStatus};
use runvane::domain::workflow::{Hooks, TaskSpec, WorkflowDef, SPEC_VERSION};
use runvane::engine::dry_run::simulate_workflow;
use runvane::persistence::memory::MemoryStore;
use runvane::persistence::Store;
use runvane::plugins::handler::Registry;
use runvane::scheduler::executor::RunExecutor;
use runvane::scheduler::throttle::ConcurrencyLimiter;
use runvane::storage::artifacts::{ArtifactStore, MemoryArtifactStore};
use runvane::storage::gc::{sweep_artifacts, ArtifactGcOptions};
use serde_json::json;

fn make_test_def(name: &str, tasks: Vec<TaskSpec>) -> WorkflowDef {
    WorkflowDef {
        id: WorkflowId::from_validated(format!("wf_{name}")),
        tenant: "acme".into(),
        name: name.into(),
        version: 1,
        description: "e2e pipeline".into(),
        tasks,
        timeout_ms: 60_000,
        retry: RetryPolicy::fixed(3, 50),
        default_priority: Priority::default(),
        hooks: Hooks::default(),
        tags: BTreeMap::new(),
        spec_version: SPEC_VERSION,
        created_at_ms: 1_000,
        updated_at_ms: 1_000,
    }
}

fn make_task(name: &str, handler: &str, deps: &[&str]) -> TaskSpec {
    TaskSpec {
        name: name.to_owned(),
        handler: HandlerId::parse(handler).unwrap(),
        input: json!({"step": name}),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        timeout_ms: Some(10_000),
        retry: None,
        meta: BTreeMap::new(),
    }
}

#[test]
fn diamond_dag_fanout_fanin_pipeline() {
    let clock = Arc::new(ManualClock::at(1_000));
    let store = Arc::new(MemoryStore::new());
    let registry = Arc::new(Registry::new());
    registry.register(
        HandlerId::parse("runvane.echo").unwrap(),
        Arc::new(runvane::plugins::handler::EchoHandler),
    );

    // DAG: start -> [metric_proc, log_proc] -> aggregate
    let tasks = vec![
        make_task("start", "runvane.echo", &[]),
        make_task("metric_proc", "runvane.echo", &["start"]),
        make_task("log_proc", "runvane.echo", &["start"]),
        make_task("aggregate", "runvane.echo", &["metric_proc", "log_proc"]),
    ];
    let def = make_test_def("diamond_pipeline", tasks);
    store.put_workflow(def.clone()).unwrap();

    let rid = RunId::from_validated("rn_diamond00000000".to_owned());
    let run = Run {
        id: rid.clone(),
        tenant: "acme".into(),
        def_name: "diamond_pipeline".into(),
        def_version: 1,
        input: json!({"probe": true}),
        status: RunStatus::Running,
        attempts: 1,
        next_attempt_at_ms: None,
        deadline_at_ms: None,
        started_at_ms: Some(1_000),
        finished_at_ms: None,
        error: None,
        output: None,
        tags: BTreeMap::new(),
        created_at_ms: 1_000,
        run_number: 1,
    };
    store.put_run(&run).unwrap();

    let executor = RunExecutor::new(store.as_ref(), registry, clock.as_ref(), 42);
    let outcome = executor.attempt(rid.as_str()).unwrap();

    assert_eq!(outcome.run_status, RunStatus::Succeeded);

    let final_run = store.get_run(&rid).unwrap();
    assert_eq!(final_run.status, RunStatus::Succeeded);

    let tasks = store.list_task_runs_for_run(&rid).unwrap();
    assert_eq!(tasks.len(), 4);
    for t in &tasks {
        assert_eq!(t.status, TaskStatus::Succeeded);
    }
}

#[test]
fn cas_artifact_pipeline_with_integrity_and_gc() {
    let store = MemoryArtifactStore::new();
    let payload = b"{\"dataset\": [1, 2, 3, 4, 5], \"metadata\": \"telemetry\"}";

    // 1. Put artifact
    let desc1 = store
        .put("acme", "dataset.json", payload, "application/json", 1_000)
        .unwrap();

    // 2. Put same payload with another name: verify CAS deduplication (identical SHA-256 digest)
    let desc2 = store
        .put("acme", "copy.json", payload, "application/json", 2_000)
        .unwrap();
    assert_eq!(desc1.id, desc2.id);

    // 3. Verify content retrieval
    let retrieved = store.get(&desc1.id).unwrap().unwrap();
    assert_eq!(retrieved, payload);

    // 4. Garbage Collection test: simulate retention expiration
    let options = ArtifactGcOptions {
        retention_ms: 10_000,
        dry_run: false,
        tenants: vec!["acme".into()],
    };

    // Case A: referenced in active run -> protected from sweep
    let mut active_refs = BTreeSet::new();
    active_refs.insert(desc1.id.clone());
    let stats = sweep_artifacts(&store, &options, &active_refs, 20_000).unwrap();
    assert_eq!(stats.reclaimed_count, 0);
    assert!(store.get(&desc1.id).unwrap().is_some());

    // Case B: reference released -> swept and reclaimed
    active_refs.clear();
    let stats = sweep_artifacts(&store, &options, &active_refs, 20_000).unwrap();
    assert_eq!(stats.reclaimed_count, 1);
    assert_eq!(stats.reclaimed_bytes, payload.len());
    assert!(store.get(&desc1.id).unwrap().is_none());
}

#[test]
fn concurrency_limiter_tenant_fairness_and_release() {
    let limiter = ConcurrencyLimiter::new(2);

    // Acquire permits up to capacity
    let p1 = limiter.try_acquire("tenant_a");
    assert!(p1.is_ok());
    assert_eq!(limiter.active_count("tenant_a"), 1);

    let p2 = limiter.try_acquire("tenant_a");
    assert!(p2.is_ok());
    assert_eq!(limiter.active_count("tenant_a"), 2);

    // Third acquire for tenant_a must be throttled
    let p3 = limiter.try_acquire("tenant_a");
    assert!(p3.is_err());

    // Separate tenant is unaffected by tenant_a's usage
    let p_b = limiter.try_acquire("tenant_b");
    assert!(p_b.is_ok());
    assert_eq!(limiter.active_count("tenant_b"), 1);

    // Drop permit releases slot immediately
    drop(p1);
    assert_eq!(limiter.active_count("tenant_a"), 1);
    let p3_retry = limiter.try_acquire("tenant_a");
    assert!(p3_retry.is_ok());
}

#[test]
fn static_dry_run_diamond_simulation_and_metrics() {
    let tasks = vec![
        make_task("ingest", "runvane.echo", &[]),
        make_task("clean_a", "runvane.echo", &["ingest"]),
        make_task("clean_b", "runvane.echo", &["ingest"]),
        make_task("clean_c", "runvane.echo", &["ingest"]),
        make_task("export", "runvane.echo", &["clean_a", "clean_b", "clean_c"]),
    ];
    let def = make_test_def("wide_fanout", tasks);

    let report = simulate_workflow(&def).unwrap();
    assert_eq!(report.total_tasks, 5);
    assert_eq!(report.max_parallelism, 3);
    assert_eq!(report.stages.len(), 3);
    assert_eq!(report.stages[0].tasks, vec!["ingest"]);
    assert_eq!(
        report.stages[1].tasks,
        vec!["clean_a", "clean_b", "clean_c"]
    );
    assert_eq!(report.stages[2].tasks, vec!["export"]);
    assert_eq!(report.critical_path.len(), 3);
}
