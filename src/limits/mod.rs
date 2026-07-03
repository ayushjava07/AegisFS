use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::error::{AegisError, AegisResult};

#[derive(Debug)]
struct ResourceUsage {
    memory_used: AtomicU64,
    disk_used: AtomicU64,
    #[allow(dead_code)]
    fds_used: AtomicU64,
}

impl ResourceUsage {
    fn new() -> Self {
        Self {
            memory_used: AtomicU64::new(0),
            disk_used: AtomicU64::new(0),
            fds_used: AtomicU64::new(0),
        }
    }
}

#[derive(Debug)]
pub struct ResourceLimits {
    max_memory: u64,
    max_disk: u64,
    #[allow(dead_code)]
    max_fds: u64,
    usage: ResourceUsage,
}

impl ResourceLimits {
    pub fn new(max_memory: u64, max_disk: u64, max_fds: u64) -> Self {
        Self {
            max_memory,
            max_disk,
            max_fds,
            usage: ResourceUsage::new(),
        }
    }

    pub fn check_memory(&self, bytes: u64) -> AegisResult<()> {
        let used = self.usage.memory_used.load(Ordering::Acquire);
        if used + bytes > self.max_memory {
            return Err(AegisError::ResourceExhausted(format!(
                "memory limit exceeded: {} + {} > {}",
                used, bytes, self.max_memory
            )));
        }
        self.usage.memory_used.fetch_add(bytes, Ordering::Release);
        Ok(())
    }

    pub fn check_disk(&self, bytes: u64) -> AegisResult<()> {
        let used = self.usage.disk_used.load(Ordering::Acquire);
        if used + bytes > self.max_disk {
            return Err(AegisError::ResourceExhausted(format!(
                "disk limit exceeded: {} + {} > {}",
                used, bytes, self.max_disk
            )));
        }
        self.usage.disk_used.fetch_add(bytes, Ordering::Release);
        Ok(())
    }

    pub fn memory_available(&self) -> u64 {
        self.max_memory
            .saturating_sub(self.usage.memory_used.load(Ordering::Acquire))
    }

    pub fn disk_available(&self) -> u64 {
        self.max_disk
            .saturating_sub(self.usage.disk_used.load(Ordering::Acquire))
    }
}
