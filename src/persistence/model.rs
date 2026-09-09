//! Durable records the store layer manages.

use serde::{Deserialize, Serialize};

use crate::domain::ids::RunId;
use crate::domain::{
    run::{Run, TaskRun},
    workflow::WorkflowDef,
};

/// A workflow definition as stored. Definitions are versioned internally; the
/// store keys them by the natural `(tenant, name)` pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRecord {
    /// The definition payload.
    pub def: WorkflowDef,
}

/// A claim token guarding a queue entry's lease. On `claim` the worker proves
/// ownership by presenting the token on subsequent `ack`/`release` calls.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClaimToken(pub String);

impl ClaimToken {
    /// A new random token.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// A new empty token (used by the memory store's free-claim path).
    pub fn empty() -> Self {
        Self(String::new())
    }
}

impl Default for ClaimToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ClaimToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A row in the run queue. The queue is the durable rendezvous between the
/// scheduler's dispatch and the worker pool's pickup: entries are inserted on
/// submit/retry, visible to `scan_ready` once `due_at_ms` passes, and become
/// lease-guarded once a worker `claim`s them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// The run this entry schedules.
    pub run_id: RunId,
    /// Ownership token; empty until claimed.
    pub token: ClaimToken,
    /// Earliest time (epoch ms) the entry may be scanned by a dispatcher.
    pub due_at_ms: i64,
    /// Lease expiry (epoch ms); `None` while unclaimed.
    pub lease_until_ms: Option<i64>,
    /// Dispatcher identity that holds the lease, for diagnostics.
    pub claimed_by: Option<String>,
}

impl QueueEntry {
    /// Whether the entry is currently leased (claimed and not expired).
    pub fn is_leased(&self, now_ms: i64) -> bool {
        matches!(self.lease_until_ms, Some(exp) if exp > now_ms)
    }

    /// Whether the entry is ready to be scanned at `now_ms`.
    pub fn is_ready(&self, now_ms: i64) -> bool {
        self.due_at_ms <= now_ms && !self.is_leased(now_ms)
    }
}

/// A usual batch of records retrieved together for compaction/export.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunBundle {
    /// Run records.
    pub runs: Vec<Run>,
    /// Task-run records indexed the same way the store returned them.
    pub task_runs: Vec<TaskRun>,
}

/// Aggregates a workflow's current stored form: definition plus the latest
/// run. Used by the maintenance reporter, not the hot path.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSummary {
    /// Definition record.
    pub workflow: WorkflowRecord,
    /// Count of runs recorded for the definition.
    pub run_count: u64,
    /// Latest run, when one exists.
    pub latest_run: Option<Run>,
}

impl WorkflowSummary {
    /// Builds a summary from store query results.
    pub fn assemble(workflow: WorkflowRecord, runs: &[Run]) -> Self {
        Self {
            workflow,
            run_count: runs.len() as u64,
            latest_run: runs.first().cloned(),
        }
    }
}