//! Deterministic backoff arithmetic over [`RetryPolicy`].
//!
//! The delay curve is computed exactly (fixed / linear / exponential, capped
//! by `max_delay_ms`), then the configured jitter is applied. The RNG is an
//! explicit, seedable `ChaCha8Rng` — never the process-global generator — so
//! scheduling behavior is reproducible in tests and in the replay dashboard.

use rand::Rng;
use rand_chacha::ChaCha8Rng;

use rand::SeedableRng;

use crate::domain::retry_policy::{BackoffKind, JitterKind, RetryPolicy};

/// A seeded backoff generator bound to one policy.
pub struct Backoff {
    policy: RetryPolicy,
    rng: ChaCha8Rng,
}

impl Backoff {
    /// Creates a backoff generator for `policy`, seeded deterministically.
    pub fn new(policy: &RetryPolicy, seed: u64) -> Self {
        Self {
            policy: policy.clone(),
            rng: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// The deterministic delay for `attempt_index` (0 = first retry, i.e. the
    /// delay after the first failed attempt), before jitter.
    pub fn raw_delay(&self, attempt_index: u32) -> u64 {
        let base = self.policy.base_delay_ms;
        let max = self
            .policy
            .max_delay_ms
            .min(crate::domain::retry_policy::MAX_BACKOFF_MS);
        let raw = match self.policy.backoff {
            BackoffKind::Fixed => base,
            BackoffKind::Linear => {
                let growth = self.policy.multiplier;
                base.saturating_add((base as f64 * (growth - 1.0) * attempt_index as f64) as u64)
            }
            BackoffKind::Exponential => {
                let growth = self.policy.multiplier;
                (base as f64 * growth.powf(attempt_index as f64)) as u64
            }
        };
        raw.clamp(base, max)
    }

    /// Full delay for `attempt_index`, jittered per the policy.
    pub fn next(&mut self, attempt_index: u32) -> u64 {
        let raw = self.raw_delay(attempt_index);
        match self.policy.jitter {
            JitterKind::None => raw,
            JitterKind::Full => {
                // random(0, raw]: spread retry storms while staying bounded.
                self.rng.gen_range(0..=raw.max(1))
            }
            JitterKind::Equal => {
                let half = raw / 2;
                half + self.rng.gen_range(0..=half.max(1))
            }
        }
    }

    /// The policy this generator is bound to.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }
}

/// Whether another attempt is permitted for a task/run with `attempts`
/// consumed, given the failure's retryability.
pub fn should_retry(policy: &RetryPolicy, attempts: u32, failure_retryable: bool) -> bool {
    policy.allows_attempt(attempts) && (!policy.retryable_only || failure_retryable)
}

/// Computes the next dispatch timestamp (epoch ms) for a failed attempt, or
/// `None` when retries are exhausted.
pub fn next_attempt_at_ms(
    policy: &RetryPolicy,
    attempts: u32,
    failure_retryable: bool,
    now_ms: i64,
    seed: u64,
) -> Option<i64> {
    if !should_retry(policy, attempts, failure_retryable) {
        return None;
    }
    let mut backoff = Backoff::new(policy, seed);
    // attempt_index is the number of completed attempts so far.
    let delay = backoff.next(attempts.saturating_sub(1));
    Some(now_ms.saturating_add(delay as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 42;

    #[test]
    // [P2P] RV-016 witness (deadline/backoff math stays bounded, no overflow).
    fn fixed_without_jitter_is_constant() {
        let p = RetryPolicy::fixed(5, 2_000);
        let mut b = Backoff::new(&p, SEED);
        for i in 0..5 {
            assert_eq!(b.next(i), 2_000);
            assert_eq!(b.raw_delay(i), 2_000);
        }
    }

    #[test]
    fn exponential_raw_grows_and_caps() {
        let p = RetryPolicy {
            max_attempts: 10,
            base_delay_ms: 1_000,
            max_delay_ms: 4_000,
            multiplier: 2.0,
            backoff: BackoffKind::Exponential,
            jitter: JitterKind::None,
            retryable_only: false,
        };
        let b = Backoff::new(&p, SEED);
        assert_eq!(b.raw_delay(0), 1_000);
        assert_eq!(b.raw_delay(1), 2_000);
        assert_eq!(b.raw_delay(2), 4_000);
        assert_eq!(b.raw_delay(3), 4_000); // capped
    }

    #[test]
    fn linear_adds_per_attempt() {
        let p = RetryPolicy {
            max_attempts: 10,
            base_delay_ms: 1_000,
            max_delay_ms: 10_000,
            multiplier: 1.5,
            backoff: BackoffKind::Linear,
            jitter: JitterKind::None,
            retryable_only: false,
        };
        let b = Backoff::new(&p, SEED);
        assert_eq!(b.raw_delay(0), 1_000);
        assert_eq!(b.raw_delay(1), 1_500);
        assert_eq!(b.raw_delay(2), 2_000);
    }

    #[test]
    // [P2P] RV-015 witness (retry boundary: delays stay in bounds at the edges).
    fn full_jitter_stays_in_bounds() {
        let p = RetryPolicy::exponential(10, 1_000, 60_000);
        let mut b = Backoff::new(&p, SEED);
        for i in 0..20 {
            let d = b.next(i);
            let raw = b.raw_delay(i);
            assert!(d <= raw, "jitter exceeded raw delay");
        }
    }

    #[test]
    fn jitter_is_seed_deterministic() {
        let p = RetryPolicy::exponential(10, 1_000, 60_000);
        let mut a = Backoff::new(&p, 7);
        let mut b = Backoff::new(&p, 7);
        let seq_a: Vec<u64> = (0..10).map(|i| a.next(i)).collect();
        let seq_b: Vec<u64> = (0..10).map(|i| b.next(i)).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn should_retry_respects_attempt_budget() {
        let p = RetryPolicy::fixed(3, 100);
        assert!(should_retry(&p, 0, true));
        assert!(should_retry(&p, 2, true));
        assert!(!should_retry(&p, 3, true));
        // retryable_only: non-retryable failure is dropped.
        let strict = RetryPolicy {
            retryable_only: true,
            ..p
        };
        assert!(!should_retry(&strict, 0, false));
        assert!(should_retry(&strict, 0, true));
    }

    #[test]
    fn next_attempt_at_ms_exhaustion() {
        let p = RetryPolicy::fixed(1, 1_000);
        // attempts == max_attempts -> exhausted.
        assert_eq!(next_attempt_at_ms(&p, 1, true, 5_000, SEED), None);
        // Room remains -> schedule at now + delay.
        let p2 = RetryPolicy::fixed(2, 1_000);
        let next = next_attempt_at_ms(&p2, 1, true, 5_000, SEED).unwrap();
        assert_eq!(next, 6_000);
    }
}
