//! The store abstraction: durable state + the run queue.
//!
//! Every backend (in-memory, SQLite, later FUSE-adjacent or distributed)
//! implements [`Store`]. The trait is deliberately synchronous: the network
//! layer owns concurrency, and a store is expected to be cheaply internally
//! synchronous. Optimistic concurrency is expressed through the queue's
//! claim tokens and workflow version guards, which is what the defect
//! catalog later attacks.

pub mod filter;
#[cfg(test)]
pub mod fixtures;
pub mod memory;
#[cfg(feature = "sqlite")]
pub mod migrations;
pub mod model;
#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

use crate::domain::ids::{RunId, TaskRunId};
use crate::domain::run::{Run, TaskRun};
use crate::domain::workflow::WorkflowDef;
use crate::error::StorageError;

pub use filter::RunFilter;
pub use model::{ClaimToken, QueueEntry, WorkflowRecord, WorkflowSummary};

/// Backends must be cheaply clonable handles (an `Arc` to shared state).
pub trait Store: Send + Sync {
    // ----- workflows ----------------------------------------------------

    /// Stores a definition payload, creating or replacing its natural key.
    fn put_workflow(&self, def: WorkflowDef) -> Result<WorkflowRecord, StorageError>;

    /// Stores a definition only when the stored version still equals
    /// `expected_version`, and the new version is exactly `expected + 1`.
    fn update_workflow_version(
        &self,
        tenant: &str,
        name: &str,
        def: WorkflowDef,
        expected_version: u32,
    ) -> Result<WorkflowRecord, StorageError>;

    /// Loads a definition by natural key.
    fn get_workflow(&self, tenant: &str, name: &str) -> Result<WorkflowRecord, StorageError>;

    /// Lists all definitions.
    fn list_workflows(&self) -> Result<Vec<WorkflowRecord>, StorageError>;

    /// Workflow summaries with run counts for the overview dashboard.
    fn list_workflow_summaries(&self) -> Result<Vec<WorkflowSummary>, StorageError>;

    // ----- runs ---------------------------------------------------------

    /// Inserts or replaces a run by id.
    fn put_run(&self, run: &Run) -> Result<(), StorageError>;

    /// Loads a run by id.
    fn get_run(&self, id: &RunId) -> Result<Run, StorageError>;

    /// Removes a run and its task runs.
    fn delete_run(&self, id: &RunId) -> Result<(), StorageError>;

    /// Runs matching `filter`, newest first, respecting the filter's limit.
    fn list_runs(&self, filter: &RunFilter) -> Result<Vec<Run>, StorageError>;

    /// Count of runs matching `filter`.
    fn count_runs(&self, filter: &RunFilter) -> Result<usize, StorageError>;

    // ----- task runs ----------------------------------------------------

    /// Inserts or replaces a task run by id.
    fn put_task_run(&self, tr: &TaskRun) -> Result<(), StorageError>;

    /// Loads a task run by id.
    fn get_task_run(&self, id: &TaskRunId) -> Result<TaskRun, StorageError>;

    /// Task runs of a run, ordered deterministically.
    fn list_task_runs_for_run(&self, run_id: &RunId) -> Result<Vec<TaskRun>, StorageError>;

    // ----- submission counters ------------------------------------------

    /// Atomically increments the submission counter of a definition.
    fn next_run_number(&self, def_name: &str) -> Result<u64, StorageError>;

    // ----- queue ---------------------------------------------------------

    /// Inserts a queue entry. Conflicts if the run is already queued.
    fn enqueue(&self, entry: QueueEntry) -> Result<(), StorageError>;

    /// Up to `limit` entries that are due and unclaimed, earliest first.
    fn scan_ready(&self, now_ms: i64, limit: usize) -> Result<Vec<QueueEntry>, StorageError>;

    /// Leases the entry to `token` until `now_ms + lease_ms`. Fails with
    /// `ClaimLost` when the entry is leased by another token.
    fn claim(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<(), StorageError>;

    /// Removes the entry; the presented token must match the lease holder.
    fn ack(&self, run_id: &RunId, token: &ClaimToken) -> Result<(), StorageError>;

    /// Clears the lease and re-schedules the entry at `retry_at_ms`.
    fn release(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        retry_at_ms: i64,
    ) -> Result<(), StorageError>;

    /// Clears a lease owners are sure they hold, leaving the entry visible to
    /// `scan_ready` again (used by maintenance and defect telemetry).
    fn failclaim(&self, run_id: &RunId, token: &ClaimToken) -> Result<(), StorageError>;

    /// Un-leases every entry whose lease has expired; returns how many.
    fn recover_expired_leases(&self, now_ms: i64) -> Result<usize, StorageError>;

    /// Number of live queue entries (for gauges).
    fn len_queue(&self) -> usize;
}

#[cfg(test)]
mod store_tests;

#[cfg(test)]
mod tests {
    use super::memory::MemoryStore;
    use super::store_tests::run_store_suite;
    use super::*;
    use crate::domain::status::RunStatus;
    use crate::persistence::fixtures;
    use std::sync::Arc;
    use std::thread;

    fn new_memory() -> MemoryStore {
        MemoryStore::new()
    }

    #[test]
    fn memory_store_full_suite() {
        run_store_suite(&new_memory());
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_store_full_suite() {
        let sqlite = SqliteStore::open_in_memory().unwrap();
        assert_eq!(sqlite.schema_version().unwrap(), migrations::latest_version());
        run_store_suite(&sqlite);
    }

    /// Concurrent claims of the same entry must not double-lease: exactly one
    /// worker wins and the others get `ClaimLost`.
    #[test]
    fn concurrent_claim_has_single_winner() {
        let store = Arc::new(new_memory());
        let n = 8;
        let run = fixtures::run("rn_race", "acme", "race", RunStatus::Queued, 0);
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

        let mut handles = Vec::new();
        for _ in 0..n {
            let store = Arc::clone(&store);
            let run_id = run.id.clone();
            handles.push(thread::spawn(move || {
                let token = ClaimToken::new();
                match store.claim(&run_id, &token, 100, 10_000) {
                    Ok(()) => {
                        // Winner: hold briefly, then ack.
                        store.ack(&run_id, &token).unwrap();
                        true
                    }
                    Err(StorageError::ClaimLost(_)) => false,
                    Err(StorageError::NotFound(_)) => false, // another worker already acked
                    Err(other) => panic!("unexpected claim error: {other:?}"),
                }
            }));
        }
        let winners: Vec<bool> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(winners.iter().filter(|w| **w).count(), 1);
    }

    /// Awarding `Run` ids must stay unique under concurrent submissions.
    #[test]
    fn run_numbers_are_unique_under_concurrency() {
        let store = Arc::new(new_memory());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                let mut out = Vec::new();
                for _ in 0..200 {
                    out.push(store.next_run_number("burst").unwrap());
                }
                out
            }));
        }
        let mut all: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort_unstable();
        let unique = all.windows(2).all(|w| w[0] != w[1]);
        assert!(unique, "run numbers collided");
        assert_eq!(all.len(), 8 * 200);
    }

    /// Deleting a run must cascade to its task runs.
    #[test]
    fn delete_run_cascades_to_tasks() {
        let store = new_memory();
        let run = fixtures::run("rn_del", "acme", "nightly", RunStatus::Running, 1);
        store.put_run(&run).unwrap();
        store
            .put_task_run(&fixtures::task(
                &run.id,
                "a",
                crate::domain::status::TaskStatus::Running,
            ))
            .unwrap();
        store.delete_run(&run.id).unwrap();
        assert!(store.list_task_runs_for_run(&run.id).unwrap().is_empty());
        assert!(matches!(store.get_run(&run.id), Err(StorageError::NotFound(_))));
    }

    // Store backends must be thread-safe handles.
    #[allow(dead_code)]
    fn _typeassert_store_is_send_sync<S: Send + Sync>() {}
    #[allow(dead_code)]
    fn _pin_backends() {
        _typeassert_store_is_send_sync::<MemoryStore>();
        #[cfg(feature = "sqlite")]
        _typeassert_store_is_send_sync::<SqliteStore>();
    }
}