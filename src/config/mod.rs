use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::core::error::{AegisError, AegisResult};
use crate::core::types::*;
use crate::sync::SyncConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AegisConfig {
    pub storage: StorageConfig,
    pub archive_defaults: ArchiveDefaults,
    pub cache: CacheConfig,
    pub compression: CompressionConfig,
    pub encryption: EncryptionConfig,
    pub sync: SyncConfig,
    pub replication: ReplicationConfig,
    pub logging: LoggingConfig,
    pub scheduler: SchedulerConfig,
}

impl Default for AegisConfig {
    fn default() -> Self {
        Self {
            storage: StorageConfig {
                kind: StorageBackendKind::Local,
                path: Some("/var/lib/aegisfs/data".into()),
                endpoint: None,
                bucket: None,
                region: None,
                credentials: None,
            },
            archive_defaults: ArchiveDefaults::default(),
            cache: CacheConfig::default(),
            compression: CompressionConfig::default(),
            encryption: EncryptionConfig::default(),
            sync: SyncConfig::default(),
            replication: ReplicationConfig::default(),
            logging: LoggingConfig::default(),
            scheduler: SchedulerConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveDefaults {
    pub chunk_size: u64,
    pub compression: CompressionAlgorithm,
    pub dedup_enabled: bool,
}

impl Default for ArchiveDefaults {
    fn default() -> Self {
        Self {
            chunk_size: 64 * 1024,
            compression: CompressionAlgorithm::Zstd(3),
            dedup_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub max_entries: usize,
    pub max_memory_mb: usize,
    pub ttl_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_memory_mb: 512,
            ttl_seconds: 3600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    pub algorithm: CompressionAlgorithm,
    pub level: i32,
    pub min_size_for_compression: u64,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Zstd(3),
            level: 3,
            min_size_for_compression: 256,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    pub algorithm: EncryptionAlgorithm,
    pub key_path: Option<String>,
    pub auto_generate: bool,
}

impl Default for EncryptionConfig {
    fn default() -> Self {
        Self {
            algorithm: EncryptionAlgorithm::Aes256Gcm,
            key_path: None,
            auto_generate: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: String,
    pub file_path: Option<String>,
    pub max_files: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: "json".into(),
            file_path: None,
            max_files: 7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    pub worker_threads: usize,
    pub max_queue_depth: usize,
    pub task_timeout_seconds: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            worker_threads: 4,
            max_queue_depth: 10_000,
            task_timeout_seconds: 300,
        }
    }
}

pub struct ConfigBuilder;

impl ConfigBuilder {
    pub fn from_file(path: &str) -> AegisResult<AegisConfig> {
        let data = fs::read_to_string(path).map_err(|e| {
            AegisError::InvalidConfig(format!("failed to read config file '{}': {}", path, e))
        })?;
        serde_json::from_str(&data).map_err(|e| {
            AegisError::InvalidConfig(format!("failed to parse config file '{}': {}", path, e))
        })
    }

    pub fn from_env() -> AegisConfig {
        let mut config = AegisConfig::default();

        if let Ok(v) = std::env::var("AEGIS_STORAGE_KIND") {
            config.storage.kind = match v.to_lowercase().as_str() {
                "local" => StorageBackendKind::Local,
                "memory" => StorageBackendKind::Memory,
                "s3" => StorageBackendKind::S3,
                "gcs" => StorageBackendKind::Gcs,
                "azure" => StorageBackendKind::Azure,
                _ => StorageBackendKind::Local,
            };
        }
        if let Ok(v) = std::env::var("AEGIS_STORAGE_PATH") {
            config.storage.path = Some(v);
        }
        if let Ok(v) = std::env::var("AEGIS_STORAGE_ENDPOINT") {
            config.storage.endpoint = Some(v);
        }
        if let Ok(v) = std::env::var("AEGIS_STORAGE_BUCKET") {
            config.storage.bucket = Some(v);
        }
        if let Ok(v) = std::env::var("AEGIS_STORAGE_REGION") {
            config.storage.region = Some(v);
        }
        if let Ok(v) = std::env::var("AEGIS_CHUNK_SIZE") {
            if let Ok(n) = v.parse::<u64>() {
                config.archive_defaults.chunk_size = n;
            }
        }
        if let Ok(v) = std::env::var("AEGIS_CACHE_MAX_ENTRIES") {
            if let Ok(n) = v.parse::<usize>() {
                config.cache.max_entries = n;
            }
        }
        if let Ok(v) = std::env::var("AEGIS_CACHE_MAX_MEMORY_MB") {
            if let Ok(n) = v.parse::<usize>() {
                config.cache.max_memory_mb = n;
            }
        }
        if let Ok(v) = std::env::var("AEGIS_CACHE_TTL_SECONDS") {
            if let Ok(n) = v.parse::<u64>() {
                config.cache.ttl_seconds = n;
            }
        }
        if let Ok(v) = std::env::var("AEGIS_LOG_LEVEL") {
            config.logging.level = match v.to_lowercase().as_str() {
                "trace" => LogLevel::Trace,
                "debug" => LogLevel::Debug,
                "info" => LogLevel::Info,
                "warn" => LogLevel::Warn,
                "error" => LogLevel::Error,
                _ => LogLevel::Info,
            };
        }
        if let Ok(v) = std::env::var("AEGIS_LOG_FORMAT") {
            config.logging.format = v;
        }
        if let Ok(v) = std::env::var("AEGIS_LOG_FILE") {
            config.logging.file_path = Some(v);
        }
        if let Ok(v) = std::env::var("AEGIS_WORKER_THREADS") {
            if let Ok(n) = v.parse::<usize>() {
                config.scheduler.worker_threads = n;
            }
        }
        if let Ok(v) = std::env::var("AEGIS_TASK_TIMEOUT_SECONDS") {
            if let Ok(n) = v.parse::<u64>() {
                config.scheduler.task_timeout_seconds = n;
            }
        }

        config
    }

    pub fn with_storage(mut config: AegisConfig, storage: StorageConfig) -> AegisConfig {
        config.storage = storage;
        config
    }

    pub fn with_archive_defaults(
        mut config: AegisConfig,
        defaults: ArchiveDefaults,
    ) -> AegisConfig {
        config.archive_defaults = defaults;
        config
    }

    pub fn with_cache(mut config: AegisConfig, cache: CacheConfig) -> AegisConfig {
        config.cache = cache;
        config
    }

    pub fn with_compression(
        mut config: AegisConfig,
        compression: CompressionConfig,
    ) -> AegisConfig {
        config.compression = compression;
        config
    }

    pub fn with_encryption(mut config: AegisConfig, encryption: EncryptionConfig) -> AegisConfig {
        config.encryption = encryption;
        config
    }

    pub fn with_sync(mut config: AegisConfig, sync: SyncConfig) -> AegisConfig {
        config.sync = sync;
        config
    }

    pub fn with_replication(
        mut config: AegisConfig,
        replication: ReplicationConfig,
    ) -> AegisConfig {
        config.replication = replication;
        config
    }

    pub fn with_logging(mut config: AegisConfig, logging: LoggingConfig) -> AegisConfig {
        config.logging = logging;
        config
    }

    pub fn with_scheduler(mut config: AegisConfig, scheduler: SchedulerConfig) -> AegisConfig {
        config.scheduler = scheduler;
        config
    }
}

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn load(paths: &[PathBuf]) -> AegisResult<AegisConfig> {
        if paths.is_empty() {
            return Ok(AegisConfig::default());
        }

        let mut config = AegisConfig::default();

        for path in paths {
            if path.exists() {
                let partial: AegisConfig = ConfigBuilder::from_file(&path.to_string_lossy())?;
                config = Self::merge(config, partial);
            }
        }

        Ok(config)
    }

    pub fn merge(_base: AegisConfig, overrides: AegisConfig) -> AegisConfig {
        AegisConfig {
            storage: overrides.storage,
            archive_defaults: overrides.archive_defaults,
            cache: overrides.cache,
            compression: overrides.compression,
            encryption: overrides.encryption,
            sync: overrides.sync,
            replication: overrides.replication,
            logging: overrides.logging,
            scheduler: overrides.scheduler,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_default_values() {
        let config = AegisConfig::default();
        assert_eq!(config.archive_defaults.chunk_size, 64 * 1024);
        assert_eq!(
            config.archive_defaults.compression,
            CompressionAlgorithm::Zstd(3)
        );
        assert!(config.archive_defaults.dedup_enabled);
        assert_eq!(config.cache.max_entries, 10_000);
        assert_eq!(config.cache.max_memory_mb, 512);
        assert_eq!(config.cache.ttl_seconds, 3600);
        assert_eq!(config.compression.algorithm, CompressionAlgorithm::Zstd(3));
        assert_eq!(config.compression.level, 3);
        assert_eq!(config.compression.min_size_for_compression, 256);
        assert_eq!(config.encryption.algorithm, EncryptionAlgorithm::Aes256Gcm);
        assert!(config.encryption.auto_generate);
        assert!(config.encryption.key_path.is_none());
        assert_eq!(config.logging.level, LogLevel::Info);
        assert_eq!(config.logging.format, "json");
        assert_eq!(config.logging.max_files, 7);
        assert_eq!(config.scheduler.worker_threads, 4);
        assert_eq!(config.scheduler.max_queue_depth, 10_000);
        assert_eq!(config.scheduler.task_timeout_seconds, 300);
    }

    #[test]
    fn test_config_builder_roundtrip() -> AegisResult<()> {
        let mut config = AegisConfig::default();
        config.archive_defaults.chunk_size = 128 * 1024;
        config.cache.max_entries = 20_000;
        config.compression.level = 6;
        config.encryption.algorithm = EncryptionAlgorithm::ChaCha20Poly1305;
        config.logging.level = LogLevel::Debug;
        config.scheduler.worker_threads = 8;

        let json = serde_json::to_string(&config).unwrap();
        let restored: AegisConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.archive_defaults.chunk_size, 128 * 1024);
        assert_eq!(restored.cache.max_entries, 20_000);
        assert_eq!(restored.compression.level, 6);
        assert_eq!(
            restored.encryption.algorithm,
            EncryptionAlgorithm::ChaCha20Poly1305
        );
        assert_eq!(restored.logging.level, LogLevel::Debug);
        assert_eq!(restored.scheduler.worker_threads, 8);

        Ok(())
    }

    #[test]
    fn test_merge_overrides_all() {
        let base = AegisConfig::default();
        let mut overrides = AegisConfig::default();
        overrides.cache.max_entries = 99;
        overrides.cache.ttl_seconds = 42;

        let merged = ConfigLoader::merge(base, overrides);
        assert_eq!(merged.cache.max_entries, 99);
        assert_eq!(merged.cache.ttl_seconds, 42);
    }

    #[test]
    fn test_merge_preserves_unrelated() {
        let mut base = AegisConfig::default();
        base.archive_defaults.chunk_size = 999;

        let overrides = AegisConfig::default();
        let merged = ConfigLoader::merge(base, overrides);

        assert_eq!(merged.archive_defaults.chunk_size, 64 * 1024);
    }

    #[test]
    fn test_load_empty_paths() -> AegisResult<()> {
        let config = ConfigLoader::load(&[])?;
        assert_eq!(config.archive_defaults.chunk_size, 64 * 1024);
        Ok(())
    }

    #[test]
    fn test_load_nonexistent_path() -> AegisResult<()> {
        let path = PathBuf::from("/tmp/nonexistent_aegis_config_xxxxx.json");
        let config = ConfigLoader::load(&[path])?;
        assert_eq!(config.archive_defaults.chunk_size, 64 * 1024);
        Ok(())
    }

    #[test]
    fn test_from_file() -> AegisResult<()> {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test_config.json");
        let config = AegisConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        std::fs::write(&file_path, &json).unwrap();

        let loaded = ConfigBuilder::from_file(&file_path.to_string_lossy())?;
        assert_eq!(loaded.cache.max_entries, 10_000);
        assert_eq!(loaded.scheduler.worker_threads, 4);
        Ok(())
    }

    #[test]
    fn test_builder_with_methods() {
        let config = AegisConfig::default();

        let custom_cache = CacheConfig {
            max_entries: 5000,
            max_memory_mb: 256,
            ttl_seconds: 1800,
        };

        let updated = ConfigBuilder::with_cache(config, custom_cache);
        assert_eq!(updated.cache.max_entries, 5000);
        assert_eq!(updated.cache.max_memory_mb, 256);
        assert_eq!(updated.cache.ttl_seconds, 1800);
    }

    #[test]
    fn test_builder_chain() {
        let config = AegisConfig::default();

        let custom_compression = CompressionConfig {
            algorithm: CompressionAlgorithm::Lz4,
            level: 0,
            min_size_for_compression: 512,
        };

        let custom_logging = LoggingConfig {
            level: LogLevel::Warn,
            format: "text".into(),
            file_path: Some("/var/log/aegis.log".into()),
            max_files: 14,
        };

        let updated = ConfigBuilder::with_compression(
            ConfigBuilder::with_logging(config, custom_logging),
            custom_compression,
        );

        assert_eq!(updated.compression.algorithm, CompressionAlgorithm::Lz4);
        assert_eq!(updated.compression.min_size_for_compression, 512);
        assert_eq!(updated.logging.level, LogLevel::Warn);
        assert_eq!(
            updated.logging.file_path.as_deref(),
            Some("/var/log/aegis.log")
        );
        assert_eq!(updated.logging.max_files, 14);
    }
}
