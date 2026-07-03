use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::core::error::AegisResult;

struct Inner {
    tokens: f64,
    last_refill: Instant,
    rate: f64,
    burst: f64,
}

pub struct Throttle {
    inner: Arc<Mutex<Inner>>,
}

impl Throttle {
    pub fn new(rate: u64, burst: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                tokens: burst as f64,
                last_refill: Instant::now(),
                rate: rate as f64,
                burst: burst as f64,
            })),
        }
    }

    fn refill(inner: &mut Inner) {
        let now = Instant::now();
        let elapsed = now.duration_since(inner.last_refill).as_secs_f64();
        inner.last_refill = now;
        inner.tokens = (inner.tokens + elapsed * inner.rate).min(inner.burst);
    }

    pub fn acquire(&self, tokens: u64) -> AegisResult<()> {
        let tokens_needed = tokens as f64;
        loop {
            let mut inner = self.inner.lock();
            Self::refill(&mut inner);
            if inner.tokens >= tokens_needed {
                inner.tokens -= tokens_needed;
                return Ok(());
            }
            let deficit = tokens_needed - inner.tokens;
            let wait_secs = deficit / inner.rate;
            drop(inner);
            if wait_secs > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(wait_secs));
            }
        }
    }

    pub fn try_acquire(&self, tokens: u64) -> AegisResult<bool> {
        let tokens_needed = tokens as f64;
        let mut inner = self.inner.lock();
        Self::refill(&mut inner);
        if inner.tokens >= tokens_needed {
            inner.tokens -= tokens_needed;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn rate(&self) -> u64 {
        self.inner.lock().rate as u64
    }

    pub fn available(&self) -> u64 {
        let mut inner = self.inner.lock();
        Self::refill(&mut inner);
        inner.tokens as u64
    }
}

impl Clone for Throttle {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_acquire_ok() {
        let t = Throttle::new(100, 10);
        assert!(t.try_acquire(5).unwrap());
    }

    #[test]
    fn test_try_acquire_exceeds_burst() {
        let t = Throttle::new(100, 10);
        assert!(t.try_acquire(10).unwrap());
        assert!(!t.try_acquire(1).unwrap());
    }

    #[test]
    fn test_available_tokens() {
        let t = Throttle::new(100, 20);
        assert_eq!(t.available(), 20);
        t.try_acquire(5).unwrap();
        assert_eq!(t.available(), 15);
    }

    #[test]
    fn test_acquire_blocking() {
        let t = Throttle::new(1000, 5);
        assert!(t.try_acquire(5).unwrap());
        let start = Instant::now();
        t.acquire(1).unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(1));
    }

    #[test]
    fn test_clone() {
        let t1 = Throttle::new(50, 10);
        let t2 = t1.clone();
        t1.try_acquire(3).unwrap();
        assert!(t2.try_acquire(3).unwrap());
        assert!(t2.try_acquire(3).unwrap());
        assert!(!t2.try_acquire(2).unwrap());
    }
}
