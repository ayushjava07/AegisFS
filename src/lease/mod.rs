use std::collections::HashMap;
use std::time::Instant;

use parking_lot::Mutex;

use crate::core::error::{AegisError, AegisResult};

#[derive(Debug, Clone)]
pub struct Lease {
    pub name: String,
    pub holder: String,
    pub expires_at: Instant,
    pub renewed_at: Instant,
}

pub struct LeaseManager {
    leases: Mutex<HashMap<String, Lease>>,
}

impl LeaseManager {
    pub fn new() -> Self {
        Self {
            leases: Mutex::new(HashMap::new()),
        }
    }

    pub fn acquire(&self, name: &str, ttl_secs: u64) -> AegisResult<Lease> {
        let mut leases = self.leases.lock();

        if let Some(existing) = leases.get(name) {
            if existing.expires_at > Instant::now() {
                return Err(AegisError::AlreadyExists(format!(
                    "lease '{}' is already held",
                    name
                )));
            }
        }

        let now = Instant::now();
        let lease = Lease {
            name: name.to_string(),
            holder: format!("node-{}", std::process::id()),
            expires_at: now + std::time::Duration::from_secs(ttl_secs),
            renewed_at: now,
        };
        leases.insert(name.to_string(), lease.clone());
        Ok(lease)
    }

    pub fn renew(&self, lease: &Lease) -> AegisResult<()> {
        let mut leases = self.leases.lock();

        match leases.get_mut(&lease.name) {
            Some(existing) => {
                let now = Instant::now();
                existing.expires_at = now + (lease.expires_at - lease.renewed_at);
                existing.renewed_at = now;
                Ok(())
            }
            None => Err(AegisError::Internal(format!(
                "lease '{}' not found",
                lease.name
            ))),
        }
    }

    pub fn release(&self, lease: &Lease) -> AegisResult<()> {
        let mut leases = self.leases.lock();

        if leases.remove(&lease.name).is_some() {
            Ok(())
        } else {
            Err(AegisError::Internal(format!(
                "lease '{}' not found",
                lease.name
            )))
        }
    }

    pub fn is_held(&self, name: &str) -> bool {
        let leases = self.leases.lock();
        leases
            .get(name)
            .is_some_and(|l| l.expires_at > Instant::now())
    }
}

impl Default for LeaseManager {
    fn default() -> Self {
        Self::new()
    }
}
