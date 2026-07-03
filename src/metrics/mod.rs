use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::core::types::MetricsSnapshot;

#[derive(Debug)]
pub struct MetricsState {
    chunks_stored: AtomicU64,
    chunks_deleted: AtomicU64,
    read_ops: AtomicU64,
    write_ops: AtomicU64,
    sync_ops: AtomicU64,
    errors: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    bytes_read: AtomicU64,
    bytes_written: AtomicU64,
}

impl MetricsState {
    pub fn new() -> Self {
        Self {
            chunks_stored: AtomicU64::new(0),
            chunks_deleted: AtomicU64::new(0),
            read_ops: AtomicU64::new(0),
            write_ops: AtomicU64::new(0),
            sync_ops: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            bytes_read: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
        }
    }
}

impl Default for MetricsState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct MetricsCollector {
    inner: Arc<Mutex<MetricsState>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MetricsState::new())),
        }
    }

    pub fn increment_chunks_stored(&self, n: u64) {
        let state = self.inner.lock().unwrap();
        state.chunks_stored.fetch_add(n, Ordering::Relaxed);
    }

    pub fn increment_chunks_deleted(&self, n: u64) {
        let state = self.inner.lock().unwrap();
        state.chunks_deleted.fetch_add(n, Ordering::Relaxed);
    }

    pub fn increment_read_ops(&self) {
        let state = self.inner.lock().unwrap();
        state.read_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_write_ops(&self) {
        let state = self.inner.lock().unwrap();
        state.write_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_sync_ops(&self) {
        let state = self.inner.lock().unwrap();
        state.sync_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_errors(&self) {
        let state = self.inner.lock().unwrap();
        state.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_hit(&self) {
        let state = self.inner.lock().unwrap();
        state.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_miss(&self) {
        let state = self.inner.lock().unwrap();
        state.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_bytes_read(&self, n: u64) {
        let state = self.inner.lock().unwrap();
        state.bytes_read.fetch_add(n, Ordering::Relaxed);
    }

    pub fn record_bytes_written(&self, n: u64) {
        let state = self.inner.lock().unwrap();
        state.bytes_written.fetch_add(n, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let state = self.inner.lock().unwrap();
        MetricsSnapshot {
            timestamp: Utc::now(),
            total_chunks: state.chunks_stored.load(Ordering::Relaxed),
            total_size: state.bytes_written.load(Ordering::Relaxed),
            dedup_size: 0,
            compressed_size: 0,
            chunks_stored: state.chunks_stored.load(Ordering::Relaxed),
            chunks_deleted: state.chunks_deleted.load(Ordering::Relaxed),
            read_ops: state.read_ops.load(Ordering::Relaxed),
            write_ops: state.write_ops.load(Ordering::Relaxed),
            sync_ops: state.sync_ops.load(Ordering::Relaxed),
            errors: state.errors.load(Ordering::Relaxed),
            cache_hits: state.cache_hits.load(Ordering::Relaxed),
            cache_misses: state.cache_misses.load(Ordering::Relaxed),
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_counter_increments() {
        let collector = MetricsCollector::new();

        collector.increment_chunks_stored(5);
        collector.increment_chunks_deleted(2);
        collector.increment_read_ops();
        collector.increment_write_ops();
        collector.increment_sync_ops();
        collector.increment_errors();
        collector.record_cache_hit();
        collector.record_cache_miss();
        collector.record_bytes_read(1000);
        collector.record_bytes_written(500);

        let snap = collector.snapshot();
        assert_eq!(snap.chunks_stored, 5);
        assert_eq!(snap.chunks_deleted, 2);
        assert_eq!(snap.read_ops, 1);
        assert_eq!(snap.write_ops, 1);
        assert_eq!(snap.sync_ops, 1);
        assert_eq!(snap.errors, 1);
        assert_eq!(snap.cache_hits, 1);
        assert_eq!(snap.cache_misses, 1);
        assert_eq!(snap.total_size, 500);
    }

    #[test]
    fn test_snapshot_values() {
        let collector = MetricsCollector::new();

        for _ in 0..10 {
            collector.increment_read_ops();
        }
        for _ in 0..5 {
            collector.increment_write_ops();
        }
        for _ in 0..3 {
            collector.record_cache_hit();
        }
        collector.record_cache_miss();

        let snap = collector.snapshot();
        assert_eq!(snap.read_ops, 10);
        assert_eq!(snap.write_ops, 5);
        assert_eq!(snap.cache_hits, 3);
        assert_eq!(snap.cache_misses, 1);
    }

    #[test]
    fn test_concurrent_updates() {
        let collector = Arc::new(MetricsCollector::new());
        let mut handles = Vec::new();

        for _ in 0..8 {
            let c = Arc::clone(&collector);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    c.increment_read_ops();
                    c.increment_chunks_stored(1);
                    c.record_cache_hit();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let snap = collector.snapshot();
        assert_eq!(snap.read_ops, 8000);
        assert_eq!(snap.chunks_stored, 8000);
        assert_eq!(snap.cache_hits, 8000);
    }

    #[test]
    fn test_metrics_default() {
        let collector = MetricsCollector::default();
        let snap = collector.snapshot();
        assert_eq!(snap.chunks_stored, 0);
        assert_eq!(snap.chunks_deleted, 0);
        assert_eq!(snap.read_ops, 0);
        assert_eq!(snap.write_ops, 0);
        assert_eq!(snap.sync_ops, 0);
        assert_eq!(snap.errors, 0);
        assert_eq!(snap.cache_hits, 0);
        assert_eq!(snap.cache_misses, 0);
        assert_eq!(snap.chunks_stored, 0);
    }

    #[test]
    fn test_multiple_snapshots() {
        let collector = MetricsCollector::new();

        let snap1 = collector.snapshot();
        assert_eq!(snap1.read_ops, 0);

        collector.increment_read_ops();

        let snap2 = collector.snapshot();
        assert_eq!(snap2.read_ops, 1);
    }

    #[test]
    fn test_bytes_counters() {
        let collector = MetricsCollector::new();

        collector.record_bytes_read(1024);
        collector.record_bytes_read(2048);
        collector.record_bytes_written(4096);

        let snap = collector.snapshot();
        assert_eq!(snap.chunks_stored, 0);
        assert_eq!(snap.total_size, 4096);
    }
}
