//! Read-path caching decorator over `Store`.
//!
//! `runvane serve` otherwise pays a full map/table lookup for every workflow
//! fetch and submission. This decorator memoizes `get_workflow` by natural
//! key with an LRU eviction policy and invalidates the entry on any write
//! that could have changed it. All mutating and list calls delegate straight
//! to the inner store, so ordering semantics and error behavior are untouched.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use lru::LruCache;

use crate::domain::ids::{RunId, TaskRunId};
use crate::domain::run::{Run, TaskRun};
use crate::domain::workflow::WorkflowDef;
use crate::error::StorageError;
use crate::persistence::{model::WorkflowRecord, RunFilter, Store};

/// Default number of workflow definitions kept warm on the read path.
pub const DEFAULT_CACHE_CAPACITY: usize = 128;

/// Wraps any `Store` and caches workflow lookups by `(tenant, name)`.
///
/// Cloning is cheap (an `Arc` to shared state) and cache hits keep the
/// definition payload unaffected by later writes to other keys.
pub struct LruStore {
    inner: Arc<dyn Store>,
    cache: Mutex<LruCache<(String, String), WorkflowRecord>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl LruStore {
    /// Wraps `inner`, caching up to `capacity` workflow records.
    pub fn with_capacity(inner: Arc<dyn Store>, capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity).expect("cache capacity must be non-zero");
        Self {
            inner,
            cache: Mutex::new(LruCache::new(capacity)),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Wraps `inner` with the default capacity.
    pub fn wrap(inner: Arc<dyn Store>) -> Self {
        Self::with_capacity(inner, DEFAULT_CACHE_CAPACITY)
    }

    /// Returns the live hit and miss totals.
    pub fn cache_stats(&self) -> (u64, u64) {
        (
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
        )
    }

    fn fetch(&self, tenant: &str, name: &str) -> Result<WorkflowRecord, StorageError> {
        let key = (tenant.to_owned(), name.to_owned());
        if let Some(rec) = self.cache.lock().unwrap().get(&key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(rec.clone());
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        let rec = self.inner.get_workflow(tenant, name)?;
        self.cache.lock().unwrap().put(key, rec.clone());
        Ok(rec)
    }

    fn invalidate(&self, tenant: &str, name: &str) {
        let mut cache = self.cache.lock().unwrap();
        let key = &(tenant.to_owned(), name.to_owned());
        cache.pop(key);
    }
}

impl Store for LruStore {
    fn put_workflow(&self, def: WorkflowDef) -> Result<WorkflowRecord, StorageError> {
        let rec = self.inner.put_workflow(def)?;
        self.invalidate(&rec.def.tenant, &rec.def.name);
        Ok(rec)
    }

    fn update_workflow_version(
        &self,
        tenant: &str,
        name: &str,
        def: WorkflowDef,
        expected_version: u32,
    ) -> Result<WorkflowRecord, StorageError> {
        let rec = self
            .inner
            .update_workflow_version(tenant, name, def, expected_version)?;
        self.invalidate(tenant, name);
        Ok(rec)
    }

    fn get_workflow(&self, tenant: &str, name: &str) -> Result<WorkflowRecord, StorageError> {
        self.fetch(tenant, name)
    }

    fn list_workflows(&self) -> Result<Vec<WorkflowRecord>, StorageError> {
        self.inner.list_workflows()
    }

    fn list_workflow_summaries(
        &self,
    ) -> Result<Vec<crate::persistence::WorkflowSummary>, StorageError> {
        self.inner.list_workflow_summaries()
    }

    fn put_run(&self, run: &Run) -> Result<(), StorageError> {
        self.inner.put_run(run)
    }

    fn get_run(&self, id: &RunId) -> Result<Run, StorageError> {
        self.inner.get_run(id)
    }

    fn delete_run(&self, id: &RunId) -> Result<(), StorageError> {
        self.inner.delete_run(id)
    }

    fn list_runs(&self, filter: &RunFilter) -> Result<Vec<Run>, StorageError> {
        self.inner.list_runs(filter)
    }

    fn count_runs(&self, filter: &RunFilter) -> Result<usize, StorageError> {
        self.inner.count_runs(filter)
    }

    fn put_task_run(&self, tr: &TaskRun) -> Result<(), StorageError> {
        self.inner.put_task_run(tr)
    }

    fn get_task_run(&self, id: &TaskRunId) -> Result<TaskRun, StorageError> {
        self.inner.get_task_run(id)
    }

    fn list_task_runs_for_run(&self, run_id: &RunId) -> Result<Vec<TaskRun>, StorageError> {
        self.inner.list_task_runs_for_run(run_id)
    }

    fn next_run_number(&self, def_name: &str) -> Result<u64, StorageError> {
        self.inner.next_run_number(def_name)
    }

    fn enqueue(&self, entry: crate::persistence::QueueEntry) -> Result<(), StorageError> {
        self.inner.enqueue(entry)
    }

    fn scan_ready(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<crate::persistence::QueueEntry>, StorageError> {
        self.inner.scan_ready(now_ms, limit)
    }

    fn claim(
        &self,
        run_id: &RunId,
        token: &crate::persistence::ClaimToken,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<(), StorageError> {
        self.inner.claim(run_id, token, now_ms, lease_ms)
    }

    fn ack(
        &self,
        run_id: &RunId,
        token: &crate::persistence::ClaimToken,
    ) -> Result<(), StorageError> {
        self.inner.ack(run_id, token)
    }

    fn release(
        &self,
        run_id: &RunId,
        token: &crate::persistence::ClaimToken,
        retry_at_ms: i64,
    ) -> Result<(), StorageError> {
        self.inner.release(run_id, token, retry_at_ms)
    }

    fn failclaim(
        &self,
        run_id: &RunId,
        token: &crate::persistence::ClaimToken,
    ) -> Result<(), StorageError> {
        self.inner.failclaim(run_id, token)
    }

    fn cancel_run(&self, run_id: &RunId, now_ms: i64) -> Result<bool, StorageError> {
        self.inner.cancel_run(run_id, now_ms)
    }

    fn recover_expired_leases(&self, now_ms: i64) -> Result<usize, StorageError> {
        self.inner.recover_expired_leases(now_ms)
    }

    fn len_queue(&self) -> usize {
        self.inner.len_queue()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{TaskSpec, WorkflowDef};
    use crate::persistence::memory::MemoryStore;
    use std::collections::BTreeMap;

    fn def(name: &str) -> WorkflowDef {
        WorkflowDef {
            id: crate::domain::ids::WorkflowId::from_validated("wf_x".to_owned()),
            tenant: "acme".to_owned(),
            name: name.to_owned(),
            version: 1,
            description: String::new(),
            tasks: vec![TaskSpec {
                name: "a".to_owned(),
                handler: crate::domain::ids::HandlerId::from_validated("runvane.echo".to_owned()),
                depends_on: Vec::new(),
                input: serde_json::Value::Null,
                timeout_ms: None,
                retry: None,
                meta: BTreeMap::new(),
            }],
            timeout_ms: 60_000,
            retry: crate::domain::retry_policy::RetryPolicy::fixed(1, 1_000),
            default_priority: crate::domain::status::Priority::Normal,
            hooks: crate::domain::workflow::Hooks::default(),
            tags: BTreeMap::new(),
            spec_version: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn cached() -> LruStore {
        LruStore::wrap(Arc::new(MemoryStore::new()))
    }

    #[test]
    fn hits_serve_from_cache_and_evict_on_write() {
        let store = cached();
        store.put_workflow(def("ship")).unwrap();

        let first = store.get_workflow("acme", "ship").unwrap();
        let second = store.get_workflow("acme", "ship").unwrap();
        assert_eq!(first.def.name, second.def.name);
        let (hits, misses) = store.cache_stats();
        assert_eq!((hits, misses), (1, 1));

        // A write invalidates the cached copy; the next read goes to the store.
        store.put_workflow(def("ship")).unwrap();
        let (hits, misses) = store.cache_stats();
        assert_eq!((hits, misses), (1, 1), "cache still warm before write");
        store.get_workflow("acme", "ship").unwrap();
        let (hits, misses) = store.cache_stats();
        assert_eq!((hits, misses), (1, 2), "write evicted, read missed");
    }

    #[test]
    fn capacity_evicts_least_recently_used_key() {
        let store = LruStore::with_capacity(Arc::new(MemoryStore::new()), 2);
        for name in ["a", "b", "c"] {
            store.put_workflow(def(name)).unwrap();
        }
        // Writes always invalidate, so the cache warms only through reads.
        store.get_workflow("acme", "b").unwrap(); // miss 1 -> [b]
        store.get_workflow("acme", "c").unwrap(); // miss 2 -> [b, c]
        store.get_workflow("acme", "a").unwrap(); // miss 3 -> [c, a], b evicted
        store.get_workflow("acme", "c").unwrap(); // hit 1
        store.get_workflow("acme", "a").unwrap(); // hit 2
        store.get_workflow("acme", "b").unwrap(); // miss 4 -> b was evicted
        let (hits, misses) = store.cache_stats();
        assert_eq!((hits, misses), (2, 4), "lru evicted the oldest warm key");
    }

    #[test]
    fn put_run_and_list_do_not_pollute_the_cache() {
        let store = LruStore::wrap(Arc::new(MemoryStore::new()));
        store.put_workflow(def("ship")).unwrap();
        store.get_workflow("acme", "ship").unwrap(); // miss, cached now
        store.get_workflow("acme", "ship").unwrap(); // hit
        let run = crate::persistence::fixtures::run(
            "rn_1",
            "acme",
            "ship",
            crate::domain::status::RunStatus::Queued,
            1,
        );
        store.put_run(&run).unwrap();
        let _all = store.list_runs(&RunFilter::default()).unwrap();
        let (hits, misses) = store.cache_stats();
        assert_eq!(
            (hits, misses),
            (1, 1),
            "definition unaffected by unrelated ops"
        );
    }
}
