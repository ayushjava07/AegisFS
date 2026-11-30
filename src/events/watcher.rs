//! The differential event watcher.
//!
//! The watcher turns *persisted* run state changes into events, one poll at a
//! time. It keeps two watermarks (started, finished) initialized to its boot
//! timestamp, so a clean boot replays nothing. Each poll:
//!
//! 1. reads all runs and finds those whose `started_at_ms`/`finished_at_ms`
//!    advanced past the watermarks;
//! 2. orders candidates by `(timestamp, run_id)` for a deterministic delivery
//!    sequence;
//! 3. loads each run's definition, builds the event doc, and dispatches
//!    through the [`Dispatcher`].
//!
//! Watermark updates happen *after* dispatch, and only to the newest value in
//! the batch, so a partially-failed batch is re-observed next poll. The watch
//! is idempotent under restart because boot time re-anchors both watermarks.

use std::sync::Arc;

use crate::clock::Clock;
use crate::domain::run::Run;
use crate::domain::status::RunStatus;
use crate::events::dispatch::{Dispatcher, WebhookSink};
use crate::events::{EventKind, RunEventDoc};
use crate::persistence::Store;

/// Counters from one poll round.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WatchStats {
    /// Runs that crossed the started watermark.
    pub started: usize,
    /// Runs that crossed the finished watermark.
    pub finished: usize,
    /// Event documents dispatched in this round.
    pub dispatched: usize,
}

/// The monotonic cursor over poll observations.
#[derive(Debug, Clone, Copy)]
struct Cursor {
    started_ms: i64,
    finished_ms: i64,
}

/// Walks the store and fans events out to a dispatcher.
#[derive(Debug, Clone)]
pub struct Watcher {
    dispatcher: Dispatcher,
    cursor: Cursor,
}

impl Watcher {
    /// A watcher anchored at the given boot time and bound to a dispatcher.
    pub fn new(dispatcher: Dispatcher, boot_ms: i64) -> Self {
        Self {
            dispatcher,
            cursor: Cursor {
                started_ms: boot_ms,
                finished_ms: boot_ms,
            },
        }
    }

    /// A watcher over the given sink, anchored at `clock.now_ms()`.
    pub fn with_sink(sink: Arc<dyn WebhookSink>, clock: &dyn Clock) -> Self {
        Self::new(Dispatcher::new(sink), clock.now_ms())
    }

    /// One differential pass. Never errors: a backend hiccup is reported as a
    /// zero-stats round so the scheduler loop keeps ticking.
    pub fn poll(&mut self, store: &dyn Store, now_ms: i64) -> WatchStats {
        let runs = match store.list_runs(&crate::persistence::RunFilter::default()) {
            Ok(runs) => runs,
            Err(_) => return WatchStats::default(),
        };

        // Started candidates: running now, started after the watermark.
        let mut started: Vec<(i64, &Run)> = runs
            .iter()
            .filter(|r| r.status == RunStatus::Running)
            .filter_map(|r| r.started_at_ms.map(|at| (at, r)))
            .filter(|(at, _)| *at > self.cursor.started_ms)
            .collect();

        // Finished candidates: terminal with a finished timestamp past the
        // finished watermark (status maps to a terminal kind).
        let mut finished: Vec<(i64, EventKind, &Run)> = runs
            .iter()
            .filter_map(|r| {
                let at = r.finished_at_ms?;
                RunEventDoc::terminal_kind_for(r.status).map(|kind| (at, kind, r))
            })
            .filter(|(at, _, _)| *at > self.cursor.finished_ms)
            .collect();

        started.sort_by_key(|(at, r)| (*at, r.id.clone()));
        finished.sort_by_key(|(at, _, r)| (*at, r.id.clone()));

        let stats = WatchStats {
            started: started.len(),
            finished: finished.len(),
            dispatched: 0,
        };

        // Starts first, then terminals: preserves start-before-finish order
        // for a run that crossed both watermarks in a single poll.
        for (_, run) in &started {
            self.dispatch(EventKind::RunStarted, run, store, now_ms);
        }
        for (_, kind, run) in &finished {
            self.dispatch(*kind, run, store, now_ms);
        }

        if let Some((at, _)) = started.last() {
            self.cursor.started_ms = *at;
        }
        if let Some((at, _, _)) = finished.last() {
            self.cursor.finished_ms = *at;
        }

        WatchStats {
            dispatched: stats.started + stats.finished,
            ..stats
        }
    }

    /// Loads the definition and dispatches the run event; a vanished
    /// definition is surfaced as a warning, never a panic.
    fn dispatch(&self, kind: EventKind, run: &Run, store: &dyn Store, now_ms: i64) {
        let def = match store.get_workflow(&run.tenant, &run.def_name) {
            Ok(rec) => rec.def,
            Err(err) => {
                tracing::warn!(
                    run = %run.id,
                    kind = %kind,
                    error = %err,
                    "event dropped: definition missing"
                );
                return;
            }
        };
        let ev = RunEventDoc::from_run(run, kind, now_ms);
        self.dispatcher.dispatch(&def, kind, &ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::domain::workflow::{HookSpec, Hooks};
    use crate::persistence::fixtures;
    use crate::persistence::memory::MemoryStore;

    fn store_with_def(hooks: Hooks) -> MemoryStore {
        let store = MemoryStore::new();
        let def = crate::domain::workflow::WorkflowDef {
            tenant: "acme".to_owned(),
            name: "ship".to_owned(),
            hooks,
            ..crate::domain::workflow::WorkflowDef::default()
        };
        store.put_workflow(def).unwrap();
        store
    }

    fn run_at(store: &MemoryStore, id: &str, status: RunStatus, at: i64) {
        let mut run = fixtures::run(id, "acme", "ship", status, at - 10);
        run.started_at_ms = Some(at - 10);
        run.finished_at_ms = Some(at);
        if status == RunStatus::Failed {
            run.error = Some(crate::domain::run::RunError {
                message: "boom".into(),
                kind: crate::domain::status::FailureKind::Rejected,
                task: None,
                depth: 0,
                attempts: 1,
            });
        }
        store.put_run(&run).unwrap();
    }

    fn hook(url: &str) -> HookSpec {
        HookSpec {
            webhook_url: Some(url.to_owned()),
            event_filter: None,
            headers: Default::default(),
        }
    }

    #[test]
    fn poll_fires_started_and_terminal_events_in_order() {
        let hooks = crate::domain::workflow::Hooks {
            on_start: vec![hook("http://h/start")],
            on_success: vec![hook("http://h/success")],
            on_failure: vec![hook("http://h/fail")],
            on_cancel: vec![hook("http://h/cancel")],
        };
        let store = store_with_def(hooks);
        let clock = ManualClock::at(1_000);
        run_at(&store, "rn_b", RunStatus::Failed, 1_200);
        run_at(&store, "rn_a", RunStatus::Succeeded, 1_100);

        let sink = crate::events::dispatch::RecordingSink::default();
        let mut watcher = Watcher::with_sink(Arc::new(sink.clone()), &clock);

        // Runs finished after boot are visible on the first poll, ordered by
        // (finished_at, run_id).
        let stats1 = watcher.poll(&store, 1_300);
        assert_eq!(stats1.started, 0, "no runs were observed Running");
        assert_eq!(stats1.finished, 2);
        assert_eq!(stats1.dispatched, 2);
        let recorded = sink.recorded();
        assert_eq!(recorded.len(), 2);
        let first: RunEventDoc = serde_json::from_str(&recorded[0].body_json).unwrap();
        let second: RunEventDoc = serde_json::from_str(&recorded[1].body_json).unwrap();
        assert_eq!(first.run_id, "rn_a");
        assert_eq!(first.kind, "run.succeeded");
        assert_eq!(second.run_id, "rn_b");
        assert_eq!(second.kind, "run.failed");

        // Repeat poll: nothing new (watermarks advanced).
        let stats2 = watcher.poll(&store, 1_400);
        assert_eq!(stats2.dispatched, 0);
        assert_eq!(sink.count(), 2);
    }

    #[test]
    fn running_runs_fire_started_exactly_once() {
        let hooks = crate::domain::workflow::Hooks {
            on_start: vec![hook("http://h/start")],
            ..Default::default()
        };
        let store = store_with_def(hooks);
        let clock = ManualClock::at(1_000);

        let mut run = fixtures::run("rn_live", "acme", "ship", RunStatus::Running, 1_100);
        run.started_at_ms = Some(1_150);
        store.put_run(&run).unwrap();

        let sink = crate::events::dispatch::RecordingSink::default();
        let mut watcher = Watcher::with_sink(Arc::new(sink.clone()), &clock);
        let stats = watcher.poll(&store, 1_200);
        assert_eq!(stats.started, 1);
        assert_eq!(stats.dispatched, 1);
        let recorded = sink.recorded();
        let doc: RunEventDoc = serde_json::from_str(&recorded[0].body_json).unwrap();
        assert_eq!(doc.kind, "run.started");
        assert_eq!(doc.status, "running");

        let stats2 = watcher.poll(&store, 1_300);
        assert_eq!(stats2.started, 0, "same run must not refire");
        assert_eq!(stats2.dispatched, 0);
    }

    #[test]
    fn timed_out_uses_the_failure_slice() {
        let hooks = crate::domain::workflow::Hooks {
            on_failure: vec![hook("http://h/fail")],
            ..Default::default()
        };
        let store = store_with_def(hooks);
        let clock = ManualClock::at(1_000);
        run_at(&store, "rn_to", RunStatus::TimedOut, 2_100);

        let sink = crate::events::dispatch::RecordingSink::default();
        let mut watcher = Watcher::with_sink(Arc::new(sink.clone()), &clock);
        watcher.poll(&store, 2_200);
        let recorded = sink.recorded();
        assert_eq!(recorded.len(), 1);
        let doc: RunEventDoc = serde_json::from_str(&recorded[0].body_json).unwrap();
        assert_eq!(doc.kind, "run.timed_out");
        assert_eq!(doc.error.as_deref(), None);
    }

    #[test]
    fn cancelled_runs_fire_the_cancel_slice() {
        let hooks = crate::domain::workflow::Hooks {
            on_cancel: vec![hook("http://h/cancel")],
            ..Default::default()
        };
        let store = store_with_def(hooks);
        let clock = ManualClock::at(1_000);
        run_at(&store, "rn_c", RunStatus::Cancelled, 3_000);

        let sink = crate::events::dispatch::RecordingSink::default();
        let mut watcher = Watcher::with_sink(Arc::new(sink.clone()), &clock);
        watcher.poll(&store, 3_100);
        let recorded = sink.recorded();
        assert_eq!(recorded.len(), 1);
        let doc: RunEventDoc = serde_json::from_str(&recorded[0].body_json).unwrap();
        assert_eq!(doc.kind, "run.cancelled");
    }

    #[test]
    fn definitionless_runs_do_not_panic() {
        let store = MemoryStore::new(); // no definition registered
        let clock = ManualClock::at(1_000);
        run_at(&store, "rn_orphan", RunStatus::Succeeded, 1_500);

        let sink = crate::events::dispatch::RecordingSink::default();
        let mut watcher = Watcher::with_sink(Arc::new(sink.clone()), &clock);
        let stats = watcher.poll(&store, 1_600);
        assert_eq!(stats.finished, 1);
        assert_eq!(stats.dispatched, 1);
        assert_eq!(sink.count(), 0, "event dropped because definition is gone");
    }

    #[test]
    fn start_then_finish_in_one_poll_orders_start_first() {
        let hooks = crate::domain::workflow::Hooks {
            on_start: vec![hook("http://h/start")],
            on_success: vec![hook("http://h/success")],
            ..Default::default()
        };
        let store = store_with_def(hooks);
        let clock = ManualClock::at(1_000);
        run_at(&store, "rn_fast", RunStatus::Succeeded, 1_250);

        let sink = crate::events::dispatch::RecordingSink::default();
        let mut watcher = Watcher::with_sink(Arc::new(sink.clone()), &clock);
        // Poll after the run already started+finished: the run is no longer
        // `Running`, so only the terminal event fires (started is observed by
        // a poll that catches it mid-flight).
        let stats = watcher.poll(&store, 1_500);
        assert_eq!(stats.dispatched, 1);
        let recorded = sink.recorded();
        let doc: RunEventDoc = serde_json::from_str(&recorded[0].body_json).unwrap();
        assert_eq!(doc.kind, "run.succeeded");
    }
}
