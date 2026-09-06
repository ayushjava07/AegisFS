//! Multi-tenant concurrency throttling, leaky-bucket rate limiting, and fair-share queueing.
//!
//! In shared multi-tenant deployments, runaway workflows or traffic bursts from one tenant
//! can exhaust database connections or starve worker threads. This module provides:
//!
//! * [`ConcurrencyLimiter`] with RAII [`ConcurrencyPermit`] guards for in-flight bounds;
//! * [`LeakyBucket`] rate limiters for sustained request pacing with burst tolerance;
//! * [`FairShareDispatcher`] for interleaved round-robin task dispatching across tenants.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

/// Errors arising during concurrency acquisition or rate limiting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThrottleError {
    /// The maximum in-flight concurrency limit was reached for this key.
    #[error("concurrency limit of {limit} reached for '{key}' (active: {active})")]
    ConcurrencyLimitExceeded {
        /// Key or tenant identity.
        key: String,
        /// Current in-flight count.
        active: usize,
        /// Configured limit.
        limit: usize,
    },

    /// The rate limiter rate was exceeded and cannot accept more operations.
    #[error("rate limit exceeded for '{key}' (retry after {retry_after_ms}ms)")]
    RateLimitExceeded {
        /// Key or tenant identity.
        key: String,
        /// Milliseconds until next token becomes available.
        retry_after_ms: u64,
    },
}

/// An RAII permit holding an in-flight concurrency slot.
///
/// When dropped, the permit automatically decrements the active concurrency counter.
#[derive(Debug)]
pub struct ConcurrencyPermit {
    key: String,
    counter: Arc<AtomicUsize>,
}

impl ConcurrencyPermit {
    /// Returns the key or tenant bound to this permit.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Thread-safe multi-tenant concurrency governor.
#[derive(Debug, Default)]
pub struct ConcurrencyLimiter {
    limits: Mutex<HashMap<String, usize>>,
    counters: Mutex<HashMap<String, Arc<AtomicUsize>>>,
    default_limit: usize,
}

impl ConcurrencyLimiter {
    /// Creates a limiter with a default fallback limit for unspecified keys.
    pub fn new(default_limit: usize) -> Self {
        Self {
            limits: Mutex::new(HashMap::new()),
            counters: Mutex::new(HashMap::new()),
            default_limit: default_limit.max(1),
        }
    }

    /// Sets an explicit maximum concurrency limit for a specific tenant or key.
    pub fn set_limit(&self, key: impl Into<String>, limit: usize) {
        self.limits.lock().insert(key.into(), limit.max(1));
    }

    /// Attempts to acquire an in-flight execution permit for `key`.
    pub fn try_acquire(&self, key: &str) -> Result<ConcurrencyPermit, ThrottleError> {
        let limit = self
            .limits
            .lock()
            .get(key)
            .copied()
            .unwrap_or(self.default_limit);

        let counter = {
            let mut counters = self.counters.lock();
            counters
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
                .clone()
        };

        // CAS loop to safely increment without exceeding limit
        let mut current = counter.load(Ordering::SeqCst);
        loop {
            if current >= limit {
                return Err(ThrottleError::ConcurrencyLimitExceeded {
                    key: key.to_owned(),
                    active: current,
                    limit,
                });
            }
            match counter.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    return Ok(ConcurrencyPermit {
                        key: key.to_owned(),
                        counter,
                    });
                }
                Err(actual) => current = actual,
            }
        }
    }

    /// Returns the current active concurrency count for `key`.
    pub fn active_count(&self, key: &str) -> usize {
        self.counters
            .lock()
            .get(key)
            .map_or(0, |c| c.load(Ordering::Relaxed))
    }
}

/// Leaky-bucket rate limiter for smooth request pacing with burst allowance.
#[derive(Debug)]
pub struct LeakyBucket {
    capacity: f64,
    leak_rate_per_sec: f64,
    state: Mutex<BucketState>,
}

#[derive(Debug)]
struct BucketState {
    level: f64,
    last_update_ms: i64,
}

impl LeakyBucket {
    /// Creates a new leaky bucket with maximum `burst_capacity` and steady-state `leak_rate_per_sec`.
    pub fn new(burst_capacity: f64, leak_rate_per_sec: f64, now_ms: i64) -> Self {
        Self {
            capacity: burst_capacity.max(1.0),
            leak_rate_per_sec: leak_rate_per_sec.max(0.001),
            state: Mutex::new(BucketState {
                level: 0.0,
                last_update_ms: now_ms,
            }),
        }
    }

    /// Attempts to add `cost` tokens to the bucket at `now_ms`.
    pub fn try_acquire(&self, cost: f64, now_ms: i64) -> Result<(), u64> {
        let mut state = self.state.lock();

        // Compute leaked amount since last update
        let elapsed_ms = (now_ms - state.last_update_ms).max(0);
        let leaked = (elapsed_ms as f64 / 1000.0) * self.leak_rate_per_sec;
        state.level = (state.level - leaked).max(0.0);
        state.last_update_ms = now_ms;

        if state.level + cost <= self.capacity {
            state.level += cost;
            Ok(())
        } else {
            let excess = (state.level + cost) - self.capacity;
            let wait_secs = excess / self.leak_rate_per_sec;
            let wait_ms = (wait_secs * 1000.0).ceil() as u64;
            Err(wait_ms.max(1))
        }
    }
}

/// Fair-share dispatcher that interleaves queue items across distinct tenants to prevent starvation.
#[derive(Debug, Default)]
pub struct FairShareDispatcher<T> {
    queues: BTreeMap<String, VecDeque<T>>,
    tenant_order: VecDeque<String>,
}

impl<T> FairShareDispatcher<T> {
    /// Creates an empty fair-share dispatcher.
    pub fn new() -> Self {
        Self {
            queues: BTreeMap::new(),
            tenant_order: VecDeque::new(),
        }
    }

    /// Enqueues an item under the specified tenant.
    pub fn push(&mut self, tenant: impl Into<String>, item: T) {
        let tenant_str = tenant.into();
        if !self.queues.contains_key(&tenant_str) {
            self.tenant_order.push_back(tenant_str.clone());
        }
        self.queues.entry(tenant_str).or_default().push_back(item);
    }

    /// Pops the next item using round-robin rotation across active tenants.
    pub fn pop(&mut self) -> Option<(String, T)> {
        for _ in 0..self.tenant_order.len() {
            if let Some(tenant) = self.tenant_order.pop_front() {
                if let Some(q) = self.queues.get_mut(&tenant) {
                    if let Some(item) = q.pop_front() {
                        // Put tenant at back if it still has items
                        if !q.is_empty() {
                            self.tenant_order.push_back(tenant.clone());
                        } else {
                            self.queues.remove(&tenant);
                        }
                        return Some((tenant, item));
                    }
                }
            }
        }
        None
    }

    /// Total number of queued items across all tenants.
    pub fn len(&self) -> usize {
        self.queues.values().map(VecDeque::len).sum()
    }

    /// Whether all tenant queues are empty.
    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrency_limiter_enforces_limits_and_releases_via_permit() {
        let limiter = ConcurrencyLimiter::new(2);
        limiter.set_limit("acme", 2);

        let permit1 = limiter.try_acquire("acme").expect("permit 1");
        assert_eq!(limiter.active_count("acme"), 1);

        let permit2 = limiter.try_acquire("acme").expect("permit 2");
        assert_eq!(limiter.active_count("acme"), 2);

        // 3rd should fail
        let err = limiter.try_acquire("acme").unwrap_err();
        assert!(matches!(
            err,
            ThrottleError::ConcurrencyLimitExceeded { limit: 2, .. }
        ));

        // Release permit 1
        drop(permit1);
        assert_eq!(limiter.active_count("acme"), 1);

        // Now acquire succeeds
        let _permit3 = limiter.try_acquire("acme").expect("permit 3");
        assert_eq!(limiter.active_count("acme"), 2);

        drop(permit2);
    }

    #[test]
    fn leaky_bucket_rate_limiter_burst_and_leak() {
        let mut now = 1000;
        let bucket = LeakyBucket::new(2.0, 1.0, now);

        // First 2 tokens succeed within burst capacity
        assert!(bucket.try_acquire(1.0, now).is_ok());
        assert!(bucket.try_acquire(1.0, now).is_ok());

        // 3rd token exceeds capacity
        let wait = bucket.try_acquire(1.0, now).unwrap_err();
        assert!(wait > 0);

        // Advance time by 1 second (1000ms) -> leaks 1 token
        now += 1000;
        assert!(bucket.try_acquire(1.0, now).is_ok());
    }

    #[test]
    fn fair_share_dispatcher_interleaves_tenants() {
        let mut dispatcher = FairShareDispatcher::new();
        dispatcher.push("tenant_a", "job_a1");
        dispatcher.push("tenant_a", "job_a2");
        dispatcher.push("tenant_a", "job_a3");
        dispatcher.push("tenant_b", "job_b1");
        dispatcher.push("tenant_c", "job_c1");

        assert_eq!(dispatcher.len(), 5);

        // Round robin: tenant_a, tenant_b, tenant_c, tenant_a, tenant_a
        assert_eq!(dispatcher.pop(), Some(("tenant_a".into(), "job_a1")));
        assert_eq!(dispatcher.pop(), Some(("tenant_b".into(), "job_b1")));
        assert_eq!(dispatcher.pop(), Some(("tenant_c".into(), "job_c1")));
        assert_eq!(dispatcher.pop(), Some(("tenant_a".into(), "job_a2")));
        assert_eq!(dispatcher.pop(), Some(("tenant_a".into(), "job_a3")));
        assert_eq!(dispatcher.pop(), None);
        assert!(dispatcher.is_empty());
    }
}
