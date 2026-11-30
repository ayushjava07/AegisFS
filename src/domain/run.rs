//! Run and task-run records: the durable state the platform persists.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::ids::{RunId, TaskRunId};
use super::status::{FailureKind, RunStatus, TaskStatus};

/// A run is one submitted execution of a workflow definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Unique run identifier.
    pub id: RunId,
    /// Owning tenant (bound at submission).
    pub tenant: String,
    /// Name of the definition this run executes. Definition snapshot is
    /// captured by `def_version`, so later definition changes never affect an
    /// in-flight run.
    pub def_name: String,
    /// Version of the definition snapshot this run executes.
    pub def_version: u32,
    /// Input payload as submitted.
    pub input: Json,
    /// Current lifecycle status.
    pub status: RunStatus,
    /// Attempts consumed so far (1 after first dispatch).
    pub attempts: u32,
    /// Earliest allowed re-dispatch time, epoch milliseconds.
    pub next_attempt_at_ms: Option<i64>,
    /// Absolute run deadline, epoch milliseconds. `None` means no deadline.
    pub deadline_at_ms: Option<i64>,
    /// First-dispatch timestamp.
    pub started_at_ms: Option<i64>,
    /// Terminal timestamp.
    pub finished_at_ms: Option<i64>,
    /// Terminal error, when the run failed or timed out.
    pub error: Option<RunError>,
    /// Successful final output, when the run succeeded.
    pub output: Option<Json>,
    /// Operator tags captured at submission (used by filtering).
    pub tags: BTreeMap<String, String>,
    /// Submission timestamp.
    pub created_at_ms: i64,
    /// Monotonic submission counter per definition, for display.
    pub run_number: u64,
}

impl Run {
    /// Whether the run is in a terminal status.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

/// Structured error detail attached to a failed or timed-out run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunError {
    /// Human-readable message for operators and the dashboard.
    pub message: String,
    /// Machine-readable failure classification.
    pub kind: FailureKind,
    /// Task that produced the failure, when attributable.
    pub task: Option<String>,
    /// Depth of the failure within the task graph, for display.
    pub depth: u8,
    /// Attempt count at the moment of failure.
    pub attempts: u32,
}

impl RunError {
    /// Renders a compact single-line message for logs.
    pub fn to_log_line(&self) -> String {
        match &self.task {
            Some(task) => format!(
                "[{}] {}: {} (attempt {})",
                self.kind.code(),
                task,
                self.message,
                self.attempts
            ),
            None => format!(
                "[{}] {} (attempt {})",
                self.kind.code(),
                self.message,
                self.attempts
            ),
        }
    }
}

/// One task execution within a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRun {
    /// Unique task-run identifier.
    pub id: TaskRunId,
    /// Owning run.
    pub run_id: RunId,
    /// Task name within the definition.
    pub task_name: String,
    /// Task lifecycle status.
    pub status: TaskStatus,
    /// Attempts consumed.
    pub attempts: u32,
    /// Last failure message attributed to this task.
    pub last_error: Option<String>,
    /// First-dispatch timestamp.
    pub started_at_ms: Option<i64>,
    /// Terminal timestamp.
    pub finished_at_ms: Option<i64>,
    /// Handler output.
    pub output: Option<Json>,
}

impl TaskRun {
    /// Whether the task is terminal.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_run() -> Run {
        Run {
            id: RunId::from_validated("rn_test123".to_owned()),
            tenant: "acme".to_owned(),
            def_name: "nightly".to_owned(),
            def_version: 3,
            input: json!({"bucket": "logs"}),
            status: RunStatus::Queued,
            attempts: 0,
            next_attempt_at_ms: Some(1_700_000_000_000),
            deadline_at_ms: None,
            started_at_ms: None,
            finished_at_ms: None,
            error: None,
            output: None,
            tags: BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
            created_at_ms: 1_700_000_000_000,
            run_number: 12,
        }
    }

    #[test]
    fn run_serde_round_trips() {
        let run = sample_run();
        let json = serde_json::to_string(&run).unwrap();
        let back: Run = serde_json::from_str(&json).unwrap();
        assert_eq!(back, run);
        assert_eq!(back.status, RunStatus::Queued);
    }

    #[test]
    fn run_terminal_flag() {
        assert!(!sample_run().is_terminal());
        let mut failed = sample_run();
        failed.status = RunStatus::Failed;
        assert!(failed.is_terminal());
    }

    #[test]
    fn error_log_line_shapes() {
        let with_task = RunError {
            message: "connection reset".to_owned(),
            kind: FailureKind::TransientFailure,
            task: Some("fetch".to_owned()),
            depth: 1,
            attempts: 2,
        };
        assert_eq!(
            with_task.to_log_line(),
            "[transient] fetch: connection reset (attempt 2)"
        );
        let bare = RunError {
            task: None,
            ..with_task.clone()
        };
        assert_eq!(
            bare.to_log_line(),
            "[transient] connection reset (attempt 2)"
        );
    }

    #[test]
    fn task_run_serde_round_trips() {
        let tr = TaskRun {
            id: TaskRunId::from_validated("tr_test1".to_owned()),
            run_id: RunId::from_validated("rn_test123".to_owned()),
            task_name: "ghost".to_owned(),
            status: TaskStatus::Failed,
            attempts: 2,
            last_error: Some("boom".to_owned()),
            started_at_ms: Some(1),
            finished_at_ms: Some(2),
            output: None,
        };
        let json = serde_json::to_string(&tr).unwrap();
        let back: TaskRun = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tr);
        assert!(back.is_terminal());
    }
}
