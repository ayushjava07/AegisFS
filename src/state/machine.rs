//! A small, generic state-machine core.
//!
//! The machine is deliberately *derived*, not data-driven-from-storage: the
//! transition table is a pure function of the label types, so the full set of
//! legal (and illegal) transitions is enumerable and testable without any
//! I/O. The engine core here stays label-agnostic; the concrete tables for
//! runs and tasks live in [`super::run_fsm`] and [`super::task_fsm`], where
//! the individual transitions carry documentation about *why* each edge
//! exists.

use std::fmt::Debug;
use std::hash::Hash;

/// A label the machine can be in. Labels must be cheap, copyable values that
/// can report whether they are terminal.
pub trait StateLabel: Copy + PartialEq + Eq + Hash + Debug + 'static {
    /// Whether the label is a terminal (no outgoing edges) state.
    fn is_terminal(self) -> bool;
}

/// The transition table of a machine: a pure predicate over (from, to).
///
/// Tables are typically built with a const function or a closure and shared
/// across threads; they carry no mutable state.
#[derive(Clone, Copy)]
pub struct TransitionTable<L>
where
    L: StateLabel + 'static,
{
    legal: fn(L, L) -> bool,
    all: fn() -> &'static [L],
}

impl<L> TransitionTable<L>
where
    L: StateLabel + 'static,
{
    /// Builds a table from a legal-transition predicate and the full label
    /// list (used by `allowed_targets`).
    pub const fn new(legal: fn(L, L) -> bool, all: fn() -> &'static [L]) -> Self {
        Self { legal, all }
    }

    /// The number of labels in the label set.
    pub fn len_labels(&self) -> usize {
        (self.all)().len()
    }

    /// Whether the label set is empty.
    pub fn is_empty(&self) -> bool {
        (self.all)().is_empty()
    }

    /// Whether the transition `from -> to` is legal.
    pub fn is_legal(&self, from: L, to: L) -> bool {
        (self.legal)(from, to)
    }

    /// All labels in the label set, in declaration order.
    pub fn all_labels(&self) -> &'static [L] {
        (self.all)()
    }

    /// All labels reachable in one step from `from`.
    pub fn allowed_targets(&self, from: L) -> Vec<L> {
        self.all_labels()
            .iter()
            .copied()
            .filter(|&to| self.is_legal(from, to))
            .collect()
    }

    /// Enumerates every *legal* transition in the machine as `(from, to)`.
    pub fn legal_edges(&self) -> Vec<(L, L)> {
        let mut edges = Vec::new();
        for &from in self.all_labels() {
            for &to in self.all_labels() {
                if self.is_legal(from, to) {
                    edges.push((from, to));
                }
            }
        }
        edges
    }

    /// Enumerates every *illegal* transition (including all terminal-source
    /// edges). Used by the exhaustive tests that pin the machine shape.
    pub fn illegal_edges(&self) -> Vec<(L, L)> {
        let mut edges = Vec::new();
        for &from in self.all_labels() {
            for &to in self.all_labels() {
                if !self.is_legal(from, to) {
                    edges.push((from, to));
                }
            }
        }
        edges
    }
}

/// A record of one applied transition, kept for audit/event purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition<L: StateLabel> {
    /// The label before the transition.
    pub from: L,
    /// The label after the transition.
    pub to: L,
    /// Epoch-millisecond timestamp of the transition (wall clock is fine
    /// here; this is audit metadata, not scheduler determinism).
    pub at_ms: i64,
}

impl<L: StateLabel> Transition<L> {
    /// Records a transition at `at_ms` without validating it.
    pub fn record(from: L, to: L, at_ms: i64) -> Self {
        Self { from, to, at_ms }
    }
}

/// Result of attempting to build a transition record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResult<L: StateLabel> {
    /// Recorded; the machine is now in label `to`.
    Applied(Transition<L>),
    /// The edge is not defined by the table.
    Illegal(Transition<L>),
}

/// Applies a transition through `table`, returning the record or the illegal
/// attempt. This is the single choke-point used by run supervision.
pub fn apply<L: StateLabel>(
    table: &TransitionTable<L>,
    from: L,
    to: L,
    at_ms: i64,
) -> ApplyResult<L> {
    let attempt = Transition::record(from, to, at_ms);
    if table.is_legal(from, to) {
        ApplyResult::Applied(attempt)
    } else {
        ApplyResult::Illegal(attempt)
    }
}

/// Number of labels (states) in a machine — surfaced by the `/debug` endpoint.
pub fn state_count<L: StateLabel>(table: &TransitionTable<L>) -> usize {
    table.len_labels()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Light {
        On,
        Off,
    }

    impl StateLabel for Light {
        fn is_terminal(self) -> bool {
            false
        }
    }

    fn legal_light(from: Light, to: Light) -> bool {
        from != to
    }

    fn all_light() -> &'static [Light] {
        &[Light::On, Light::Off]
    }

    #[test]
    fn table_enumerates_edges() {
        let t = TransitionTable::new(legal_light, all_light);
        let legal = t.legal_edges();
        assert_eq!(legal.len(), 2); // On->Off, Off->On
        let illegal = t.illegal_edges();
        assert_eq!(illegal.len(), 2); // On->On, Off->Off
        assert!(t.is_legal(Light::On, Light::Off));
    }

    #[test]
    fn apply_records_or_rejects() {
        let t = TransitionTable::new(legal_light, all_light);
        assert!(matches!(
            apply(&t, Light::On, Light::Off, 5),
            ApplyResult::Applied(Transition {
                from: Light::On,
                to: Light::Off,
                at_ms: 5
            })
        ));
        assert!(matches!(
            apply(&t, Light::On, Light::On, 6),
            ApplyResult::Illegal(_)
        ));
    }

    #[test]
    fn allowed_targets_excludes_self() {
        let t = TransitionTable::new(legal_light, all_light);
        assert_eq!(t.allowed_targets(Light::On), vec![Light::Off]);
    }
}
