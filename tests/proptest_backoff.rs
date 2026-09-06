//! Property-based tests for backoff and retry arithmetic.

use proptest::prelude::*;
use runvane::domain::retry_policy::{BackoffKind, JitterKind, RetryPolicy, MAX_BACKOFF_MS};
use runvane::retry::backoff::{should_retry, Backoff};

proptest! {
    #[test]
    fn backoff_raw_delay_is_bounded_and_monotonic(
        base in 1u64..100_000,
        max_mult in 1u64..1_000,
        multiplier in 1.0f64..10.0f64,
        seed in any::<u64>(),
    ) {
        let max = (base.saturating_mul(max_mult)).min(MAX_BACKOFF_MS);
        for kind in [BackoffKind::Fixed, BackoffKind::Linear, BackoffKind::Exponential] {
            let policy = RetryPolicy {
                max_attempts: 50,
                base_delay_ms: base,
                max_delay_ms: max,
                multiplier,
                backoff: kind,
                jitter: JitterKind::None,
                retryable_only: false,
            };
            let b = Backoff::new(&policy, seed);
            let mut prev = 0u64;
            for i in 0..20 {
                let delay = b.raw_delay(i);
                prop_assert!(delay >= base, "delay {} < base {}", delay, base);
                prop_assert!(delay <= max, "delay {} > max {}", delay, max);
                if i > 0 && kind != BackoffKind::Fixed {
                    prop_assert!(delay >= prev, "non-monotonic: delay {} < prev {}", delay, prev);
                }
                prev = delay;
            }
        }
    }

    #[test]
    fn backoff_jitter_always_stays_in_range(
        base in 1u64..10_000,
        max in 10_000u64..1_000_000,
        multiplier in 1.1f64..3.0f64,
        seed in any::<u64>(),
    ) {
        for jitter in [JitterKind::None, JitterKind::Full, JitterKind::Equal] {
            let policy = RetryPolicy {
                max_attempts: 20,
                base_delay_ms: base,
                max_delay_ms: max,
                multiplier,
                backoff: BackoffKind::Exponential,
                jitter,
                retryable_only: false,
            };
            let mut b = Backoff::new(&policy, seed);
            for i in 0..15 {
                let raw = b.raw_delay(i);
                let actual = b.next(i);
                match jitter {
                    JitterKind::None => prop_assert_eq!(actual, raw),
                    JitterKind::Full => {
                        prop_assert!(actual <= raw.max(1), "actual {} > raw {}", actual, raw);
                    }
                    JitterKind::Equal => {
                        let half = raw / 2;
                        prop_assert!(actual >= half, "actual {} < half {}", actual, half);
                        prop_assert!(actual <= raw.max(1) + 1, "actual {} > raw {}", actual, raw);
                    }
                }
            }
        }
    }

    #[test]
    fn should_retry_obeys_budget_and_retryability(
        max_attempts in 1u32..50,
        attempts in 0u32..60,
        failure_retryable in any::<bool>(),
        retryable_only in any::<bool>(),
    ) {
        let policy = RetryPolicy {
            max_attempts,
            base_delay_ms: 100,
            max_delay_ms: 1000,
            multiplier: 2.0,
            backoff: BackoffKind::Fixed,
            jitter: JitterKind::None,
            retryable_only,
        };

        let result = should_retry(&policy, attempts, failure_retryable);
        if attempts >= max_attempts {
            prop_assert!(!result, "should not retry after budget exhausted");
        } else if retryable_only && !failure_retryable {
            prop_assert!(!result, "should not retry non-retryable failure when retryable_only is true");
        } else {
            prop_assert!(result, "should retry when within budget and allowed");
        }
    }
}
