//! In-process, thread-safe store backed by hash maps.
//!
//! This is the default store for tests, single-node deployments, and the
//! dashboard. Every operation is a short critical section on one of a small
//! set of `parking_lot::Mutex` guards; there is deliberately no global lock,
//! so run records and queue entries mutate under different guards — the queue
//! ops carry their own ownership tokens, making cross-guard races detectable
//! rather than silent.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::ids::{RunId, TaskRunId};
use crate::domain::run::{Run, TaskRun};
use crate::domain::workflow::WorkflowDef;
use crate::error::StorageError as StoreError;

use super::filter::RunFilter;
use super::model::{ClaimToken, QueueEntry, WorkflowRecord, WorkflowSummary};
use super::Store;

/// The in-memory backend.
#[derive(Clone)]
pub struct MemoryStore {
    inner: Arc<Mutex<MemoryState>>,
}

#[derive(Default)]
struct MemoryState {
    runs: HashMap<RunId, Run>,
    task_runs: HashMap<TaskRunId, TaskRun>,
    defs: HashMap<(String, String), WorkflowRecord>, // (tenant, name)
    queue: HashMap<RunId, QueueEntry>,
    run_numbers: HashMap<String, u64>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MemoryState::default())),
        }
    }
}

impl MemoryStore {
    /// Creates a new store.
    pub fn new() -> Self {
        Self::default()
    }
}

fn task_runs_sorted(runs: &[&TaskRun]) -> Vec<TaskRun> {
    // Order by name to keep the dashboard deterministic across backends.
    let mut owned: Vec<TaskRun> = runs.iter().map(|t| (*t).clone()).collect();
    owned.sort_by(|a, b| a.task_name.cmp(&b.task_name));
    owned
}

impl Store for MemoryStore {
    fn put_workflow(&self, def: WorkflowDef) -> Result<WorkflowRecord, StoreError> {
        let rec = WorkflowRecord { def };
        let key = (rec.def.tenant.clone(), rec.def.name.clone());
        self.inner.lock().defs.insert(key, rec.clone());
        Ok(rec)
    }

    fn update_workflow_version(
        &self,
        tenant: &str,
        name: &str,
        def: WorkflowDef,
        expected_version: u32,
    ) -> Result<WorkflowRecord, StoreError> {
        let mut guard = self.inner.lock();
        let key = (tenant.to_owned(), name.to_owned());
        let current = guard
            .defs
            .get(&key)
            .ok_or(StoreError::NotFound(format!("workflow {tenant}/{name}")))?;
        if current.def.version != expected_version {
            return Err(StoreError::ConcurrentModification(format!(
                "workflow {tenant}/{name}"
            )));
        }
        if def.version != expected_version + 1 {
            return Err(StoreError::Conflict(format!(
                "workflow {tenant}/{name} version must be {}+1, got {}",
                expected_version, def.version
            )));
        }
        let rec = WorkflowRecord { def };
        guard.defs.insert(key, rec.clone());
        Ok(rec)
    }

    fn get_workflow(&self, tenant: &str, name: &str) -> Result<WorkflowRecord, StoreError> {
        self.inner
            .lock()
            .defs
            .get(&(tenant.to_owned(), name.to_owned()))
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("workflow {tenant}/{name}")))
    }

    fn list_workflows(&self) -> Result<Vec<WorkflowRecord>, StoreError> {
        let mut recs: Vec<WorkflowRecord> = self.inner.lock().defs.values().cloned().collect();
        recs.sort_by(|a, b| (&a.def.tenant, &a.def.name).cmp(&(&b.def.tenant, &b.def.name)));
        Ok(recs)
    }

    fn list_workflow_summaries(&self) -> Result<Vec<WorkflowSummary>, StoreError> {
        let guard = self.inner.lock();
        let mut out = Vec::new();
        for rec in guard.defs.values() {
            let mut runs: Vec<Run> = guard
                .runs
                .values()
                .filter(|r| r.def_name == rec.def.name && r.tenant == rec.def.tenant)
                .cloned()
                .collect();
            runs.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
            out.push(WorkflowSummary::assemble(rec.clone(), &runs));
        }
        Ok(out)
    }

    fn put_run(&self, run: &Run) -> Result<(), StoreError> {
        self.inner.lock().runs.insert(run.id.clone(), run.clone());
        Ok(())
    }

    fn get_run(&self, id: &RunId) -> Result<Run, StoreError> {
        self.inner
            .lock()
            .runs
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("run {id}")))
    }

    fn delete_run(&self, id: &RunId) -> Result<(), StoreError> {
        let mut guard = self.inner.lock();
        guard.runs.remove(id);
        guard.task_runs.retain(|_, tr| tr.run_id != *id);
        Ok(())
    }

    fn list_runs(&self, filter: &RunFilter) -> Result<Vec<Run>, StoreError> {
        let guard = self.inner.lock();
        let matched: Vec<&Run> = guard.runs.values().filter(|r| filter.matches(r)).collect();
        Ok(filter.apply_order(matched))
    }

    fn count_runs(&self, filter: &RunFilter) -> Result<usize, StoreError> {
        let guard = self.inner.lock();
        Ok(guard.runs.values().filter(|r| filter.matches(r)).count())
    }

    fn put_task_run(&self, tr: &TaskRun) -> Result<(), StoreError> {
        self.inner
            .lock()
            .task_runs
            .insert(tr.id.clone(), tr.clone());
        Ok(())
    }

    fn get_task_run(&self, id: &TaskRunId) -> Result<TaskRun, StoreError> {
        self.inner
            .lock()
            .task_runs
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("task run {id}")))
    }

    fn list_task_runs_for_run(&self, run_id: &RunId) -> Result<Vec<TaskRun>, StoreError> {
        let guard = self.inner.lock();
        let matched: Vec<&TaskRun> = guard
            .task_runs
            .values()
            .filter(|t| t.run_id == *run_id)
            .collect();
        Ok(task_runs_sorted(&matched))
    }

    fn next_run_number(&self, def_name: &str) -> Result<u64, StoreError> {
        let mut guard = self.inner.lock();
        let next = guard.run_numbers.entry(def_name.to_owned()).or_insert(0);
        *next += 1;
        Ok(*next)
    }

    fn enqueue(&self, entry: QueueEntry) -> Result<(), StoreError> {
        let mut guard = self.inner.lock();
        if guard.queue.contains_key(&entry.run_id) {
            return Err(StoreError::Conflict(format!(
                "already queued: {}",
                entry.run_id
            )));
        }
        guard.queue.insert(entry.run_id.clone(), entry);
        Ok(())
    }

    fn scan_ready(&self, now_ms: i64, limit: usize) -> Result<Vec<QueueEntry>, StoreError> {
        let guard = self.inner.lock();
        let mut ready: Vec<QueueEntry> = guard
            .queue
            .values()
            .filter(|e| e.is_ready(now_ms))
            .cloned()
            .collect();
        ready.sort_by_key(|e| e.due_at_ms);
        ready.truncate(limit);
        Ok(ready)
    }

    fn claim(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<(), StoreError> {
        let mut guard = self.inner.lock();
        let entry = guard
            .queue
            .get(run_id)
            .ok_or_else(|| StoreError::NotFound(format!("queue entry {run_id}")))?;
        if entry.is_leased(now_ms) && entry.token != *token {
            return Err(StoreError::ClaimLost(format!("run {run_id}")));
        }
        if entry.token.0.is_empty() && token.0.is_empty() {
            // Free claim: allowed only when the entry is currently unleased.
            if entry.is_leased(now_ms) {
                return Err(StoreError::ClaimLost(format!("run {run_id}")));
            }
        }
        let mut new_entry = entry.clone();
        new_entry.token = token.clone();
        new_entry.claimed_by = Some("dispatcher".to_owned());
        new_entry.lease_until_ms = Some(now_ms + lease_ms);
        guard.queue.insert(run_id.clone(), new_entry);
        Ok(())
    }

    fn ack(&self, run_id: &RunId, token: &ClaimToken) -> Result<(), StoreError> {
        let mut guard = self.inner.lock();
        let entry = guard
            .queue
            .get(run_id)
            .ok_or_else(|| StoreError::NotFound(format!("queue entry {run_id}")))?;
        if entry.token != *token {
            return Err(StoreError::ClaimLost(format!("run {run_id}")));
        }
        guard.queue.remove(run_id);
        Ok(())
    }

    fn release(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        retry_at_ms: i64,
    ) -> Result<(), StoreError> {
        let mut guard = self.inner.lock();
        let entry = guard
            .queue
            .get(run_id)
            .ok_or_else(|| StoreError::NotFound(format!("queue entry {run_id}")))?;
        if entry.token != *token {
            return Err(StoreError::ClaimLost(format!("run {run_id}")));
        }
        let mut new_entry = entry.clone();
        new_entry.token = ClaimToken::empty();
        new_entry.claimed_by = None;
        new_entry.lease_until_ms = None;
        new_entry.due_at_ms = retry_at_ms;
        guard.queue.insert(run_id.clone(), new_entry);
        Ok(())
    }

    fn failclaim(&self, run_id: &RunId, token: &ClaimToken) -> Result<(), StoreError> {
        let mut guard = self.inner.lock();
        let entry = guard
            .queue
            .get(run_id)
            .ok_or_else(|| StoreError::NotFound(format!("queue entry {run_id}")))?;
        if entry.token != *token {
            return Err(StoreError::ClaimLost(format!("run {run_id}")));
        }
        let mut new_entry = entry.clone();
        new_entry.token = ClaimToken::empty();
        new_entry.claimed_by = None;
        new_entry.lease_until_ms = None;
        guard.queue.insert(run_id.clone(), new_entry);
        Ok(())
    }

    fn renew_lease(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        now_ms: i64,
        extend_by_ms: i64,
    ) -> Result<(), StoreError> {
        let mut guard = self.inner.lock();
        let entry = guard
            .queue
            .get(run_id)
            .ok_or_else(|| StoreError::NotFound(format!("queue entry {run_id}")))?;
        if entry.token != *token || !entry.is_leased(now_ms) {
            return Err(StoreError::ClaimLost(format!("run {run_id}")));
        }
        let mut new_entry = entry.clone();
        new_entry.lease_until_ms = Some(now_ms + extend_by_ms);
        guard.queue.insert(run_id.clone(), new_entry);
        Ok(())
    }

    fn cancel_run(&self, run_id: &RunId, now_ms: i64) -> Result<bool, StoreError> {
        let mut guard = self.inner.lock();
        let mut run = guard
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("run {run_id}")))?;
        if run.is_terminal() {
            return Ok(false);
        }
        run.status = crate::domain::status::RunStatus::Cancelled;
        run.finished_at_ms = Some(now_ms);
        run.error = None;
        guard.runs.insert(run_id.clone(), run);
        guard.queue.remove(run_id);
        Ok(true)
    }

    fn recover_expired_leases(&self, now_ms: i64) -> Result<usize, StoreError> {
        let mut guard = self.inner.lock();
        let mut recovered = 0;
        let expired: Vec<RunId> = guard
            .queue
            .values()
            .filter(|e| matches!(e.lease_until_ms, Some(exp) if exp <= now_ms))
            .map(|e| e.run_id.clone())
            .collect();
        for id in expired {
            if let Some(entry) = guard.queue.get_mut(&id) {
                entry.token = ClaimToken::empty();
                entry.claimed_by = None;
                entry.lease_until_ms = None;
                recovered += 1;
            }
        }
        Ok(recovered)
    }

    fn len_queue(&self) -> usize {
        self.inner.lock().queue.len()
    }
}
