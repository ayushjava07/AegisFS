//! Process-level metrics shared by the API and the scheduler.
//!
//! Counters are lock-free atomics bumped at well-defined points in the
//! request/dispatch path. A snapshot is served over `/v1/debug/metrics`
//! alongside a few store-derived gauges so operators can size the queue and
//! spot stuck runs without scraping an external system.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The immutable (for this build) crate lineage reported by the debug pages.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runvane's metric counters.
///
/// Naming is deliberate: `_total` suffixes follow the Prometheus convention
/// for monotonic counters; gauges are computed at snapshot time.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Workflow-payload creations accepted over any transport.
    pub workflow_creations_total: AtomicU64,
    /// Run submissions accepted over any transport.
    pub run_submissions_total: AtomicU64,
    /// Cancel requests that flipped a non-terminal run.
    pub run_cancellations_total: AtomicU64,
    /// Queue-pollers dispatched by the scheduler thread since boot.
    pub dispatches_total: AtomicU64,
    /// Terminal runs observed by the differential watcher.
    pub watch_terminals_total: AtomicU64,
    /// Webhook/recording deliveries attempted (success or not).
    pub event_deliveries_total: AtomicU64,
    /// Delivery attempts that completed without a sink error.
    pub event_deliveries_ok_total: AtomicU64,
    /// Finished runs deleted by the retention maintenance pass.
    pub reaped_runs_total: AtomicU64,
}

impl Metrics {
    /// Creates an empty counter set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshots every counter without disturbing them.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            version: VERSION.to_owned(),
            workflow_creations_total: self.workflow_creations_total.load(Ordering::Relaxed),
            run_submissions_total: self.run_submissions_total.load(Ordering::Relaxed),
            run_cancellations_total: self.run_cancellations_total.load(Ordering::Relaxed),
            dispatches_total: self.dispatches_total.load(Ordering::Relaxed),
            watch_terminals_total: self.watch_terminals_total.load(Ordering::Relaxed),
            event_deliveries_total: self.event_deliveries_total.load(Ordering::Relaxed),
            event_deliveries_ok_total: self.event_deliveries_ok_total.load(Ordering::Relaxed),
            reaped_runs_total: self.reaped_runs_total.load(Ordering::Relaxed),
        }
    }
}

/// A point-in-time view of the counters plus store-derived gauges.
#[derive(Debug, serde::Serialize)]
pub struct MetricsSnapshot {
    /// Crate version this snapshot came from.
    pub version: String,
    /// Definitions created over any transport.
    pub workflow_creations_total: u64,
    /// Run submissions accepted over any transport.
    pub run_submissions_total: u64,
    /// Cancel requests that flipped a non-terminal run.
    pub run_cancellations_total: u64,
    /// Queue-pollers dispatched by the scheduler thread.
    pub dispatches_total: u64,
    /// Terminal runs observed by the differential watcher.
    pub watch_terminals_total: u64,
    /// Event deliveries attempted (matching hooks).
    pub event_deliveries_total: u64,
    /// Deliveries that completed without a sink error.
    pub event_deliveries_ok_total: u64,
    /// Finished runs deleted by the retention maintenance pass.
    pub reaped_runs_total: u64,
}

/// Convenience constructor for the shared arc used by handlers and threads.
pub fn shared() -> Arc<Metrics> {
    Arc::new(Metrics::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_and_snapshot_does_not_reset() {
        let metrics = Metrics::new();
        metrics.run_submissions_total.store(3, Ordering::Relaxed);
        metrics
            .event_deliveries_ok_total
            .store(1, Ordering::Relaxed);

        let snap = metrics.snapshot();
        assert_eq!(snap.run_submissions_total, 3);
        assert_eq!(snap.event_deliveries_ok_total, 1);
        assert_eq!(snap.event_deliveries_total, 0);
        assert_eq!(snap.version, VERSION);

        // Snapshotting is read-only; the counters survive it.
        metrics
            .run_submissions_total
            .fetch_add(1, Ordering::Relaxed);
        assert_eq!(metrics.snapshot().run_submissions_total, 4);
    }
}
