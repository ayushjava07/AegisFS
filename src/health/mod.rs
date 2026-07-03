use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Ok,
    Degraded(String),
    Fatal(String),
}

#[derive(Debug, Clone)]
pub struct HealthReport {
    pub name: String,
    pub status: HealthStatus,
    pub message: Option<String>,
}

type HealthCheck = Box<dyn Fn() -> HealthStatus + Send + Sync>;

pub struct HealthRegistry {
    checks: HashMap<String, HealthCheck>,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self {
            checks: HashMap::new(),
        }
    }

    pub fn register<N, F>(&mut self, name: N, check: F)
    where
        N: Into<String>,
        F: Fn() -> HealthStatus + Send + Sync + 'static,
    {
        self.checks.insert(name.into(), Box::new(check));
    }

    pub fn unregister(&mut self, name: &str) {
        self.checks.remove(name);
    }

    pub fn check_all(&self) -> Vec<HealthReport> {
        let mut reports = Vec::with_capacity(self.checks.len());
        for (name, check) in &self.checks {
            let status = check();
            let message = match &status {
                HealthStatus::Ok => None,
                HealthStatus::Degraded(msg) => Some(msg.clone()),
                HealthStatus::Fatal(msg) => Some(msg.clone()),
            };
            reports.push(HealthReport {
                name: name.clone(),
                status,
                message,
            });
        }
        reports
    }

    pub fn is_healthy(&self) -> bool {
        self.check_all()
            .iter()
            .all(|r| r.status == HealthStatus::Ok)
    }

    pub fn has_fatal(&self) -> bool {
        self.check_all()
            .iter()
            .any(|r| matches!(r.status, HealthStatus::Fatal(_)))
    }

    pub fn register_liveness(&mut self) {
        self.register("liveness", || HealthStatus::Ok);
    }

    pub fn register_readiness(&mut self, ready: bool) {
        let ready_state = ready;
        self.register("readiness", move || {
            if ready_state {
                HealthStatus::Ok
            } else {
                HealthStatus::Degraded("not ready".into())
            }
        });
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_ok() {
        let mut registry = HealthRegistry::new();
        registry.register("test", || HealthStatus::Ok);
        let reports = registry.check_all();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, HealthStatus::Ok);
    }

    #[test]
    fn test_health_degraded() {
        let mut registry = HealthRegistry::new();
        registry.register("disk", || HealthStatus::Degraded("low space".into()));
        assert!(!registry.is_healthy());
    }

    #[test]
    fn test_health_fatal() {
        let mut registry = HealthRegistry::new();
        registry.register("db", || HealthStatus::Fatal("connection failed".into()));
        assert!(registry.has_fatal());
    }

    #[test]
    fn test_liveness() {
        let mut registry = HealthRegistry::new();
        registry.register_liveness();
        assert!(registry.is_healthy());
    }

    #[test]
    fn test_readiness() {
        let mut registry = HealthRegistry::new();
        registry.register_readiness(true);
        assert!(registry.is_healthy());
        let mut registry2 = HealthRegistry::new();
        registry2.register_readiness(false);
        assert!(!registry2.is_healthy());
    }
}
