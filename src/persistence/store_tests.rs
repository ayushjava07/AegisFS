//! Behavior tests shared between store backends.
//!
//! Each test is written against the [`Store`] trait and run once per backend,
//! so a divergence between the memory and SQLite implementations surfaces as
//! a test failure instead of a silent behavioral drift.

use crate::domain::status::{RunStatus, TaskStatus};
use crate::error::StorageError;
use crate::persistence::fixtures;
use crate::persistence::{ClaimToken, QueueEntry, RunFilter, Store};

/// Runs the full behavior suite against `store`.
pub fn run_store_suite(store: &dyn Store) {
    workflow_roundtrip_and_version_guard(store);
    run_crud_and_filters(store);
    task_run_crud(store);
    run_number_increments(store);
    queue_lifecycle(store);
    queue_claimed_entry_hides_from_scan(store);
    cancel_run_semantics(store);
}

/// Cancellation transitions non-terminal runs, drops the queue entry, and is
/// idempotent-neutral for terminal runs (returns `false`, never rewrites).
fn cancel_run_semantics(store: &dyn Store) {
    // Queued run cancels and disappears from the queue.
    let queued = fixtures::run("rn_c1", "acme", "ship", RunStatus::Queued, 100);
    store.put_run(&queued).unwrap();
    store
        .enqueue(QueueEntry {
            run_id: queued.id.clone(),
            token: ClaimToken::empty(),
            due_at_ms: 100,
            lease_until_ms: None,
            claimed_by: None,
        })
        .unwrap();
    assert!(store.cancel_run(&queued.id, 250).unwrap());
    let after = store.get_run(&queued.id).unwrap();
    assert_eq!(after.status, RunStatus::Cancelled);
    assert_eq!(after.finished_at_ms, Some(250));
    // The queue entry is gone: re-enqueueing the same run no longer conflicts.
    assert!(store
        .enqueue(QueueEntry {
            run_id: queued.id.clone(),
            token: ClaimToken::empty(),
            due_at_ms: 500,
            lease_until_ms: None,
            claimed_by: None,
        })
        .is_ok());

    // Running run (lease holder present) cancels and its queue entry is gone.
    let running = fixtures::run("rn_c2", "acme", "ship", RunStatus::Running, 100);
    store.put_run(&running).unwrap();
    store
        .enqueue(QueueEntry {
            run_id: running.id.clone(),
            token: ClaimToken::new(),
            due_at_ms: 100,
            lease_until_ms: Some(1_000),
            claimed_by: Some("dispatcher".into()),
        })
        .unwrap();
    assert!(store.cancel_run(&running.id, 300).unwrap());
    assert_eq!(store.get_run(&running.id).unwrap().status, RunStatus::Cancelled);
    assert!(store
        .enqueue(QueueEntry {
            run_id: running.id.clone(),
            token: ClaimToken::empty(),
            due_at_ms: 600,
            lease_until_ms: None,
            claimed_by: None,
        })
        .is_ok());

    // Terminal runs are left untouched.
    let done = fixtures::run("rn_c3", "acme", "ship", RunStatus::Succeeded, 100);
    store.put_run(&done).unwrap();
    assert!(!store.cancel_run(&done.id, 400).unwrap());
    let still = store.get_run(&done.id).unwrap();
    assert_eq!(still.status, RunStatus::Succeeded);
    assert_eq!(still.finished_at_ms, None);

    // Unknown runs surface as NotFound.
    let missing = crate::domain::ids::RunId::from_validated("rn_zzz".into());
    assert!(matches!(
        store.cancel_run(&missing, 0),
        Err(StorageError::NotFound(_))
    ));
}

fn workflow_roundtrip_and_version_guard(store: &dyn Store) {
    let def = crate::domain::workflow::WorkflowDef {
        id: crate::domain::ids::WorkflowId::from_validated("wf_abc".into()),
        tenant: "acme".into(),
        name: "ship".into(),
        version: 1,
        description: String::new(),
        tasks: Vec::new(),
        timeout_ms: 60_000,
        retry: Default::default(),
        default_priority: Default::default(),
        hooks: Default::default(),
        tags: Default::default(),
        spec_version: crate::domain::workflow::SPEC_VERSION,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    store.put_workflow(def.clone()).unwrap();
    let loaded = store.get_workflow("acme", "ship").unwrap();
    assert_eq!(loaded.def.name, "ship");

    // Version guard: stale writer is rejected.
    let v2 = {
        let mut candidate = def.clone();
        candidate.version = 2;
        candidate
    };
    let ok = store
        .update_workflow_version("acme", "ship", v2, 1)
        .unwrap();
    assert_eq!(ok.def.version, 2);

    let stale = def.clone();
    let mut stale = stale;
    stale.version = 2;
    assert!(matches!(
        store.update_workflow_version("acme", "ship", stale, 1),
        Err(StorageError::ConcurrentModification(_))
    ));
    assert!(matches!(
        store.update_workflow_version("acme", "ship", def.clone(), 2),
        Err(StorageError::Conflict(_))
    ));

    let list = store.list_workflows().unwrap();
    assert_eq!(list.len(), 1);
}

fn run_crud_and_filters(store: &dyn Store) {
    let a = fixtures::run("rn_a", "acme", "nightly", RunStatus::Queued, 100);
    let b = fixtures::run("rn_b", "acme", "nightly", RunStatus::Running, 200);
    let c = fixtures::run("rn_c", "beta", "nightly", RunStatus::Failed, 300);
    store.put_run(&a).unwrap();
    store.put_run(&b).unwrap();
    store.put_run(&c).unwrap();

    assert_eq!(store.get_run(&a.id).unwrap(), a);

    let all = store
        .list_runs(&RunFilter::default())
        .unwrap();
    assert_eq!(all.len(), 3);
    // Newest first.
    assert_eq!(all[0].id, c.id);

    let failed = store
        .list_runs(&RunFilter {
            status: Some(RunStatus::Failed),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].id, c.id);

    let tenant = store
        .list_runs(&RunFilter {
            tenant: Some("acme".into()),
            limit: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tenant.len(), 1);
    assert_eq!(tenant[0].id, b.id);

    assert_eq!(
        store.count_runs(&RunFilter { name: Some("nightly".into()), ..Default::default() }).unwrap(),
        3
    );
}

fn task_run_crud(store: &dyn Store) {
    let run = fixtures::run("rn_t", "acme", "nightly", RunStatus::Running, 1);
    store.put_run(&run).unwrap();
    let t1 = fixtures::task(&run.id, "fetch", TaskStatus::Running);
    let t2 = fixtures::task(&run.id, "build", TaskStatus::Pending);
    store.put_task_run(&t1).unwrap();
    store.put_task_run(&t2).unwrap();

    assert_eq!(store.get_task_run(&t1.id).unwrap(), t1);
    let tasks = store.list_task_runs_for_run(&run.id).unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].task_name, "build"); // deterministic ordering
    assert_eq!(tasks[1].task_name, "fetch");
}

fn run_number_increments(store: &dyn Store) {
    assert_eq!(store.next_run_number("nightly").unwrap(), 1);
    assert_eq!(store.next_run_number("nightly").unwrap(), 2);
    assert_eq!(store.next_run_number("other").unwrap(), 1);
}

fn queue_lifecycle(store: &dyn Store) {
    let run = fixtures::run("rn_q", "acme", "nightly", RunStatus::Queued, 0);
    store.put_run(&run).unwrap();
    let entry = QueueEntry {
        run_id: run.id.clone(),
        token: ClaimToken::empty(),
        due_at_ms: 50,
        lease_until_ms: None,
        claimed_by: None,
    };
    store.enqueue(entry).unwrap();

    // Not ready yet.
    assert!(store.scan_ready(10, 10).unwrap().is_empty());
    // Ready once due.
    let ready = store.scan_ready(60, 10).unwrap();
    assert_eq!(ready.len(), 1);

    // Claim wins for the given token.
    let token = ClaimToken::new();
    store.claim(&run.id, &token, 60, 100).unwrap();
    // Leased: hidden from scans.
    assert!(store.scan_ready(70, 10).unwrap().is_empty());
    // Second claim with a different token fails.
    let other = ClaimToken::new();
    assert!(matches!(
        store.claim(&run.id, &other, 70, 100),
        Err(StorageError::ClaimLost(_))
    ));
    // Same token re-claim (renew) is allowed.
    store.claim(&run.id, &token, 80, 100).unwrap();

    // Release a failed attempt: re-schedules later.
    store.release(&run.id, &token, 200).unwrap();
    assert!(store.scan_ready(199, 10).unwrap().is_empty());
    let again = store.scan_ready(200, 10).unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].run_id, run.id);

    // Ack removes the entry.
    store.claim(&run.id, &token, 200, 100).unwrap();
    store.ack(&run.id, &token).unwrap();
    assert!(store.scan_ready(10_000, 10).unwrap().is_empty());
    assert_eq!(store.len_queue(), 0);
}

fn queue_claimed_entry_hides_from_scan(store: &dyn Store) {
    let run = fixtures::run("rn_h", "acme", "nightly", RunStatus::Queued, 0);
    store.put_run(&run).unwrap();
    store
        .enqueue(QueueEntry {
            run_id: run.id.clone(),
            token: ClaimToken::empty(),
            due_at_ms: 0,
            lease_until_ms: None,
            claimed_by: None,
        })
        .unwrap();
    let token = ClaimToken::new();
    store.claim(&run.id, &token, 0, 1000).unwrap();
    assert!(store.scan_ready(0, 10).unwrap().is_empty());
    // Expired lease: recover returns the entry to the pool.
    assert_eq!(store.recover_expired_leases(2000).unwrap(), 1);
    let reclaimed = store.scan_ready(2000, 10).unwrap();
    assert_eq!(reclaimed.len(), 1);
}