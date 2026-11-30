//! The task-run state machine.
//!
//! Tasks are the smallest schedulable unit. Their machine is intentionally
//! narrower than the run machine: a task can be delayed until dependencies
//! are met (`Pending`), run, retried, or skipped when an upstream task fails.

use crate::domain::status::TaskStatus;
use crate::error::RunvaneError;
use crate::state::machine::{apply, StateLabel, Transition, TransitionTable};

impl StateLabel for TaskStatus {
    fn is_terminal(self) -> bool {
        self.is_terminal()
    }
}

/// The singleton task transition table.
pub const TASK_TABLE: TransitionTable<TaskStatus> = TransitionTable::new(can_transition, all_task);

/// Every task status, in declaration order.
pub const fn all_task() -> &'static [TaskStatus] {
    &[
        TaskStatus::Pending,
        TaskStatus::Running,
        TaskStatus::Succeeded,
        TaskStatus::Failed,
        TaskStatus::Skipped,
    ]
}

/// The legal-transition predicate for tasks.
///
/// ```text
///   Pending ──► Running ──► Succeeded
///      │ │        │
///      │ └──► Failed ┤
///      └──► Skipped   └─► Running   (retry)
/// ```
pub const fn can_transition(from: TaskStatus, to: TaskStatus) -> bool {
    use TaskStatus::*;
    matches!(
        (from, to),
        (Pending, Running)
            | (Pending, Failed)
            | (Pending, Skipped)
            | (Running, Succeeded)
            | (Running, Failed)
            | (Failed, Running)
    )
}

/// Whether the task transition is legal.
pub fn is_legal(from: TaskStatus, to: TaskStatus) -> bool {
    TASK_TABLE.is_legal(from, to)
}

/// Validates a task transition.
pub fn validate(from: TaskStatus, to: TaskStatus) -> Result<(), RunvaneError> {
    if is_legal(from, to) {
        Ok(())
    } else {
        Err(RunvaneError::from(
            crate::domain::error::DomainError::IllegalTaskTransition(from, to),
        ))
    }
}

/// Exhaustive legal task edges.
pub fn legal_transitions() -> Vec<(TaskStatus, TaskStatus)> {
    TASK_TABLE.legal_edges()
}

/// All task statuses reachable in one step from `from`.
pub fn allowed_targets(from: TaskStatus) -> Vec<TaskStatus> {
    TASK_TABLE.allowed_targets(from)
}

/// Records a task transition after validation.
pub fn record(
    from: TaskStatus,
    to: TaskStatus,
    at_ms: i64,
) -> Result<Transition<TaskStatus>, RunvaneError> {
    match apply(&TASK_TABLE, from, to, at_ms) {
        crate::state::machine::ApplyResult::Applied(t) => Ok(t),
        crate::state::machine::ApplyResult::Illegal(_) => Err(RunvaneError::from(
            crate::domain::error::DomainError::IllegalTaskTransition(from, to),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::status::TaskStatus::*;
    use std::collections::BTreeSet;

    #[test]
    fn task_legal_edges_match_spec() {
        let expected: BTreeSet<(TaskStatus, TaskStatus)> = BTreeSet::from([
            (Pending, Running),
            (Pending, Failed),
            (Pending, Skipped),
            (Running, Succeeded),
            (Running, Failed),
            (Failed, Running),
        ]);
        assert_eq!(
            legal_transitions().into_iter().collect::<BTreeSet<_>>(),
            expected
        );
    }

    #[test]
    fn every_legal_edge_validates() {
        for (from, to) in legal_transitions() {
            assert!(validate(from, to).is_ok(), "{from:?}->{to:?}");
        }
    }

    #[test]
    fn representative_illegal_task_edges() {
        let illegal: &[(TaskStatus, TaskStatus)] = &[
            (Succeeded, Pending),
            (Succeeded, Running),
            (Skipped, Running),
            (Running, Skipped),
            (Pending, Succeeded),
            (Succeeded, Failed),
            (Skipped, Skipped),
        ];
        for &(from, to) in illegal {
            assert!(!is_legal(from, to), "{from:?}->{to:?} must be illegal");
            assert!(record(from, to, 0).is_err());
        }
    }

    #[test]
    fn terminal_tasks_have_no_edges() {
        assert!(allowed_targets(Succeeded).is_empty());
        assert!(allowed_targets(Skipped).is_empty());
    }

    #[test]
    fn retry_is_failed_back_to_running() {
        assert!(is_legal(Failed, Running));
        assert!(!is_legal(Failed, Succeeded));
        assert!(!is_legal(Failed, Skipped));
    }

    #[test]
    fn pending_abort_paths() {
        assert!(is_legal(Pending, Failed));
        assert!(is_legal(Pending, Skipped));
    }
}
