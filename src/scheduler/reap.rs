//! Maintenance pass for the scheduler: reclaiming expired queue leases.
//!
//! A worker that dies mid-execution leaves its run's queue entry leased up to
//! `lease_ms`. The reaper un-leases those entries so the next dispatch round
//! re-claims and re-runs them. It deliberately does NOT touch entries that are
//! still inside their lease window, so in-flight work is never double-run.

use crate::clock::Clock;
use crate::persistence::Store;

/// Outcome of one reap round.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReapStats {
    /// Entries recovered (un-leased) by this round.
    pub recovered: usize,
}

/// Un-leases every queue entry whose lease has lapsed at `now`.
pub fn reap_expired_leases(store: &dyn Store, clock: &dyn Clock) -> ReapStats {
    ReapStats {
        recovered: store
            .recover_expired_leases(clock.now_ms())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::persistence::memory::MemoryStore;
    use crate::persistence::{ClaimToken, QueueEntry};
    use crate::domain::ids::RunId;

    fn entry(run_id: &str, due_at_ms: i64, lease_until_ms: Option<i64>) -> QueueEntry {
        QueueEntry {
            run_id: RunId::from_validated(run_id.to_owned()),
            token: ClaimToken::empty(),
            due_at_ms,
            lease_until_ms,
            claimed_by: None,
        }
    }

    #[test]
    fn expired_leases_are_recovered_only_after_the_window() {
        let store = MemoryStore::default();
        let clock = ManualClock::at(10_000);
        store
            .enqueue(entry("rn_reap0000001", 10_000, Some(10_500)))
            .unwrap();
        store
            .enqueue(entry("rn_reap0000002", 10_000, Some(10_200)))
            .unwrap();
        store
            .enqueue(entry("rn_reap0000003", 10_000, None))
            .unwrap();

        // Lease 1 expires at 10_500, lease 2 expires at 10_200: at 10_300 only
        // entry 2 is recoverable.
        clock.set(10_300);
        let stats = reap_expired_leases(&store, &clock);
        assert_eq!(stats.recovered, 1);

        // Reaching 10_500 recovers the remaining leased entry; the never-leased
        // entry is untouched (recover only ever un-leases).
        clock.set(10_500);
        let stats = reap_expired_leases(&store, &clock);
        assert_eq!(stats.recovered, 1);

        assert_eq!(store.len_queue(), 3);
    }
}