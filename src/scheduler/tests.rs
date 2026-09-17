//! End-to-end scheduler tests: dispatcher + worker pool + executor over the
//! memory store with a manual clock.
//!
//! Outcomes are fully deterministic (seeded backoff, injected clock). The only
//! wall-clock dependency is a bounded 1 ms yield used to wait for worker
//! threads; that caps polling latency but never decides a result, so the suite
//! stays immune to load spikes.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::clock::ManualClock;
use crate::domain::ids::{HandlerId, RunId, WorkflowId};
use crate::domain::retry_policy::RetryPolicy;
use crate::domain::status::{Priority, RunStatus, TaskStatus};
use crate::domain::workflow::{Hooks, TaskSpec, WorkflowDef};
use crate::persistence::fixtures;
use crate::persistence::memory::MemoryStore;
use crate::persistence::{ClaimToken, QueueEntry, Store};
use crate::plugins::handler::{Handler, HandlerError, HandlerResult, Registry, TaskContext};
use crate::scheduler::pool::{Dispatcher, WorkerPool};

const ACK_DRAIN_READY_LABEL: &str = "queue entry ready";

/// Test-only handler that can be flaky or always-failing per task.
#[derive(Debug)]
struct Flaky {
    attempts: std::sync::Mutex<BTreeMap<String, u32>>,
    fail_count: u32,
    always_fail: bool,
}

impl Flaky {
    fn succeeds_after(fail_count: u32) -> Arc<Self> {
        Arc::new(Self {
            attempts: std::sync::Mutex::new(BTreeMap::new()),
            fail_count,
            always_fail: false,
        })
    }

    fn always_fail() -> Arc<Self> {
        Arc::new(Self {
            attempts: std::sync::Mutex::new(BTreeMap::new()),
            fail_count: 0,
            always_fail: true,
        })
    }
}

impl Handler for Flaky {
    fn execute(&self, ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError> {
        let mut guard = self.attempts.lock().unwrap();
        let count = guard.entry(ctx.task_name.to_owned()).or_insert(0);
        *count += 1;
        if self.always_fail || *count <= self.fail_count {
            Err(HandlerError::transient("intentional flake"))
        } else {
            Ok(HandlerResult {
                output: json!({ "task": ctx.task_name, "attempt": *count }),
            })
        }
    }

    fn description(&self) -> &'static str {
        "test flaky handler"
    }
}

fn task(name: &str, handler: &str, deps: &[&str]) -> TaskSpec {
    TaskSpec {
        name: name.to_owned(),
        handler: HandlerId::from_validated(handler.to_owned()),
        input: json!({}),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        timeout_ms: None,
        retry: None,
        meta: BTreeMap::new(),
    }
}

fn def(id: &str, name: &str, tasks: Vec<TaskSpec>, retry: RetryPolicy) -> WorkflowDef {
    WorkflowDef {
        id: WorkflowId::from_validated(id.to_owned()),
        tenant: "tenant1".to_owned(),
        name: name.to_owned(),
        version: 1,
        description: "scheduler e2e fixture".to_owned(),
        tasks,
        timeout_ms: 60_000,
        retry,
        default_priority: Priority::Normal,
        hooks: Hooks::default(),
        tags: BTreeMap::new(),
        spec_version: 1,
        created_at_ms: 1_000_000,
        updated_at_ms: 1_000_000,
    }
}

fn enqueue(store: &dyn Store, run_id: &RunId, at_ms: i64) {
    store
        .enqueue(QueueEntry {
            run_id: run_id.clone(),
            token: ClaimToken::empty(),
            due_at_ms: at_ms,
            lease_until_ms: None,
            claimed_by: None,
        })
        .unwrap();
}

/// Bounded poll; sleeps 1 ms between tries so a worker thread has a chance to
/// make progress. Only bounds latency, never decides an outcome.
fn poll<F: FnMut() -> bool>(mut ready: F, what: &str) {
    for _ in 0..5_000 {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {what}");
}

/// Waits until `run_id` leaves `Running` (i.e. the attempt settled).
fn wait_settled(store: &dyn Store, run_id: &RunId) {
    poll(
        || {
            store
                .get_run(run_id)
                .map(|r| r.status != RunStatus::Running)
                .unwrap_or(false)
        },
        "run attempt to settle",
    );
}

/// Waits until the queue entry is claimable at the clock's current `now`.
fn wait_ready(store: &dyn Store, clock: &ManualClock, run_id: &RunId) {
    poll(
        || {
            store
                .scan_ready(clock.value(), 16)
                .map(|v| v.iter().any(|e| e.run_id == *run_id))
                .unwrap_or(false)
        },
        ACK_DRAIN_READY_LABEL,
    );
}

#[track_caller]
fn run_status(store: &dyn Store, run_id: &RunId) -> RunStatus {
    store.get_run(run_id).unwrap().status
}

#[test]
fn dispatch_linear_run_succeeds() {
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(ManualClock::at(1_000_000));
    let registry = Arc::new(Registry::new());
    registry.register(
        HandlerId::from_validated("testgood0000".to_owned()),
        Flaky::succeeds_after(0),
    );

    let def_name = "linear0001";
    store
        .put_workflow(def(
            "wf_linear0001",
            def_name,
            vec![
                task("prepare", "testgood0000", &[]),
                task("publish", "testgood0000", &["prepare"]),
            ],
            RetryPolicy::fixed(2, 25),
        ))
        .unwrap();

    let rid = RunId::from_validated("rn_linear00000000".to_owned());
    let run = fixtures::run("rn_linear00000000", "tenant1", def_name, RunStatus::Queued, 1_000_000);
    store.put_run(&run).unwrap();
    enqueue(store.as_ref(), &rid, clock.value());

    let pool = WorkerPool::spawn(2, registry, store.clone(), clock.clone(), 42);
    let dispatcher = Dispatcher::new(store.as_ref(), &*clock, pool, 8, 60_000);

    let stats = dispatcher.step();
    assert_eq!(stats.scanned, 1);
    assert_eq!(stats.claimed, 1);

    poll(
        || run_status(store.as_ref(), &rid) == RunStatus::Succeeded,
        "run to succeed",
    );
    assert_eq!(run_status(store.as_ref(), &rid), RunStatus::Succeeded);

    let tasks = store.list_task_runs_for_run(&rid).unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|t| t.status == TaskStatus::Succeeded));
    assert_eq!(store.len_queue(), 0);

    dispatcher.into_pool().shutdown();
}

#[test]
fn two_runs_dispatch_in_parallel_and_succeed() {
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(ManualClock::at(2_000_000));
    let registry = Arc::new(Registry::new());
    registry.register(
        HandlerId::from_validated("testgood0000".to_owned()),
        Flaky::succeeds_after(0),
    );

    let def_name = "parallel0001";
    store
        .put_workflow(def(
            "wf_parallel0001",
            def_name,
            vec![task("work", "testgood0000", &[])],
            RetryPolicy::fixed(1, 25),
        ))
        .unwrap();

    let rid_a = RunId::from_validated("rn_parallel000000a".to_owned());
    let rid_b = RunId::from_validated("rn_parallel000000b".to_owned());
    for rid in [&rid_a, &rid_b] {
        let run = fixtures::run(
            rid.as_str(),
            "tenant1",
            def_name,
            RunStatus::Queued,
            clock.value(),
        );
        store.put_run(&run).unwrap();
        enqueue(store.as_ref(), rid, clock.value());
    }

    let pool = WorkerPool::spawn(2, registry, store.clone(), clock.clone(), 7);
    let dispatcher = Dispatcher::new(store.as_ref(), &*clock, pool, 8, 60_000);

    let stats = dispatcher.step();
    assert_eq!(stats.scanned, 2);
    assert_eq!(stats.claimed, 2);

    for rid in [&rid_a, &rid_b] {
        poll(
            || run_status(store.as_ref(), rid) == RunStatus::Succeeded,
            "run to succeed",
        );
    }
    assert_eq!(store.len_queue(), 0);

    dispatcher.into_pool().shutdown();
}

#[test]
fn flaky_run_cycles_through_retry_backoff_then_gives_up() {
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(ManualClock::at(3_000_000));
    let registry = Arc::new(Registry::new());
    registry.register(
        HandlerId::from_validated("testalways0000".to_owned()),
        Flaky::always_fail(),
    );

    let def_name = "flaky0001";
    store
        .put_workflow(def(
            "wf_flaky0001",
            def_name,
            vec![task("work", "testalways0000", &[])],
            RetryPolicy::fixed(3, 25),
        ))
        .unwrap();

    let rid = RunId::from_validated("rn_flaky000000000".to_owned());
    store
        .put_run(&fixtures::run(
            "rn_flaky000000000",
            "tenant1",
            def_name,
            RunStatus::Queued,
            clock.value(),
        ))
        .unwrap();
    enqueue(store.as_ref(), &rid, clock.value());

    let pool = WorkerPool::spawn(2, registry, store.clone(), clock.clone(), 11);
    let dispatcher = Dispatcher::new(store.as_ref(), &*clock, pool, 8, 60_000);

    // Attempt #1: run 0 -> Queued (retry scheduled at +25ms).
    assert_eq!(dispatcher.step().claimed, 1);
    wait_settled(store.as_ref(), &rid);
    let run = store.get_run(&rid).unwrap();
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.attempts, 1);
    assert_eq!(run.next_attempt_at_ms, Some(3_000_025));

    // Attempt #2: the entry was released, so advance the clock and re-dispatch.
    clock.set(3_000_025);
    wait_ready(store.as_ref(), &clock, &rid);
    assert_eq!(dispatcher.step().claimed, 1);
    wait_settled(store.as_ref(), &rid);
    let run = store.get_run(&rid).unwrap();
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.attempts, 2);
    assert_eq!(run.next_attempt_at_ms, Some(3_000_050));

    // Attempt #3: budgets exhausted -> terminal Failed.
    clock.set(3_000_050);
    wait_ready(store.as_ref(), &clock, &rid);
    assert_eq!(dispatcher.step().claimed, 1);
    poll(
        || run_status(store.as_ref(), &rid) == RunStatus::Failed,
        "run to fail",
    );

    let run = store.get_run(&rid).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.attempts, 2);
    let err = run.error.expect("terminal failure carries a run error");
    assert_eq!(err.message, "intentional flake");
    let tasks = store.list_task_runs_for_run(&rid).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Failed);
    assert_eq!(tasks[0].attempts, 3);
    assert_eq!(store.len_queue(), 0);

    dispatcher.into_pool().shutdown();
}

#[test]
fn flaky_run_retries_then_succeeds_within_budget() {
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(ManualClock::at(4_000_000));
    let registry = Arc::new(Registry::new());
    registry.register(
        HandlerId::from_validated("testflaky000000".to_owned()),
        Flaky::succeeds_after(2),
    );

    let def_name = "recover0001";
    store
        .put_workflow(def(
            "wf_recover0001",
            def_name,
            vec![task("work", "testflaky000000", &[])],
            RetryPolicy::fixed(3, 25),
        ))
        .unwrap();

    let rid = RunId::from_validated("rn_recover00000000".to_owned());
    store
        .put_run(&fixtures::run(
            "rn_recover00000000",
            "tenant1",
            def_name,
            RunStatus::Queued,
            clock.value(),
        ))
        .unwrap();
    enqueue(store.as_ref(), &rid, clock.value());

    let pool = WorkerPool::spawn(2, registry, store.clone(), clock.clone(), 3);
    let dispatcher = Dispatcher::new(store.as_ref(), &*clock, pool, 8, 60_000);

    // Two failing attempts, each re-queued at +25ms.
    for round in 0..2 {
        assert_eq!(dispatcher.step().claimed, 1);
        wait_settled(store.as_ref(), &rid);
        let run = store.get_run(&rid).unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        assert_eq!(run.attempts, round as u32 + 1);
        let next = run.next_attempt_at_ms.unwrap();
        clock.set(next);
        wait_ready(store.as_ref(), &clock, &rid);
    }

    // Third attempt succeeds.
    assert_eq!(dispatcher.step().claimed, 1);
    poll(
        || run_status(store.as_ref(), &rid) == RunStatus::Succeeded,
        "run to recover",
    );
    let tasks = store.list_task_runs_for_run(&rid).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Succeeded);
    assert_eq!(tasks[0].attempts, 3);
    assert_eq!(store.len_queue(), 0);

    dispatcher.into_pool().shutdown();
}

#[test]
fn run_past_deadline_times_out_without_executing_tasks() {
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(ManualClock::at(6_000_000));
    let registry = Arc::new(Registry::new());
    registry.register(
        HandlerId::from_validated("testalways0000".to_owned()),
        Flaky::always_fail(),
    );

    let def_name = "deadline0001";
    store
        .put_workflow(def(
            "wf_deadline0001",
            def_name,
            vec![task("work", "testalways0000", &[])],
            RetryPolicy::fixed(3, 25),
        ))
        .unwrap();

    let rid = RunId::from_validated("rn_deadline0000000".to_owned());
    let mut run = fixtures::run(
        "rn_deadline0000000",
        "tenant1",
        def_name,
        RunStatus::Queued,
        clock.value(),
    );
    // Deadline lands after attempt #2's retry slot but before attempt #3.
    run.deadline_at_ms = Some(clock.value() + 60);
    store.put_run(&run).unwrap();
    enqueue(store.as_ref(), &rid, clock.value());

    let pool = WorkerPool::spawn(2, registry, store.clone(), clock.clone(), 9);
    let dispatcher = Dispatcher::new(store.as_ref(), &*clock, pool, 8, 60_000);

    // Attempt #1 fails, retry scheduled at +25ms.
    assert_eq!(dispatcher.step().claimed, 1);
    wait_settled(store.as_ref(), &rid);
    let run = store.get_run(&rid).unwrap();
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.next_attempt_at_ms, Some(6_000_025));

    // Attempt #2 fails, retry scheduled at +50ms (still inside the deadline).
    clock.set(6_000_025);
    wait_ready(store.as_ref(), &clock, &rid);
    assert_eq!(dispatcher.step().claimed, 1);
    wait_settled(store.as_ref(), &rid);
    let run = store.get_run(&rid).unwrap();
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.next_attempt_at_ms, Some(6_000_050));

    // Attempt #3 hits the past-deadline guard: time out, no task runs.
    clock.set(6_000_070);
    wait_ready(store.as_ref(), &clock, &rid);
    assert_eq!(dispatcher.step().claimed, 1);
    poll(
        || run_status(store.as_ref(), &rid) == RunStatus::TimedOut,
        "run to time out",
    );

    let run = store.get_run(&rid).unwrap();
    assert_eq!(run.status, RunStatus::TimedOut);
    let err = run.error.expect("timed out run carries an error");
    assert_eq!(err.kind, crate::domain::status::FailureKind::Timeout);
    assert!(err.message.contains("deadline"));
    // The last dispatched attempt never executed its task.
    let tasks = store.list_task_runs_for_run(&rid).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].attempts, 2);
    assert_eq!(tasks[0].status, TaskStatus::Failed);
    assert_eq!(store.len_queue(), 0);

    dispatcher.into_pool().shutdown();
}

#[test]
fn failed_dependency_skips_descendants_and_run_fails() {
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(ManualClock::at(5_000_000));
    let registry = Arc::new(Registry::new());
    registry.register(
        HandlerId::from_validated("testalways0000".to_owned()),
        Flaky::always_fail(),
    );
    registry.register(
        HandlerId::from_validated("testgood0000".to_owned()),
        Flaky::succeeds_after(0),
    );

    let def_name = "chain0001";
    store
        .put_workflow(def(
            "wf_chain0001",
            def_name,
            vec![
                task("first", "testalways0000", &[]),
                task("second", "testgood0000", &["first"]),
                task("third", "testgood0000", &["second"]),
            ],
            RetryPolicy::fixed(1, 25),
        ))
        .unwrap();

    let rid = RunId::from_validated("rn_chain0000000000".to_owned());
    store
        .put_run(&fixtures::run(
            "rn_chain0000000000",
            "tenant1",
            def_name,
            RunStatus::Queued,
            clock.value(),
        ))
        .unwrap();
    enqueue(store.as_ref(), &rid, clock.value());

    let pool = WorkerPool::spawn(2, registry, store.clone(), clock.clone(), 5);
    let dispatcher = Dispatcher::new(store.as_ref(), &*clock, pool, 8, 60_000);

    assert_eq!(dispatcher.step().claimed, 1);
    poll(
        || run_status(store.as_ref(), &rid) == RunStatus::Failed,
        "run to fail",
    );

    let tasks = store.list_task_runs_for_run(&rid).unwrap();
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].status, TaskStatus::Failed);
    assert_eq!(tasks[1].status, TaskStatus::Skipped);
    assert_eq!(tasks[2].status, TaskStatus::Skipped);
    assert_eq!(store.len_queue(), 0);

    dispatcher.into_pool().shutdown();
}