use std::fs;

use crate::core::error::{AegisError, AegisResult};
use crate::core::types::*;
use crate::sync::SyncConfig;

use super::values::*;
use super::AegisConfig;

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
