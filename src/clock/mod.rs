//! Time abstraction for deterministic scheduling and tests.
//!
//! The whole platform agrees to read "what time is it" through a [`Clock`]
//! instead of calling `SystemTime` directly. Production wiring injects the
//! [`SystemClock`]; tests inject a [`ManualClock`] they can advance
//! explicitly. This is what makes retry/backoff, queue readiness, run
//! deadlines, lease expiry, and cache TTLs assertable without wall-clock
//! sleeps.
//!
//! Times are represented as Unix-epoch **milliseconds in an `i64`**, which is
//! wide enough for any time within ±292 million years and keeps SQLite,
//! JSON, and logs on a single representation.

use std::sync::Mutex;

/// A source of "now" for the platform.
pub trait Clock: Send + Sync {
    /// Current Unix-epoch time in milliseconds.
    fn now_ms(&self) -> i64;
}

/// The production clock: wall-clock time via `SystemTime`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO);
        now.as_millis() as i64
    }
}

/// A clock whose value is controlled explicitly. Shared by reference and
/// mutated via [`ManualClock::set`]/[`ManualClock::advance`].
#[derive(Debug, Default)]
pub struct ManualClock {
    inner: Mutex<i64>,
}

impl ManualClock {
    /// Creates a clock frozen at `start_ms`.
    pub fn at(start_ms: i64) -> Self {
        Self {
            inner: Mutex::new(start_ms),
        }
    }

    /// Creates a clock frozen at the Unix epoch (millisecond zero).
    pub fn epoch() -> Self {
        Self::at(0)
    }

    /// Sets the clock to exactly `now_ms`.
    pub fn set(&self, now_ms: i64) {
        *self.inner.lock().unwrap() = now_ms;
    }

    /// Advances the clock by `delta_ms`. Rejects negative deltas.
    pub fn advance(&self, delta_ms: i64) {
        assert!(
            delta_ms >= 0,
            "ManualClock::advance requires a non-negative delta"
        );
        let mut guard = self.inner.lock().unwrap();
        *guard += delta_ms;
    }

    /// Current value (for tests that want to observe call patterns).
    pub fn value(&self) -> i64 {
        *self.inner.lock().unwrap()
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.value()
    }
}

/// A shorthand for "epoch milliseconds" used across the persistence and
/// scheduler layers so the intent of a field is readable.
pub type EpochMs = i64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_non_negative() {
        // Deterministic lower bound: the test run happens after the epoch.
        assert!(!SystemClock.now_ms().is_negative());
    }

    #[test]
    fn manual_clock_starts_at_configured_time() {
        let c = ManualClock::at(1_700_000_000_000);
        assert_eq!(c.now_ms(), 1_700_000_000_000);
        assert_eq!(c.value(), 1_700_000_000_000);
    }

    #[test]
    fn manual_clock_set_and_advance() {
        let c = ManualClock::epoch();
        c.set(1_000);
        assert_eq!(c.now_ms(), 1_000);
        c.advance(250);
        assert_eq!(c.now_ms(), 1_250);
        c.advance(0);
        assert_eq!(c.now_ms(), 1_250);
    }

    #[test]
    #[should_panic(expected = "non-negative")]
    fn negative_advance_panics() {
        let c = ManualClock::epoch();
        c.advance(-5);
    }

    #[test]
    fn manual_clock_is_shareable() {
        let c = ManualClock::epoch();
        std::thread::scope(|s| {
            let c = &c;
            let h = s.spawn(move || {
                c.advance(10);
                c.now_ms()
            });
            let now = h.join().unwrap();
            assert_eq!(now, 10);
        });
    }
}
