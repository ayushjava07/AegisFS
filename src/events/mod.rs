//! Event model for the webhook/hook subsystem.
//!
//! Events are produced from run lifecycle transitions (started, and each
//! terminal state) and consumed by hook deliveries. The model is wire-agnostic
//! JSON: the same document that goes out over a webhook is what the dispatch
//! layer logs and what tests assert on. Emitting is a side effect of the
//! scheduler, never of the API handlers, so there is exactly one path that
//! turns a state change into an event.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::Digest;

use crate::domain::ids::RunId;
use crate::domain::run::{Run, RunError};

pub mod dispatch;
pub mod outbox;
pub mod watcher;

/// The lifecycle milestones a hook can be attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventKind {
    /// The run was picked up for the first time (`Queued -> Running`).
    RunStarted,
    /// The run reached `Succeeded`.
    RunSucceeded,
    /// The run reached `Failed`.
    RunFailed,
    /// The run reached `TimedOut`.
    RunTimedOut,
    /// The run was cancelled (from `Queued` or `Running`).
    RunCancelled,
}

impl EventKind {
    /// Stable machine-readable code used in the event document and by
    /// `HookSpec::event_filter` matching.
    pub fn code(&self) -> &'static str {
        match self {
            Self::RunStarted => "run.started",
            Self::RunSucceeded => "run.succeeded",
            Self::RunFailed => "run.failed",
            Self::RunTimedOut => "run.timed_out",
            Self::RunCancelled => "run.cancelled",
        }
    }

    /// Whether this kind is a terminal (run over) milestone.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::RunStarted)
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// The document delivered to subscribers. Field names are the wire contract;
/// keep them stable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEventDoc {
    /// Event kind code, e.g. `run.succeeded`.
    pub kind: String,
    /// Run id this event is about.
    pub run_id: String,
    /// Tenant namespace.
    pub tenant: String,
    /// Definition name.
    pub def_name: String,
    /// Definition version snapshot the run executed against.
    pub def_version: u32,
    /// Monotonic per-definition run counter.
    pub run_number: u64,
    /// Run status at the time of emission.
    pub status: String,
    /// First-dispatch timestamp.
    pub started_at_ms: Option<i64>,
    /// Terminal timestamp (absent for `run.started`).
    pub finished_at_ms: Option<i64>,
    /// Attempts consumed when the event fired.
    pub attempts: u32,
    /// Terminal error message, when the terminal state carries one.
    pub error: Option<String>,
    /// Final run output, when `run.succeeded`.
    pub output: Option<Json>,
    /// Event emission timestamp.
    pub created_at_ms: i64,
}

impl RunEventDoc {
    /// Builds an event document from a run snapshot at the given time.
    pub fn from_run(run: &Run, kind: EventKind, created_at_ms: i64) -> Self {
        let error = run.error.as_ref().map(|e: &RunError| e.message.clone());
        let output = match kind {
            EventKind::RunSucceeded => run.output.clone(),
            _ => None,
        };
        Self {
            kind: kind.code().to_owned(),
            run_id: run.id.as_str().to_owned(),
            tenant: run.tenant.clone(),
            def_name: run.def_name.clone(),
            def_version: run.def_version,
            run_number: run.run_number,
            status: serde_json::to_value(run.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
            started_at_ms: run.started_at_ms,
            finished_at_ms: run.finished_at_ms,
            attempts: run.attempts,
            error,
            output,
            created_at_ms,
        }
    }

    /// Normalized raw tags of the run (omitted from the serialized document to
    /// keep the payload small; subscribers can join via the run id).
    ///
    /// Kept as an associated function rather than a field so the payload stays
    /// stable even if the run gains new tags.
    pub fn tags_snapshot(run: &Run) -> BTreeMap<String, String> {
        run.tags.clone()
    }

    /// The terminal kind matching a run status, if any.
    pub fn terminal_kind_for(status: crate::domain::status::RunStatus) -> Option<EventKind> {
        match status {
            crate::domain::status::RunStatus::Succeeded => Some(EventKind::RunSucceeded),
            crate::domain::status::RunStatus::Failed => Some(EventKind::RunFailed),
            crate::domain::status::RunStatus::TimedOut => Some(EventKind::RunTimedOut),
            crate::domain::status::RunStatus::Cancelled => Some(EventKind::RunCancelled),
            _ => None,
        }
    }
}

/// Deterministic delivery id for a run+kind pair: stable across retries so
/// webhook consumers can deduplicate.
pub fn delivery_id(run_id: &RunId, kind: EventKind, tenant: &str, def_name: &str) -> String {
    let raw = format!("{tenant}/{def_name}/{run_id}/{}", kind.code());
    hex::encode(sha2::Sha256::digest(raw.as_bytes()))
}
