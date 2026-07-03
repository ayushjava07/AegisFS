use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::error::AegisResult;
use crate::core::types::*;
use crate::sync::SyncConfig;

mod builder;
mod values;

pub use builder::ConfigBuilder;
pub use values::*;

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
