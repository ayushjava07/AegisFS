use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;
use crate::core::types::*;

impl ReplicationConfig {
    pub fn new(mode: ReplicationMode) -> Self {
        Self {
            mode,
            targets: Vec::new(),
            bandwidth_limit_bps: 0,
            verify_replicas: true,
        }
    }

    pub fn validate(&self) -> AegisResult<()> {
        if self.targets.is_empty() {
            return Err(AegisError::InvalidConfig(
                "at least one replication target is required".to_string(),
            ));
        }
        for target in &self.targets {
            if target.name.is_empty() {
                return Err(AegisError::InvalidConfig(
                    "replication target name must not be empty".to_string(),
                ));
            }
            if target.endpoint.is_empty() {
                return Err(AegisError::InvalidConfig(
                    "replication target endpoint must not be empty".to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl ReplicationTarget {
    pub fn new(name: &str, endpoint: &str, backend: StorageBackendKind) -> Self {
        Self {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            credentials: None,
            backend,
        }
    }
}

impl ReplicationResult {
    pub fn success(target: &str, chunks: u64, bytes: u64, duration: f64) -> Self {
        Self {
            target: target.to_string(),
            chunks_replicated: chunks,
            bytes_replicated: bytes,
            duration_seconds: duration,
            success: true,
            error: None,
        }
    }

    pub fn failure(target: &str, error: &str) -> Self {
        Self {
            target: target.to_string(),
            chunks_replicated: 0,
            bytes_replicated: 0,
            duration_seconds: 0.0,
            success: false,
            error: Some(error.to_string()),
        }
    }
}

impl ReplicationStatus {
    pub fn new() -> Self {
        Self {
            active_jobs: 0,
            completed_jobs: 0,
            failed_jobs: 0,
            total_bytes_replicated: 0,
        }
    }
}

impl Default for ReplicationStatus {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReplicationEngineImpl {
    config: Mutex<ReplicationConfig>,
    status: Arc<Mutex<ReplicationStatus>>,
    active_jobs: AtomicUsize,
    completed_jobs: AtomicUsize,
    failed_jobs: AtomicUsize,
    total_bytes: AtomicUsize,
}

impl ReplicationEngineImpl {
    pub fn new(config: ReplicationConfig) -> Self {
        Self {
            config: Mutex::new(config),
            status: Arc::new(Mutex::new(ReplicationStatus::new())),
            active_jobs: AtomicUsize::new(0),
            completed_jobs: AtomicUsize::new(0),
            failed_jobs: AtomicUsize::new(0),
            total_bytes: AtomicUsize::new(0),
        }
    }
}

impl ReplicationEngine for ReplicationEngineImpl {
    fn replicate(
        &self,
        _archive_id: &ArchiveId,
        target: &ReplicationTarget,
    ) -> BoxFuture<'_, AegisResult<ReplicationResult>> {
        let target_name = target.name.clone();
        let status = self.status.clone();
        let active = &self.active_jobs;
        let completed = &self.completed_jobs;
        let _failed = &self.failed_jobs;
        let total_bytes = &self.total_bytes;

        Box::pin(async move {
            active.fetch_add(1, Ordering::SeqCst);
            {
                let mut s = status.lock().unwrap();
                s.active_jobs = active.load(Ordering::SeqCst);
            }

            let start = Instant::now();
            let simulated_bytes = 1024u64;

            tokio::time::sleep(std::time::Duration::from_millis(5)).await;

            let elapsed = start.elapsed().as_secs_f64();

            active.fetch_sub(1, Ordering::SeqCst);
            completed.fetch_add(1, Ordering::SeqCst);
            total_bytes.fetch_add(simulated_bytes as usize, Ordering::SeqCst);

            let mut s = status.lock().unwrap();
            s.active_jobs = active.load(Ordering::SeqCst);
            s.completed_jobs = completed.load(Ordering::SeqCst);
            s.total_bytes_replicated = total_bytes.load(Ordering::SeqCst) as u64;

            Ok(ReplicationResult::success(
                &target_name,
                1,
                simulated_bytes,
                elapsed,
            ))
        })
    }

    fn configure_replication(&self, config: ReplicationConfig) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move {
            config.validate()?;
            let mut cfg = self.config.lock().unwrap();
            *cfg = config;
            Ok(())
        })
    }

    fn status(&self) -> BoxFuture<'_, AegisResult<ReplicationStatus>> {
        let status = self.status.clone();
        Box::pin(async move {
            let s = status.lock().unwrap();
            Ok(s.clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_target(name: &str) -> ReplicationTarget {
        ReplicationTarget::new(name, "s3://backup-bucket", StorageBackendKind::S3)
    }

    #[test]
    fn test_replication_config_default() {
        let config = ReplicationConfig::default();
        assert_eq!(config.mode, ReplicationMode::Async);
        assert!(config.targets.is_empty());
        assert_eq!(config.bandwidth_limit_bps, 0);
        assert!(config.verify_replicas);
    }

    #[test]
    fn test_replication_config_validation_empty_targets() {
        let config = ReplicationConfig::new(ReplicationMode::Sync);
        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("at least one replication target"));
    }

    #[test]
    fn test_replication_config_validation_empty_name() {
        let mut config = ReplicationConfig::new(ReplicationMode::Sync);
        config.targets.push(ReplicationTarget::new(
            "",
            "s3://bucket",
            StorageBackendKind::S3,
        ));
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_replication_config_validation_empty_endpoint() {
        let mut config = ReplicationConfig::new(ReplicationMode::Sync);
        config.targets.push(ReplicationTarget::new(
            "target1",
            "",
            StorageBackendKind::S3,
        ));
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_replication_config_validation_valid() {
        let mut config = ReplicationConfig::new(ReplicationMode::Sync);
        config.targets.push(test_target("backup-s3"));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_replication_engine_status_initial() {
        let config = ReplicationConfig::default();
        let engine = ReplicationEngineImpl::new(config);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let status = rt.block_on(engine.status()).unwrap();
        assert_eq!(status.active_jobs, 0);
        assert_eq!(status.completed_jobs, 0);
        assert_eq!(status.failed_jobs, 0);
        assert_eq!(status.total_bytes_replicated, 0);
    }

    #[test]
    fn test_replication_result_success() {
        let result = ReplicationResult::success("backup-s3", 10, 2048, 1.5);
        assert!(result.success);
        assert_eq!(result.target, "backup-s3");
        assert_eq!(result.chunks_replicated, 10);
        assert_eq!(result.bytes_replicated, 2048);
        assert!(result.duration_seconds > 0.0);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_replication_result_failure() {
        let result = ReplicationResult::failure("backup-s3", "connection timeout");
        assert!(!result.success);
        assert_eq!(result.target, "backup-s3");
        assert_eq!(result.chunks_replicated, 0);
        assert_eq!(result.bytes_replicated, 0);
        assert_eq!(result.duration_seconds, 0.0);
        assert_eq!(result.error.as_deref(), Some("connection timeout"));
    }

    #[test]
    fn test_replication_engine_replicate() {
        let config = ReplicationConfig::default();
        let engine = ReplicationEngineImpl::new(config);
        let archive_id = ArchiveId::new();
        let target = test_target("backup-s3");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(engine.replicate(&archive_id, &target)).unwrap();

        assert!(result.success);
        assert_eq!(result.target, "backup-s3");
        assert!(result.bytes_replicated > 0);
    }

    #[test]
    fn test_replication_engine_configure() {
        let config = ReplicationConfig::default();
        let engine = ReplicationEngineImpl::new(config);

        let mut new_config = ReplicationConfig::new(ReplicationMode::Sync);
        new_config.targets.push(test_target("gcs-backup"));
        new_config.bandwidth_limit_bps = 100_000_000;
        new_config.verify_replicas = false;

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(engine.configure_replication(new_config.clone()))
            .unwrap();

        let status = rt.block_on(engine.status()).unwrap();
        assert_eq!(status.completed_jobs, 0);
    }

    #[test]
    fn test_replication_engine_replicate_updates_status() {
        let config = ReplicationConfig::default();
        let engine = ReplicationEngineImpl::new(config);
        let archive_id = ArchiveId::new();
        let target = test_target("s3-replica");

        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(engine.replicate(&archive_id, &target)).unwrap();
        assert!(result.success);

        let status = rt.block_on(engine.status()).unwrap();
        assert_eq!(status.completed_jobs, 1);
        assert!(status.total_bytes_replicated > 0);
    }

    #[test]
    fn test_replication_target_new() {
        let target = ReplicationTarget::new(
            "my-backup",
            "https://storage.example.com",
            StorageBackendKind::Gcs,
        );
        assert_eq!(target.name, "my-backup");
        assert_eq!(target.endpoint, "https://storage.example.com");
        assert_eq!(target.backend, StorageBackendKind::Gcs);
        assert!(target.credentials.is_none());
    }

    #[test]
    fn test_replication_engine_configure_validation_fails() {
        let config = ReplicationConfig::default();
        let engine = ReplicationEngineImpl::new(config);
        let invalid_config = ReplicationConfig::new(ReplicationMode::Sync);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(engine.configure_replication(invalid_config));
        assert!(result.is_err());
    }
}
