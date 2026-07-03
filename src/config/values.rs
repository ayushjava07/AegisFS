use serde::{Deserialize, Serialize};

use crate::core::types::*;

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
