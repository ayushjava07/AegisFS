//! Cross-entity invariants checked by tests, compaction, and the dashboard.
//!
//! A run alone cannot prove consistency: its status must agree with the
//! statuses of its task runs. These checks are pure functions over a small
//! aggregate (`RunStatus` + task statuses) so they can be property-tested
//! exhaustively and reused by the maintenance workers.

use crate::domain::status::{RunStatus, TaskStatus};

/// An invariant violation detected across a run and its tasks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvariantError {
    /// A run claims success while at least one task is not terminal.
    #[error("run is Succeeded but task {0} is not terminal")]
    SucceededWithNonTerminalTask(String),
    /// A run ended in failure while a task was still running.
    #[error("run is terminal-failed but task {0} is Running")]
    TerminalWithRunningTask(String),
    /// A run is still active but every task is terminal (cannot progress).
    #[error("run is Running with no progress possible")]
    RunBlocked,
    /// A run is active but has no recorded task runs at all.
    #[error("run has no task runs and is not terminal")]
    ActiveWithNoTasks,
    /// A task was skipped although no upstream task failed.
    #[error("task {0} is Skipped with no failed dependency")]
    SkippedWithoutFailedDependency(String),
}

/// Checks invariants of a run's status combined with its task statuses.
/// `tasks` is a `(task_name, status)` slice; empty slices are only valid for
/// a non-Running run.
pub fn check_run_consistency(
    run: RunStatus,
    tasks: &[(&str, TaskStatus)],
) -> Result<(), InvariantError> {
    let all_terminal = tasks.iter().all(|(_, s)| s.is_terminal());
    let any_running = tasks.iter().any(|(_, s)| *s == TaskStatus::Running);

    // A terminal failure must not leave a task Running.
    if (run == RunStatus::Failed || run == RunStatus::TimedOut) && any_running {
        let name = tasks
            .iter()
            .find(|(_, s)| *s == TaskStatus::Running)
            .unwrap()
            .0;
        return Err(InvariantError::TerminalWithRunningTask((*name).to_owned()));
    }

    // Succeeded requires every task terminal.
    if run == RunStatus::Succeeded {
        if !all_terminal {
            let (name, _) = tasks.iter().find(|(_, s)| !s.is_terminal()).unwrap();
            return Err(InvariantError::SucceededWithNonTerminalTask(
                (*name).to_owned(),
            ));
        }
        return Ok(());
    }

    // Running runs must have tasks still capable of progress.
    if run == RunStatus::Running {
        if tasks.is_empty() {
            return Err(InvariantError::ActiveWithNoTasks);
        }
        if all_terminal {
            return Err(InvariantError::RunBlocked);
        }
    }

    // Skipped tasks require a failed dependency — except when the run was
    // cancelled by an operator (pending tasks are skipped without a failure).
    let needs_failed_dep =
        run == RunStatus::Failed || run == RunStatus::TimedOut || run == RunStatus::Succeeded;
    if needs_failed_dep {
        let has_failed = tasks.iter().any(|(_, s)| *s == TaskStatus::Failed);
        if let Some((name, _)) = tasks.iter().find(|(_, s)| *s == TaskStatus::Skipped) {
            if !has_failed {
                return Err(InvariantError::SkippedWithoutFailedDependency(
                    (*name).to_owned(),
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::status::RunStatus::{
        Cancelled as RCan, Failed as RFail, Queued as RQue, Running as RRun, Succeeded as RSuc,
        TimedOut as RTO,
    };
    use crate::domain::status::TaskStatus::{
        Failed as TFail, Running as TRun, Skipped as TSkip, Succeeded as TSuc,
    };

    fn t<'a>(items: &'a [(&'a str, TaskStatus)]) -> Vec<(&'a str, TaskStatus)> {
        items.to_vec()
    }

    #[test]
    fn succeeded_requires_all_terminal() {
        let ok_tasks = t(&[("a", TSuc), ("b", TSuc)]);
        assert!(check_run_consistency(RSuc, &ok_tasks).is_ok());

        let bad = t(&[("a", TSuc), ("b", TRun)]);
        assert_eq!(
            check_run_consistency(RSuc, &bad),
            Err(InvariantError::SucceededWithNonTerminalTask("b".into()))
        );
    }

    #[test]
    fn terminal_failure_with_running_task() {
        let bad = t(&[("a", TRun), ("b", TFail)]);
        assert_eq!(
            check_run_consistency(RFail, &bad),
            Err(InvariantError::TerminalWithRunningTask("a".into()))
        );
    }

    #[test]
    fn running_run_with_no_work_is_blocked() {
        let all_done = t(&[("a", TSuc), ("b", TFail)]);
        assert_eq!(
            check_run_consistency(RRun, &all_done),
            Err(InvariantError::RunBlocked)
        );
    }

    #[test]
    fn running_run_requires_tasks() {
        assert_eq!(
            check_run_consistency(RRun, &[]),
            Err(InvariantError::ActiveWithNoTasks)
        );
    }

    #[test]
    fn skipped_requires_failed_dependency() {
        let t = vec![("a", TSkip), ("b", TSuc)];
        assert_eq!(
            check_run_consistency(RFail, &t),
            Err(InvariantError::SkippedWithoutFailedDependency("a".into()))
        );
        // With a failed task present, skipped is explainable.
        let explainable = vec![("a", TSkip), ("b", TFail)];
        assert!(check_run_consistency(RFail, &explainable).is_ok());
    }

    #[test]
    fn queued_run_with_no_tasks_is_fine() {
        assert!(check_run_consistency(RQue, &[]).is_ok());
        // Operator cancellation skips pending tasks without a failed dependency.
        assert!(check_run_consistency(RCan, &[("a", TSkip)]).is_ok());
        // A timed-out run with a skipped task and no failure is suspicious.
        assert_eq!(
            check_run_consistency(RTO, &[("a", TSkip)]),
            Err(InvariantError::SkippedWithoutFailedDependency("a".into()))
        );
    }
}
