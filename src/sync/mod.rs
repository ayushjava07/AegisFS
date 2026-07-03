use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::core::error::AegisResult;
use crate::core::traits::*;
use crate::core::types::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub direction: SyncDirection,
    pub conflict_resolution: ConflictResolution,
    pub batch_size: usize,
    pub verify_after_sync: bool,
    pub compression: CompressionAlgorithm,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            direction: SyncDirection::Push,
            conflict_resolution: ConflictResolution::SourceWins,
            batch_size: 100,
            verify_after_sync: true,
            compression: CompressionAlgorithm::Zstd(3),
        }
    }
}

pub struct SyncEngineImpl {
    status: Arc<Mutex<SyncStatus>>,
}

impl SyncEngineImpl {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(SyncStatus::idle())),
        }
    }
}

impl Default for SyncEngineImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncEngine for SyncEngineImpl {
    fn sync_to_remote(
        &self,
        _archive_id: &ArchiveId,
        _direction: SyncDirection,
    ) -> BoxFuture<'_, AegisResult<SyncResult>> {
        let status = self.status.clone();
        Box::pin(async move {
            let start = Instant::now();
            {
                let mut s = status.lock();
                s.in_progress = true;
                s.progress_percent = 0.0;
                s.current_file = Some("starting sync".to_string());
                s.bytes_so_far = 0;
                s.errors_so_far = 0;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            {
                let mut s = status.lock();
                s.in_progress = false;
                s.progress_percent = 100.0;
                s.current_file = None;
            }
            Ok(SyncResult {
                files_transferred: 0,
                bytes_transferred: 0,
                conflicts_resolved: 0,
                failed_files: Vec::new(),
                duration_seconds: start.elapsed().as_secs_f64(),
            })
        })
    }

    fn sync_snapshot(
        &self,
        _snapshot_id: &SnapshotId,
        _target: &str,
    ) -> BoxFuture<'_, AegisResult<SyncResult>> {
        let status = self.status.clone();
        Box::pin(async move {
            let start = Instant::now();
            {
                let mut s = status.lock();
                s.in_progress = true;
                s.progress_percent = 0.0;
                s.current_file = Some("starting snapshot sync".to_string());
                s.bytes_so_far = 0;
                s.errors_so_far = 0;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            {
                let mut s = status.lock();
                s.in_progress = false;
                s.progress_percent = 100.0;
                s.current_file = None;
            }
            Ok(SyncResult {
                files_transferred: 0,
                bytes_transferred: 0,
                conflicts_resolved: 0,
                failed_files: Vec::new(),
                duration_seconds: start.elapsed().as_secs_f64(),
            })
        })
    }

    fn status(&self) -> BoxFuture<'_, AegisResult<SyncStatus>> {
        let status = self.status.clone();
        Box::pin(async move {
            let s = status.lock();
            Ok(s.clone())
        })
    }

    fn cancel(&self) -> BoxFuture<'_, AegisResult<()>> {
        let status = self.status.clone();
        Box::pin(async move {
            let mut s = status.lock();
            s.in_progress = false;
            s.current_file = None;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_sync_config_defaults() {
        let config = SyncConfig::default();
        assert_eq!(config.direction, SyncDirection::Push);
        assert_eq!(config.conflict_resolution, ConflictResolution::SourceWins);
        assert_eq!(config.batch_size, 100);
        assert!(config.verify_after_sync);
        assert_eq!(config.compression, CompressionAlgorithm::Zstd(3));
    }

    #[test]
    fn test_sync_config_custom() {
        let config = SyncConfig {
            direction: SyncDirection::Bidirectional,
            conflict_resolution: ConflictResolution::LatestWins,
            batch_size: 50,
            verify_after_sync: false,
            compression: CompressionAlgorithm::Lz4,
        };
        assert_eq!(config.direction, SyncDirection::Bidirectional);
        assert_eq!(config.conflict_resolution, ConflictResolution::LatestWins);
        assert_eq!(config.batch_size, 50);
        assert!(!config.verify_after_sync);
        assert_eq!(config.compression, CompressionAlgorithm::Lz4);
    }

    #[test]
    fn test_sync_engine_status_idle_initial() {
        let engine = SyncEngineImpl::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let status = rt.block_on(engine.status()).unwrap();
        assert!(!status.in_progress);
        assert_eq!(status.progress_percent, 0.0);
        assert!(status.current_file.is_none());
        assert_eq!(status.bytes_so_far, 0);
        assert_eq!(status.errors_so_far, 0);
    }

    #[test]
    fn test_sync_result_tracking() {
        let result = SyncResult {
            files_transferred: 42,
            bytes_transferred: 1048576,
            conflicts_resolved: 3,
            failed_files: vec!["file1.txt".to_string(), "file2.txt".to_string()],
            duration_seconds: 12.5,
        };
        assert_eq!(result.files_transferred, 42);
        assert_eq!(result.bytes_transferred, 1048576);
        assert_eq!(result.conflicts_resolved, 3);
        assert_eq!(result.failed_files.len(), 2);
        assert!(result.duration_seconds > 0.0);
    }

    #[test]
    fn test_sync_result_default() {
        let result = SyncResult::new();
        assert_eq!(result.files_transferred, 0);
        assert_eq!(result.bytes_transferred, 0);
        assert_eq!(result.conflicts_resolved, 0);
        assert!(result.failed_files.is_empty());
        assert_eq!(result.duration_seconds, 0.0);
    }

    #[test]
    fn test_sync_engine_sync_to_remote() {
        let engine = SyncEngineImpl::new();
        let archive_id = ArchiveId::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(engine.sync_to_remote(&archive_id, SyncDirection::Push))
            .unwrap();
        assert_eq!(result.files_transferred, 0);
        assert_eq!(result.bytes_transferred, 0);
    }

    #[test]
    fn test_sync_engine_sync_snapshot() {
        let engine = SyncEngineImpl::new();
        let snapshot_id = SnapshotId::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(engine.sync_snapshot(&snapshot_id, "/backup"))
            .unwrap();
        assert_eq!(result.files_transferred, 0);
    }

    #[test]
    fn test_sync_engine_cancel() {
        let engine = SyncEngineImpl::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(engine.cancel()).unwrap();
        let status = rt.block_on(engine.status()).unwrap();
        assert!(!status.in_progress);
    }

    #[test]
    fn test_sync_engine_status_updates_during_sync() {
        let engine = Arc::new(SyncEngineImpl::new());
        let engine_clone = engine.clone();
        let archive_id = ArchiveId::new();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let initial_status = rt.block_on(engine.status()).unwrap();
        assert!(!initial_status.in_progress);

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(engine_clone.sync_to_remote(&archive_id, SyncDirection::Push))
                .unwrap();
        });

        std::thread::sleep(std::time::Duration::from_millis(5));
        let during = rt.block_on(engine.status()).unwrap();
        assert!(during.in_progress);

        handle.join().unwrap();
        let after = rt.block_on(engine.status()).unwrap();
        assert!(!after.in_progress);
    }
}
