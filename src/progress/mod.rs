use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

type ProgressListener = Box<dyn Fn(ProgressEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Advanced {
        current: u64,
        total: u64,
        percent: f64,
    },
    MessageChanged(String),
    Completed,
}

pub struct ProgressTracker {
    current: AtomicU64,
    total: u64,
    message: parking_lot::Mutex<String>,
    completed: AtomicBool,
    listeners: parking_lot::Mutex<Vec<ProgressListener>>,
}

impl ProgressTracker {
    pub fn new(total: u64) -> Self {
        Self {
            current: AtomicU64::new(0),
            total,
            message: parking_lot::Mutex::new(String::new()),
            completed: AtomicBool::new(false),
            listeners: parking_lot::Mutex::new(Vec::new()),
        }
    }

    pub fn advance(&self, n: u64) {
        let prev = self.current.fetch_add(n, Ordering::SeqCst);
        let current = prev + n;
        let pct = self.calc_percent(current);
        let event = ProgressEvent::Advanced {
            current,
            total: self.total,
            percent: pct,
        };
        self.notify(event);
        if current >= self.total {
            self.completed.store(true, Ordering::SeqCst);
            self.notify(ProgressEvent::Completed);
        }
    }

    pub fn set_message(&self, msg: &str) {
        let mut m = self.message.lock();
        *m = msg.to_string();
        self.notify(ProgressEvent::MessageChanged(msg.to_string()));
    }

    pub fn message(&self) -> String {
        self.message.lock().clone()
    }

    pub fn current(&self) -> u64 {
        self.current.load(Ordering::SeqCst)
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn percent(&self) -> f64 {
        self.calc_percent(self.current.load(Ordering::SeqCst))
    }

    pub fn is_complete(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }

    pub fn reset(&mut self, total: u64) {
        self.current.store(0, Ordering::SeqCst);
        self.total = total;
        self.completed.store(false, Ordering::SeqCst);
    }

    pub fn on_event<F>(&self, listener: F)
    where
        F: Fn(ProgressEvent) + Send + Sync + 'static,
    {
        self.listeners.lock().push(Box::new(listener));
    }

    fn calc_percent(&self, current: u64) -> f64 {
        if self.total == 0 {
            100.0
        } else {
            (current as f64 / self.total as f64) * 100.0
        }
    }

    fn notify(&self, event: ProgressEvent) {
        let listeners = self.listeners.lock();
        for listener in listeners.iter() {
            listener(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_progress_basic() {
        let p = ProgressTracker::new(100);
        assert_eq!(p.percent(), 0.0);
        assert!(!p.is_complete());
        p.advance(50);
        assert_eq!(p.percent(), 50.0);
        p.advance(50);
        assert_eq!(p.percent(), 100.0);
        assert!(p.is_complete());
    }

    #[test]
    fn test_progress_message() {
        let p = ProgressTracker::new(10);
        p.set_message("processing");
        assert_eq!(p.message(), "processing");
    }

    #[test]
    fn test_progress_reset() {
        let mut p = ProgressTracker::new(10);
        p.advance(10);
        assert!(p.is_complete());
        p.reset(20);
        assert!(!p.is_complete());
        assert_eq!(p.current(), 0);
        assert_eq!(p.total(), 20);
    }

    #[test]
    fn test_progress_zero_total() {
        let p = ProgressTracker::new(0);
        assert_eq!(p.percent(), 100.0);
    }

    #[test]
    fn test_progress_listener() {
        let p = ProgressTracker::new(10);
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let events_clone = events.clone();
        p.on_event(move |e| {
            events_clone.lock().push(e);
        });
        p.advance(10);
        let evts = events.lock();
        assert_eq!(evts.len(), 2);
    }
}
