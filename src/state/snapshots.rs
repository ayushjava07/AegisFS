//! Point-in-time snapshots of a run for observation layers.
//!
//! The dashboard, CLI, and audit trail all want a consistent picture of a run
//! and its tasks. The persistence layer writes these aggregates as a unit
//! when returning a run detail, so consumers never have to reconcile two
//! independently-fetched records.

use crate::domain::run::{Run, TaskRun};
use crate::domain::status::{RunStatus, TaskStatus};

/// One task's status within a snapshot, with the fields a dashboard row needs.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotTask {
    /// Task name.
    pub name: String,
    /// Task status.
    pub status: TaskStatus,
    /// Attempts consumed.
    pub attempts: u32,
    /// Last failure message, when present.
    pub last_error: Option<String>,
}

/// A consistent snapshot of one run and its tasks.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSnapshot {
    /// Run id.
    pub run: Run,
    /// Task statuses, in definition order when available.
    pub tasks: Vec<SnapshotTask>,
}

impl RunSnapshot {
    /// Builds a snapshot from a run and its task runs.
    pub fn capture(run: Run, task_runs: Vec<TaskRun>) -> Self {
        let tasks = task_runs
            .into_iter()
            .map(|tr| SnapshotTask {
                name: tr.task_name,
                status: tr.status,
                attempts: tr.attempts,
                last_error: tr.last_error,
            })
            .collect();
        Self { run, tasks }
    }

    /// The run's status.
    pub fn run_status(&self) -> RunStatus {
        self.run.status
    }

    /// Number of tasks that ran (attempted at least once).
    pub fn attempted_tasks(&self) -> usize {
        self.tasks.iter().filter(|t| t.attempts > 0).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{RunId, TaskRunId};
    use crate::domain::run::Run;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn run() -> Run {
        Run {
            id: RunId::from_validated("rn_x".into()),
            tenant: "acme".into(),
            def_name: "nightly".into(),
            def_version: 1,
            input: json!({}),
            status: RunStatus::Running,
            attempts: 1,
            next_attempt_at_ms: None,
            deadline_at_ms: None,
            started_at_ms: Some(1),
            finished_at_ms: None,
            error: None,
            output: None,
            tags: BTreeMap::new(),
            created_at_ms: 0,
            run_number: 1,
        }
    }

    #[test]
    fn capture_projects_task_fields() {
        let task_runs = vec![TaskRun {
            id: TaskRunId::from_validated("tr_x".into()),
            run_id: RunId::from_validated("rn_x".into()),
            task_name: "fetch".into(),
            status: TaskStatus::Failed,
            attempts: 3,
            last_error: Some("down".into()),
            started_at_ms: Some(1),
            finished_at_ms: Some(9),
            output: None,
        }];
        let snap = RunSnapshot::capture(run(), task_runs);
        assert_eq!(snap.run_status(), RunStatus::Running);
        assert_eq!(snap.attempted_tasks(), 1);
        assert_eq!(snap.tasks[0].attempts, 3);
    }

    #[test]
    fn attempted_count_ignores_untouched_tasks() {
        let task_runs = vec![
            TaskRun {
                id: TaskRunId::from_validated("tr_a".into()),
                run_id: RunId::from_validated("rn_x".into()),
                task_name: "a".into(),
                status: TaskStatus::Succeeded,
                attempts: 1,
                last_error: None,
                started_at_ms: Some(1),
                finished_at_ms: Some(2),
                output: None,
            },
            TaskRun {
                id: TaskRunId::from_validated("tr_b".into()),
                run_id: RunId::from_validated("rn_x".into()),
                task_name: "b".into(),
                status: TaskStatus::Pending,
                attempts: 0,
                last_error: None,
                started_at_ms: None,
                finished_at_ms: None,
                output: None,
            },
        ];
        let snap = RunSnapshot::capture(run(), task_runs);
        assert_eq!(snap.attempted_tasks(), 1);
    }
}
