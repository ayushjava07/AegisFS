//! Multi-backend integration tests covering concurrency, filtering, and cascading lifecycle.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;

use runvane::domain::ids::{RunId, TaskRunId};
use runvane::domain::run::{Run, TaskRun};
use runvane::domain::status::{RunStatus, TaskStatus};
use runvane::persistence::memory::MemoryStore;
use runvane::persistence::{ClaimToken, QueueEntry, RunFilter, SqliteStore, Store};

fn new_run(id: &str, tenant: &str, name: &str, status: RunStatus, submitted_at_ms: i64) -> Run {
    Run {
        id: RunId::from_validated(id.to_string()),
        tenant: tenant.to_string(),
        def_name: name.to_string(),
        def_version: 1,
        input: serde_json::json!({}),
        status,
        attempts: 1,
        next_attempt_at_ms: None,
        deadline_at_ms: None,
        started_at_ms: None,
        finished_at_ms: None,
        error: None,
        output: None,
        tags: BTreeMap::new(),
        created_at_ms: submitted_at_ms,
        run_number: 1,
    }
}

fn new_task_run(id: &str, run_id: &RunId, name: &str, status: TaskStatus) -> TaskRun {
    TaskRun {
        id: TaskRunId::from_validated(id.to_string()),
        run_id: run_id.clone(),
        task_name: name.to_string(),
        status,
        attempts: 1,
        last_error: None,
        started_at_ms: Some(100),
        finished_at_ms: Some(150),
        output: None,
    }
}

fn create_backends() -> Vec<Box<dyn Store>> {
    let mem = Box::new(MemoryStore::new());
    let sqlite = Box::new(SqliteStore::open_in_memory().unwrap());
    vec![mem, sqlite]
}

#[test]
fn concurrent_claims_have_exactly_one_winner() {
    for store in create_backends() {
        let store = Arc::new(store);
        let rid = RunId::from_validated("rn_race000000001".to_string());
        store
            .put_run(&new_run(
                "rn_race000000001",
                "tenant1",
                "wf-race",
                RunStatus::Queued,
                1_000,
            ))
            .unwrap();

        store
            .enqueue(QueueEntry {
                run_id: rid.clone(),
                token: ClaimToken::empty(),
                due_at_ms: 1_000,
                lease_until_ms: None,
                claimed_by: None,
            })
            .unwrap();

        let num_threads = 8;
        let mut handles = Vec::new();
        for _ in 0..num_threads {
            let s = Arc::clone(&store);
            let r = rid.clone();
            handles.push(thread::spawn(move || {
                let token = ClaimToken::new();
                s.claim(&r, &token, 1_000, 5_000).is_ok()
            }));
        }

        let mut winners = 0;
        for h in handles {
            if h.join().unwrap() {
                winners += 1;
            }
        }

        assert_eq!(winners, 1, "exactly one thread must win the claim race");
    }
}

#[test]
fn complex_filter_across_population() {
    for store in create_backends() {
        // Populate runs across multiple tenants and statuses
        for i in 0..50 {
            let status = match i % 4 {
                0 => RunStatus::Queued,
                1 => RunStatus::Running,
                2 => RunStatus::Succeeded,
                _ => RunStatus::Failed,
            };
            let tenant = if i % 2 == 0 { "alpha" } else { "beta" };
            let run_id = format!("rn_{i:026x}");
            let mut r = new_run(&run_id, tenant, "etl-pipeline", status, 1_000 + i * 10);
            r.tags.insert("env".to_string(), "prod".to_string());
            if i % 3 == 0 {
                r.tags.insert("priority".to_string(), "high".to_string());
            }
            store.put_run(&r).unwrap();
        }

        // Filter 1: tenant = alpha, status_in = [Succeeded, Failed]
        let f1 = RunFilter {
            tenant: Some("alpha".to_string()),
            status_in: vec![RunStatus::Succeeded, RunStatus::Failed],
            ..RunFilter::default()
        };
        let res1 = store.list_runs(&f1).unwrap();
        assert!(res1.iter().all(|r| r.tenant == "alpha"));
        assert!(res1
            .iter()
            .all(|r| r.status == RunStatus::Succeeded || r.status == RunStatus::Failed));

        // Filter 2: has_tag_keys = ["priority"]
        let f2 = RunFilter {
            has_tag_keys: vec!["priority".to_string()],
            ..RunFilter::default()
        };
        let res2 = store.list_runs(&f2).unwrap();
        assert!(res2.iter().all(|r| r.tags.contains_key("priority")));

        // Filter 3: limit and offset pagination
        let f3_page1 = RunFilter {
            limit: Some(10),
            offset: 0,
            ..RunFilter::default()
        };
        let f3_page2 = RunFilter {
            limit: Some(10),
            offset: 10,
            ..RunFilter::default()
        };
        let page1 = store.list_runs(&f3_page1).unwrap();
        let page2 = store.list_runs(&f3_page2).unwrap();
        assert_eq!(page1.len(), 10);
        assert_eq!(page2.len(), 10);
        // Ensure no overlap
        assert!(page1
            .iter()
            .all(|p1| !page2.iter().any(|p2| p1.id == p2.id)));
    }
}

#[test]
fn cascade_delete_and_retention() {
    for store in create_backends() {
        let rid = RunId::from_validated("rn_cascade0001".to_string());
        let mut r = new_run(
            "rn_cascade0001",
            "tenant1",
            "cleanup-wf",
            RunStatus::Succeeded,
            100,
        );
        r.finished_at_ms = Some(200);
        store.put_run(&r).unwrap();

        // Add task runs
        let tr_id = "tr_casc0001";
        store
            .put_task_run(&new_task_run(tr_id, &rid, "step1", TaskStatus::Succeeded))
            .unwrap();

        assert_eq!(store.list_task_runs_for_run(&rid).unwrap().len(), 1);

        // Retention reaping before 300 should delete this run and its tasks
        let reaped = store.reap_finished_runs(300).unwrap();
        assert_eq!(reaped, 1);
        assert!(store.get_run(&rid).is_err());
        assert_eq!(store.list_task_runs_for_run(&rid).unwrap().len(), 0);
    }
}
