//! Process-level metrics shared by the API and the scheduler.
//!
//! Counters are lock-free atomics bumped at well-defined points in the
//! request/dispatch path. A snapshot is served over `/v1/debug/metrics`
//! alongside a few store-derived gauges so operators can size the queue and
//! spot stuck runs without scraping an external system.
//!
//! In addition, Prometheus text exposition is supported via [`Metrics::render_prometheus`].

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The immutable (for this build) crate lineage reported by the debug pages.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cumulative duration histogram for tracking latencies in seconds.
#[derive(Debug)]
pub struct Histogram {
    buckets: Vec<f64>,
    counts: Vec<AtomicU64>,
    count: AtomicU64,
    sum: parking_lot::Mutex<f64>,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::default_latencies()
    }
}

impl Histogram {
    /// Creates a new histogram with custom bucket thresholds (in ascending order).
    pub fn new(buckets: Vec<f64>) -> Self {
        let n = buckets.len();
        let counts = (0..n).map(|_| AtomicU64::new(0)).collect();
        Self {
            buckets,
            counts,
            count: AtomicU64::new(0),
            sum: parking_lot::Mutex::new(0.0),
        }
    }

    /// Creates a histogram configured with standard latency buckets from 5ms to 60s.
    pub fn default_latencies() -> Self {
        Self::new(vec![
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
        ])
    }

    /// Observes a value in seconds, incrementing all matching cumulative buckets.
    pub fn observe(&self, value: f64) {
        if value.is_nan() || value < 0.0 {
            return;
        }
        for (i, &upper) in self.buckets.iter().enumerate() {
            if value <= upper {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        let mut sum = self.sum.lock();
        *sum += value;
    }

    /// Takes an immutable snapshot of the histogram state.
    pub fn snapshot(&self) -> HistogramSnapshot {
        let buckets = self
            .buckets
            .iter()
            .zip(self.counts.iter())
            .map(|(&le, count)| (le, count.load(Ordering::Relaxed)))
            .collect();
        HistogramSnapshot {
            buckets,
            count: self.count.load(Ordering::Relaxed),
            sum: *self.sum.lock(),
        }
    }
}

/// An immutable snapshot of a [`Histogram`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistogramSnapshot {
    /// Pairs of `(upper_bound, cumulative_count)`.
    pub buckets: Vec<(f64, u64)>,
    /// Total number of observations.
    pub count: u64,
    /// Arithmetic sum of all observed values.
    pub sum: f64,
}

/// Runvane's metric counters and histograms.
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
    /// Distributed lease renewal heartbeats successfully applied.
    pub lease_renewals_total: AtomicU64,
    /// Run execution durations in seconds.
    pub run_duration: Histogram,
    /// Task execution attempt durations in seconds.
    pub task_duration: Histogram,
}

/// Gauge values collected at scrape time for Prometheus exposition.
#[derive(Debug, Default, Clone)]
pub struct PrometheusGauges {
    /// Instantaneous depth of the ready dispatch queue.
    pub queue_depth: u64,
    /// Total number of unique registered workflows.
    pub workflows_count: u64,
    /// Total number of runs currently tracked in store.
    pub runs_count: u64,
    /// Uptime of the server process in seconds.
    pub uptime_seconds: u64,
}

impl Metrics {
    /// Creates an empty counter and histogram set.
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
            lease_renewals_total: self.lease_renewals_total.load(Ordering::Relaxed),
            run_duration: self.run_duration.snapshot(),
            task_duration: self.task_duration.snapshot(),
        }
    }

    /// Renders standard Prometheus text format exposition (v0.0.4).
    pub fn render_prometheus(&self, gauges: &PrometheusGauges) -> String {
        let mut out = String::with_capacity(2048);

        // Counters
        let _ = writeln!(
            out,
            "# HELP runvane_workflow_creations_total Total registered workflows."
        );
        let _ = writeln!(out, "# TYPE runvane_workflow_creations_total counter");
        let _ = writeln!(
            out,
            "runvane_workflow_creations_total {}",
            self.workflow_creations_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_run_submissions_total Total runs submitted."
        );
        let _ = writeln!(out, "# TYPE runvane_run_submissions_total counter");
        let _ = writeln!(
            out,
            "runvane_run_submissions_total {}",
            self.run_submissions_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_run_cancellations_total Total runs cancelled."
        );
        let _ = writeln!(out, "# TYPE runvane_run_cancellations_total counter");
        let _ = writeln!(
            out,
            "runvane_run_cancellations_total {}",
            self.run_cancellations_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_dispatches_total Total scheduler poll dispatches."
        );
        let _ = writeln!(out, "# TYPE runvane_dispatches_total counter");
        let _ = writeln!(
            out,
            "runvane_dispatches_total {}",
            self.dispatches_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_watch_terminals_total Terminal runs observed by event watcher."
        );
        let _ = writeln!(out, "# TYPE runvane_watch_terminals_total counter");
        let _ = writeln!(
            out,
            "runvane_watch_terminals_total {}",
            self.watch_terminals_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_event_deliveries_total Webhook delivery attempts."
        );
        let _ = writeln!(out, "# TYPE runvane_event_deliveries_total counter");
        let _ = writeln!(
            out,
            "runvane_event_deliveries_total {}",
            self.event_deliveries_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_event_deliveries_ok_total Successful webhook deliveries."
        );
        let _ = writeln!(out, "# TYPE runvane_event_deliveries_ok_total counter");
        let _ = writeln!(
            out,
            "runvane_event_deliveries_ok_total {}",
            self.event_deliveries_ok_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_reaped_runs_total Stale runs purged by retention cleaner."
        );
        let _ = writeln!(out, "# TYPE runvane_reaped_runs_total counter");
        let _ = writeln!(
            out,
            "runvane_reaped_runs_total {}",
            self.reaped_runs_total.load(Ordering::Relaxed)
        );

        let _ = writeln!(
            out,
            "# HELP runvane_lease_renewals_total Distributed task lease renewals."
        );
        let _ = writeln!(out, "# TYPE runvane_lease_renewals_total counter");
        let _ = writeln!(
            out,
            "runvane_lease_renewals_total {}",
            self.lease_renewals_total.load(Ordering::Relaxed)
        );

        // Gauges
        let _ = writeln!(out, "# HELP runvane_queue_depth Instantaneous queue depth.");
        let _ = writeln!(out, "# TYPE runvane_queue_depth gauge");
        let _ = writeln!(out, "runvane_queue_depth {}", gauges.queue_depth);

        let _ = writeln!(out, "# HELP runvane_workflows Total registered workflows.");
        let _ = writeln!(out, "# TYPE runvane_workflows gauge");
        let _ = writeln!(out, "runvane_workflows {}", gauges.workflows_count);

        let _ = writeln!(out, "# HELP runvane_runs Total runs stored.");
        let _ = writeln!(out, "# TYPE runvane_runs gauge");
        let _ = writeln!(out, "runvane_runs {}", gauges.runs_count);

        let _ = writeln!(
            out,
            "# HELP runvane_uptime_seconds Process uptime in seconds."
        );
        let _ = writeln!(out, "# TYPE runvane_uptime_seconds gauge");
        let _ = writeln!(out, "runvane_uptime_seconds {}", gauges.uptime_seconds);

        // Histograms
        render_histogram_prometheus(
            &mut out,
            "runvane_run_duration_seconds",
            "Run execution duration in seconds.",
            &self.run_duration.snapshot(),
        );
        render_histogram_prometheus(
            &mut out,
            "runvane_task_duration_seconds",
            "Task execution attempt duration in seconds.",
            &self.task_duration.snapshot(),
        );

        out
    }
}

fn render_histogram_prometheus(out: &mut String, name: &str, help: &str, snap: &HistogramSnapshot) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} histogram");
    for (upper, count) in &snap.buckets {
        let _ = writeln!(out, "{name}_bucket{{le=\"{upper}\"}} {count}");
    }
    let _ = writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {}", snap.count);
    let _ = writeln!(out, "{name}_sum {}", snap.sum);
    let _ = writeln!(out, "{name}_count {}", snap.count);
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
    /// Distributed lease renewals successfully recorded.
    pub lease_renewals_total: u64,
    /// Run duration histogram snapshot.
    pub run_duration: HistogramSnapshot,
    /// Task duration histogram snapshot.
    pub task_duration: HistogramSnapshot,
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

    #[test]
    fn histogram_observe_and_prometheus_rendering() {
        let metrics = Metrics::new();
        metrics.run_duration.observe(0.04);
        metrics.run_duration.observe(0.5);
        metrics.run_duration.observe(2.0);

        let snap = metrics.run_duration.snapshot();
        assert_eq!(snap.count, 3);
        assert!((snap.sum - 2.54).abs() < 1e-6);

        let gauges = PrometheusGauges {
            queue_depth: 5,
            workflows_count: 2,
            runs_count: 10,
            uptime_seconds: 120,
        };

        let rendered = metrics.render_prometheus(&gauges);
        assert!(rendered.contains("runvane_run_duration_seconds_count 3"));
        assert!(rendered.contains("runvane_queue_depth 5"));
        assert!(rendered.contains("runvane_uptime_seconds 120"));
        assert!(rendered.contains("runvane_run_duration_seconds_bucket{le=\"0.05\"} 1"));
        assert!(rendered.contains("runvane_run_duration_seconds_bucket{le=\"+Inf\"} 3"));
    }
}
