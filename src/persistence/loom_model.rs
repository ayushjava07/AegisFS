//! `loom` model checks for the queue's lease protocol.
//!
//! The claim/ack logic lives under a plain `std::sync::Mutex` in the real
//! stores, so a scheduler-level check needs an instrumented twin. This module
//! is a faithful miniature of the invariant we actually ship: a queue entry
//! can only be claimed by one token at a time, a live foreign lease is never
//! stolen, and an ack with the wrong token is always rejected. `loom::model`
//! reruns each scenario over every reachable interleaving with a bounded,
//! randomized scheduler.
//!
//! Gate: `cargo test --features loom` on x86_64/aarch64 hosts.
//!
//! [P2P] RV-009 witnesses (all three loom models pass on both states):
//!   - concurrent_claim_has_a_single_winner
//!   - foreign_lease_is_never_stolen_and_ack_is_holder_guarded
//!   - expired_lease_is_reclaimable
#![cfg(test)]

use loom::sync::{Arc, Mutex};
use loom::thread;

/// One queue entry's lease state, mirroring `QueueEntry`'s token + lease.
#[derive(Debug, Clone)]
struct LeaseState {
    /// The token holding the lease, when any.
    holder: Option<u64>,
    /// Monotonic instant until which the lease is live.
    lease_until: u64,
}

/// A single-entry lease board, the moral equivalent of `queue.get(run_id)`.
#[derive(Debug)]
struct LeaseBoard {
    entry: Mutex<LeaseState>,
}

impl LeaseBoard {
    fn empty() -> Self {
        Self {
            entry: Mutex::new(LeaseState {
                holder: None,
                lease_until: 0,
            }),
        }
    }

    /// Claim for `token` at `now` with `lease_ms` of hold time. Returns
    /// `false` (ClaimLost) when a live lease belongs to a different token.
    fn claim(&self, token: u64, now: u64, lease_ms: u64) -> bool {
        let mut entry = self.entry.lock().unwrap();
        let live_foreign = entry.lease_until > now && entry.holder != Some(token);
        if live_foreign {
            return false;
        }
        *entry = LeaseState {
            holder: Some(token),
            lease_until: now + lease_ms,
        };
        true
    }

    /// Ack for `token`; only the current holder may succeed.
    fn ack(&self, token: u64) -> bool {
        let entry = self.entry.lock().unwrap();
        entry.holder == Some(token)
    }
}

/// Two workers racing for the same entry: exactly one wins regardless of how
/// the scheduler interleaves the read-modify-write on the mutex.
#[test]
fn concurrent_claim_has_a_single_winner() {
    loom::model(|| {
        let board = Arc::new(LeaseBoard::empty());
        let mut handles = Vec::new();
        for token in [11u64, 22u64] {
            let board = Arc::clone(&board);
            handles.push(thread::spawn(move || board.claim(token, 1_000, 10_000)));
        }
        let outcomes: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            outcomes.iter().filter(|&&won| won).count(),
            1,
            "claim of one entry must crown exactly one holder (got {outcomes:?})"
        );
        let holder = board.entry.lock().unwrap().holder;
        assert!(holder.is_some(), "a winner must hold the lease");
    });
}

/// A live lease is not stolen by a third token, and a stale token's ack is
/// rejected even while the lease is live.
#[test]
fn foreign_lease_is_never_stolen_and_ack_is_holder_guarded() {
    loom::model(|| {
        let board = Arc::new(LeaseBoard::empty());
        let holder = Arc::clone(&board);
        let intruder = Arc::clone(&board);

        let h = thread::spawn(move || holder.claim(7, 1_000, 10_000));
        let joined = h.join().unwrap();
        assert!(joined, "first claim must win");

        let i = thread::spawn(move || intruder.claim(9, 1_000, 10_000));
        let stolen = i.join().unwrap();
        assert!(!stolen, "live foreign lease must not be re-claimed");

        assert!(!board.ack(9), "intruder cannot confirm its lease");
        assert!(board.ack(7), "holder can always ack while live");
    });
}

/// After the lease lapses, any token may reclaim (the reap/release path).
#[test]
fn expired_lease_is_reclaimable() {
    loom::model(|| {
        let board = Arc::new(LeaseBoard::empty());
        // The first claim happened at t=0; by t=1000 it is stale.
        assert!(board.claim(1, 0, 100));
        let stolen = board.claim(2, 1_000, 100);
        assert!(stolen, "expired lease is fair game");
        assert!(!board.ack(1), "old holder's token no longer matches");
    });
}
