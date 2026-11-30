//! Retry and backoff policy configuration.
//!
//! A `RetryPolicy` describes *how many* attempts a run or task gets and *how
//! long* to wait between them. The arithmetic that turns a policy into actual
//! delays lives in [`crate::retry`]; this module is configuration-only so the
//! policy can be serialized, validated, and stored independently of the
//! scheduling logic.

use serde::{Deserialize, Serialize};

/// Maximum number of attempts a policy may request.
pub const MAX_ATTEMPTS: u32 = 100;

/// Upper bound for a single backoff delay, in milliseconds.
pub const MAX_BACKOFF_MS: u64 = 86_400_000; // 24h

/// The shape of the delay curve between attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffKind {
    /// Always wait `base_delay`.
    Fixed,
    /// Add a fixed step (`length * multiplier`) per attempt.
    Linear,
    /// Multiply the previous delay by `multiplier` each attempt, capped at
    /// `max_delay`.
    Exponential,
}

/// How much randomness is mixed into the computed delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JitterKind {
    /// No jitter; all clients sharing a policy retry in lockstep.
    None,
    /// `random(0, delay)` — full jitter, spreads retry storms well.
    Full,
    /// `delay/2 + random(0, delay/2)` — equal jitter, bounded lower half.
    Equal,
}

/// The complete retry configuration for a run or task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Total attempts allowed (including the first). At least 1.
    pub max_attempts: u32,
    /// Base delay between attempts, in milliseconds.
    pub base_delay_ms: u64,
    /// Ceiling on any single delay, in milliseconds.
    pub max_delay_ms: u64,
    /// Growth step for `Linear` and `Exponential` backoff.
    pub multiplier: f64,
    /// The shape of the backoff curve.
    pub backoff: BackoffKind,
    /// Jitter applied on top of the deterministic delay.
    pub jitter: JitterKind,
    /// When `true`, non-retryable failures are not retried at all.
    pub retryable_only: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 1_000,
            max_delay_ms: 60_000,
            multiplier: 2.0,
            backoff: BackoffKind::Exponential,
            jitter: JitterKind::Full,
            retryable_only: false,
        }
    }
}

/// Problems discovered while validating a policy.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PolicyError {
    /// Attempts must be a positive integer within `MAX_ATTEMPTS`.
    #[error("max_attempts must be within 1..={MAX_ATTEMPTS}, got {0}")]
    BadAttempts(u32),
    /// `base_delay` must be positive.
    #[error("base_delay_ms must be positive, got {0}")]
    ZeroBaseDelay(u64),
    /// `max_delay` must be at least `base_delay`.
    #[error("max_delay_ms ({max}) must be >= base_delay_ms ({base})")]
    MaxBelowBase {
        /// Configured maximum delay.
        max: u64,
        /// Configured base delay.
        base: u64,
    },
    /// The multiplier must be finite and >= 1.0 for growth curves.
    #[error("multiplier must be finite and >= 1.0, got {0}")]
    BadMultiplier(f64),
    /// `max_delay` exceeds the platform ceiling.
    #[error("max_delay_ms {0} exceeds ceiling {MAX_BACKOFF_MS}")]
    DelayOverCeiling(u64),
}

impl RetryPolicy {
    /// Returns a fixed-delay, no-jitter policy with `attempts` attempts.
    pub fn fixed(attempts: u32, delay_ms: u64) -> Self {
        Self {
            max_attempts: attempts,
            base_delay_ms: delay_ms,
            max_delay_ms: delay_ms,
            multiplier: 1.0,
            backoff: BackoffKind::Fixed,
            jitter: JitterKind::None,
            retryable_only: false,
        }
    }

    /// Returns an exponential policy with full jitter — the production
    /// default shape.
    pub fn exponential(max_attempts: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            max_attempts,
            base_delay_ms,
            max_delay_ms,
            multiplier: 2.0,
            backoff: BackoffKind::Exponential,
            jitter: JitterKind::Full,
            retryable_only: false,
        }
    }

    /// Validates every field, returning the first problem found.
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.max_attempts == 0 || self.max_attempts > MAX_ATTEMPTS {
            return Err(PolicyError::BadAttempts(self.max_attempts));
        }
        if self.base_delay_ms == 0 {
            return Err(PolicyError::ZeroBaseDelay(self.base_delay_ms));
        }
        if self.max_delay_ms < self.base_delay_ms {
            return Err(PolicyError::MaxBelowBase {
                max: self.max_delay_ms,
                base: self.base_delay_ms,
            });
        }
        if self.max_delay_ms > MAX_BACKOFF_MS {
            return Err(PolicyError::DelayOverCeiling(self.max_delay_ms));
        }
        if !self.multiplier.is_finite() || self.multiplier < 1.0 {
            return Err(PolicyError::BadMultiplier(self.multiplier));
        }
        Ok(())
    }

    /// Whether the policy permits another attempt given the number already
    /// consumed. An attempt counter of `0` means "about to make the first go".
    pub fn allows_attempt(&self, attempts_consumed: u32) -> bool {
        attempts_consumed < self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RetryPolicy {
        RetryPolicy::exponential(3, 1_000, 30_000)
    }

    #[test]
    fn default_policy_validates() {
        assert!(RetryPolicy::default().validate().is_ok());
    }

    #[test]
    fn exponential_sample_validates() {
        assert_eq!(sample().backoff, BackoffKind::Exponential);
        assert_eq!(sample().jitter, JitterKind::Full);
        assert!(sample().validate().is_ok());
    }

    #[test]
    fn zero_attempts_rejected() {
        let mut p = sample();
        p.max_attempts = 0;
        assert_eq!(p.validate(), Err(PolicyError::BadAttempts(0)));
    }

    #[test]
    fn over_ceiling_attempts_rejected() {
        let mut p = sample();
        p.max_attempts = MAX_ATTEMPTS + 1;
        assert_eq!(
            p.validate(),
            Err(PolicyError::BadAttempts(MAX_ATTEMPTS + 1))
        );
    }

    #[test]
    fn zero_base_delay_rejected() {
        let mut p = sample();
        p.base_delay_ms = 0;
        assert_eq!(p.validate(), Err(PolicyError::ZeroBaseDelay(0)));
    }

    #[test]
    fn max_below_base_rejected() {
        let mut p = sample();
        p.max_delay_ms = 100;
        p.base_delay_ms = 500;
        assert_eq!(
            p.validate(),
            Err(PolicyError::MaxBelowBase {
                max: 100,
                base: 500
            })
        );
    }

    #[test]
    fn bad_multiplier_rejected() {
        for bad in [0.5, f64::NAN, f64::INFINITY] {
            let mut p = sample();
            p.multiplier = bad;
            assert!(p.validate().is_err());
        }
    }

    #[test]
    fn delay_over_ceiling_rejected() {
        let mut p = sample();
        p.max_delay_ms = MAX_BACKOFF_MS + 1;
        assert_eq!(
            p.validate(),
            Err(PolicyError::DelayOverCeiling(MAX_BACKOFF_MS + 1))
        );
    }

    #[test]
    fn fixed_policy_shape() {
        let p = RetryPolicy::fixed(5, 2_000);
        assert_eq!(p.backoff, BackoffKind::Fixed);
        assert_eq!(p.jitter, JitterKind::None);
        assert_eq!(p.multiplier, 1.0);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn allows_attempt_semantics() {
        let p = RetryPolicy::fixed(3, 1_000);
        assert!(p.allows_attempt(0));
        assert!(p.allows_attempt(1));
        assert!(p.allows_attempt(2));
        assert!(!p.allows_attempt(3));
        assert!(!p.allows_attempt(4));
    }

    #[test]
    fn serde_round_trip_keeps_shape() {
        let p = sample();
        let json = serde_json::to_string(&p).unwrap();
        let back: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
