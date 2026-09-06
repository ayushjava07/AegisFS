//! Retry and backoff plumbing for runs and tasks.
//!
//! [`backoff`] computes the actual delay between attempts from a
//! [`RetryPolicy`]; [`RetryPlanner`] turns a failed execution into concrete
//! scheduling state (retry now vs. schedule later vs. give up) that the
//! scheduler and the run record both consume.

pub mod backoff;

use crate::clock::Clock;
use crate::domain::retry_policy::RetryPolicy;
use crate::domain::status::FailureKind;

use backoff::next_attempt_at_ms;

/// The outcome of planning a retry for one failed attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryPlan {
    /// Retries are exhausted; the entity must transit to a terminal failure.
    GiveUp,
    /// Retry immediately (delay 0): used by in-process handler `Runner` mode.
    RetryNow,
    /// Re-schedule at this epoch-ms timestamp.
    RetryAt(i64),
}

/// Decides what to do after a failed attempt using the policy and clock.
pub struct RetryPlanner<'a> {
    policy: &'a RetryPolicy,
    clock: &'a dyn Clock,
    seed: u64,
}

impl<'a> RetryPlanner<'a> {
    /// A planner bound to a policy, clock, and a deterministic seed.
    pub fn new(policy: &'a RetryPolicy, clock: &'a dyn Clock, seed: u64) -> Self {
        Self {
            policy,
            clock,
            seed,
        }
    }

    /// Plans the next action after `attempts` attempts have `kind` failure.
    pub fn plan(&self, attempts: u32, kind: &FailureKind) -> RetryPlan {
        let retryable = kind.retryable();
        let now = self.clock.now_ms();
        match next_attempt_at_ms(self.policy, attempts, retryable, now, self.seed) {
            None => RetryPlan::GiveUp,
            Some(t) if t <= now => RetryPlan::RetryNow,
            Some(t) => RetryPlan::RetryAt(t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::domain::retry_policy::RetryPolicy;
    use crate::domain::status::FailureKind;

    fn clock_at(ms: i64) -> ManualClock {
        ManualClock::at(ms)
    }

    #[test]
    // [P2P] RV-015 witness (retry-policy boundary; passes on broken and fixed).
    fn exhausted_attempts_give_up() {
        let p = RetryPolicy::fixed(1, 1_000);
        let clock = clock_at(5_000);
        let planner = RetryPlanner::new(&p, &clock, 1);
        assert_eq!(
            planner.plan(1, &FailureKind::TransientFailure),
            RetryPlan::GiveUp
        );
    }

    #[test]
    fn retryable_failure_schedules_in_future() {
        let p = RetryPolicy::fixed(3, 1_000);
        let clock = clock_at(5_000);
        let planner = RetryPlanner::new(&p, &clock, 1);
        assert_eq!(
            planner.plan(1, &FailureKind::TransientFailure),
            RetryPlan::RetryAt(6_000)
        );
    }

    #[test]
    fn non_retryable_failure_gives_up_when_strict() {
        let p = RetryPolicy {
            retryable_only: true,
            ..RetryPolicy::fixed(3, 1_000)
        };
        let clock = clock_at(5_000);
        let planner = RetryPlanner::new(&p, &clock, 1);
        assert_eq!(planner.plan(1, &FailureKind::Rejected), RetryPlan::GiveUp);
        // Retryable failures still get scheduled.
        assert_eq!(
            planner.plan(1, &FailureKind::TransientFailure),
            RetryPlan::RetryAt(6_000)
        );
    }
}
