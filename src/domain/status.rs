//! Status enumerations for runs and task runs.
//!
//! The enumerations here are the *labels* the state machine operates on.
//! The machine that decides which transitions are legal lives in
//! [`crate::state`]; this module intentionally only owns the vocabulary so
//! the machine can be tested independently of storage and transport details.

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumIter, EnumString, VariantArray};

/// Lifecycle status of a [`crate::domain::run::Run`].
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumString,
    EnumIter,
    VariantArray,
    AsRefStr,
    Display,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum RunStatus {
    /// Awaiting a worker to pick the run up from the queue.
    Queued,
    /// At least one task has been dispatched and the run is not finished.
    Running,
    /// Every task ran successfully.
    Succeeded,
    /// A task failed and retries are exhausted (or the run was cancelled by
    /// an operator).
    Failed,
    /// An operator cancelled the run before it reached a terminal state.
    Cancelled,
    /// The run exceeded its deadline before completing.
    TimedOut,
}

impl RunStatus {
    /// Whether the status is a terminal leaf in the run state machine.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut)
    }

    /// Whether a run in this status has work outstanding (queued or running).
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    /// Human-oriented short label used by the CLI and dashboard.
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed out",
        }
    }
}

/// Lifecycle status of a single task execution within a run.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumString,
    EnumIter,
    VariantArray,
    AsRefStr,
    Display,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TaskStatus {
    /// Not yet dispatched; waiting on dependencies.
    Pending,
    /// Dispatched to a handler; awaiting completion.
    Running,
    /// Handler returned success.
    Succeeded,
    /// Handler failed and reattempts are exhausted.
    Failed,
    /// Skipped because an upstream dependency failed.
    Skipped,
}

impl TaskStatus {
    /// Whether the task status is terminal.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Skipped)
    }

    /// Whether the task can still make forward progress.
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}

/// Failure classification attached to a run error.
///
/// Handlers declare whether a failure is likely to succeed on retry; the
/// retry planner uses this to decide whether a second attempt is worthwhile.
/// The status code is machine-readable and stable across versions.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumString,
    EnumIter,
    VariantArray,
    AsRefStr,
    Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FailureKind {
    /// The request was malformed; retrying will not help.
    InvalidInput,
    /// The target system rejected the attempt (4xx); likely permanent.
    Rejected,
    /// A transient upstream problem (5xx, timeout, connection reset).
    TransientFailure,
    /// The task breached its own timeout budget.
    Timeout,
    /// The handler crashed or panicked during execution.
    HandlerCrash,
    /// The run was cancelled while the task was in flight.
    Cancelled,
    /// Permanently exhausted attempts without a more specific classification.
    Exhausted,
}

impl FailureKind {
    /// Whether a failure carrying this kind should be retried by default.
    pub fn retryable(self) -> bool {
        matches!(self, Self::TransientFailure | Self::Timeout | Self::HandlerCrash)
    }

    /// A short stable wire code for API responses and logs.
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::Rejected => "rejected",
            Self::TransientFailure => "transient",
            Self::Timeout => "timeout",
            Self::HandlerCrash => "handler_crash",
            Self::Cancelled => "cancelled",
            Self::Exhausted => "exhausted",
        }
    }
}

/// Priority for a queued run. Higher priority runs are claimed first.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    EnumString,
    EnumIter,
    VariantArray,
    AsRefStr,
    Display,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Priority {
    /// Interactive/urgent runs (e.g. manual operator submission).
    High = 2,
    /// Default priority for scheduled or API-submitted runs.
    #[default]
    Normal = 1,
    /// Bulk or maintenance runs that should yield to higher-priority work.
    Low = 0,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn run_status_serde_round_trips() {
        for status in RunStatus::VARIANTS {
            let json = serde_json::to_string(status).unwrap();
            let back: RunStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, *status);
            assert!(!json.contains(' '));
        }
    }

    #[test]
    fn run_status_labels_and_flags() {
        assert!(RunStatus::Succeeded.is_terminal());
        assert!(RunStatus::TimedOut.is_terminal());
        assert!(!RunStatus::Running.is_terminal());
        assert!(RunStatus::Queued.is_active());
        assert!(!RunStatus::Cancelled.is_active());
        assert_eq!(RunStatus::Failed.label(), "failed");
    }

    #[test]
    fn task_status_flags() {
        assert!(TaskStatus::Succeeded.is_terminal());
        assert!(TaskStatus::Pending.is_active());
        assert!(TaskStatus::Running.is_active());
        assert!(TaskStatus::Failed.is_terminal());
    }

    #[test]
    fn failure_kind_is_parseable() {
        assert!(FailureKind::TransientFailure.retryable());
        assert!(FailureKind::Timeout.retryable());
        assert!(!FailureKind::InvalidInput.retryable());
        assert!(!FailureKind::Rejected.retryable());
        assert_eq!(
            FailureKind::from_str(FailureKind::Timeout.code()).unwrap(),
            FailureKind::Timeout
        );
        assert!(FailureKind::from_str("nope").is_err());
    }

    #[test]
    fn priority_ordering() {
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
        assert_eq!(Priority::default(), Priority::Normal);
    }
}