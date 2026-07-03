use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Scope {
    name: String,
    cancelled: Arc<AtomicBool>,
}

impl Scope {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn child(&self, name: &str) -> Self {
        Self {
            name: format!("{}/{}", self.name, name),
            cancelled: self.cancelled.clone(),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
