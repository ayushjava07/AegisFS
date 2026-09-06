//! The workflow-run state machine.
//!
//! One authoritative table decides which run-status transitions are legal.
//! Every persistence `update`, scheduler dispatch, and API response relies on
//! this table; the exhaustive tests below pin both the allowed edges and a
//! representative sample of the disallowed ones so a widening of the machine
//! can never slip through unnoticed.
//!
//! ```text
//!    Queued ──► Running ──► Succeeded
//!      │  ▲      │           ▲
//!      │  │      ├──► Failed ┘
//!      │  │      │     ▲
//!      │  │      └──► TimedOut
//!      │  │
//!      ▼  └── Cancelled ◄──── Failed
//! ```
//! `Failed -> Queued` is the retry edge: the retry planner re-enqueues a
//! failed run whose policy still has attempts left.

use crate::domain::status::RunStatus;
use crate::error::RunvaneError;
use crate::state::machine::{apply, StateLabel, Transition, TransitionTable};

impl StateLabel for RunStatus {
    fn is_terminal(self) -> bool {
        self.is_terminal()
    }
}

/// The singleton run transition table.
pub const RUN_TABLE: TransitionTable<RunStatus> = TransitionTable::new(can_transition, all_run);

/// Every run status, in declaration order.
pub const fn all_run() -> &'static [RunStatus] {
    &[
        RunStatus::Queued,
        RunStatus::Running,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::TimedOut,
    ]
}

/// The legal-transition predicate for runs. See the diagram above.
pub const fn can_transition(from: RunStatus, to: RunStatus) -> bool {
    use RunStatus::*;
    matches!(
        (from, to),
        (Queued, Running)
            | (Queued, Cancelled)
            | (Running, Succeeded)
            | (Running, Failed)
            | (Running, Cancelled)
            | (Running, TimedOut)
            | (Failed, Queued)
            | (Failed, Cancelled)
    )
}

/// Whether the transition is legal.
pub fn is_legal(from: RunStatus, to: RunStatus) -> bool {
    RUN_TABLE.is_legal(from, to)
}

/// Validates a transition, returning `DomainError::IllegalTransition` when
/// the edge is not part of the machine.
pub fn validate(from: RunStatus, to: RunStatus) -> Result<(), RunvaneError> {
    if is_legal(from, to) {
        Ok(())
    } else {
        Err(RunvaneError::from(
            crate::domain::error::DomainError::IllegalTransition(from, to),
        ))
    }
}

/// Exhaustive list of legal `(from, to)` pairs.
pub fn legal_transitions() -> Vec<(RunStatus, RunStatus)> {
    RUN_TABLE.legal_edges()
}

/// All statuses reachable in one step from `from`.
pub fn allowed_targets(from: RunStatus) -> Vec<RunStatus> {
    RUN_TABLE.allowed_targets(from)
}

/// Records a transition after validation, returning the applied record.
pub fn record(
    from: RunStatus,
    to: RunStatus,
    at_ms: i64,
) -> Result<Transition<RunStatus>, RunvaneError> {
    match apply(&RUN_TABLE, from, to, at_ms) {
        crate::state::machine::ApplyResult::Applied(t) => Ok(t),
        crate::state::machine::ApplyResult::Illegal(_) => Err(RunvaneError::from(
            crate::domain::error::DomainError::IllegalTransition(from, to),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::status::RunStatus::*;
    use std::collections::BTreeSet;

    /// Every legal edge, exactly, as a set — a hard pin of the machine shape.
    fn expected_legal() -> BTreeSet<(RunStatus, RunStatus)> {
        BTreeSet::from([
            (Queued, Running),
            (Queued, Cancelled),
            (Running, Succeeded),
            (Running, Failed),
            (Running, Cancelled),
            (Running, TimedOut),
            (Failed, Queued),
            (Failed, Cancelled),
        ])
    }

    #[test]
    fn legal_table_matches_spec() {
        assert_eq!(
            legal_transitions().into_iter().collect::<BTreeSet<_>>(),
            expected_legal()
        );
    }

    #[test]
    // [F2P] RV-004 witness (every legal edge must keep validating); a
    // transition-order regression breaks this exhaustive table.
    fn every_legal_edge_validates() {
        for (from, to) in legal_transitions() {
            assert!(
                validate(from, to).is_ok(),
                "{from:?} -> {to:?} should be legal"
            );
            assert!(record(from, to, 0).is_ok());
        }
    }

    #[test]
    fn representative_illegal_edges() {
        // >=3 illegal transitions explicitly pinned (table-driven).
        let illegal: &[(RunStatus, RunStatus)] = &[
            (Succeeded, Queued),
            (Succeeded, Running),
            (Queued, Succeeded),
            (Running, Running),
            (Cancelled, Running),
            (TimedOut, Queued),
            (Queued, TimedOut),
            (Succeeded, Succeeded),
        ];
        for &(from, to) in illegal {
            assert!(!is_legal(from, to), "{from:?} -> {to:?} must be illegal");
            assert!(record(from, to, 0).is_err());
        }
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        for from in [Succeeded, Cancelled, TimedOut] {
            assert!(allowed_targets(from).is_empty());
        }
    }

    #[test]
    fn retry_edge_is_failed_to_queued_only() {
        // The only way back into the queue is via a failed run being retried.
        assert!(is_legal(Failed, Queued));
        assert!(!is_legal(TimedOut, Queued));
        assert!(!is_legal(Cancelled, Queued));
    }

    #[test]
    fn queued_runs_can_only_advance() {
        let targets = allowed_targets(Queued);
        assert!(targets.contains(&Running));
        assert!(targets.contains(&Cancelled));
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn record_marks_timestamps() {
        let t = record(Queued, Running, 1234).unwrap();
        assert_eq!(t.from, Queued);
        assert_eq!(t.to, Running);
        assert_eq!(t.at_ms, 1234);
    }
}
