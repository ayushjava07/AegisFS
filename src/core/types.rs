use std::fmt;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::{AegisError, AegisResult};
pub use super::id::*;

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ChunkFlags: u32 {
        const DELETED = 1 << 0;
        const INLINE = 1 << 1;
        const COMPACTED = 1 << 2;
        const CHECKPOINT = 1 << 3;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Chunk {
    pub id: ChunkId,
    pub data: Bytes,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub compression_algorithm: Option<CompressionAlgorithm>,
    pub encryption_algorithm: Option<EncryptionAlgorithm>,
    pub checksum: HashValue,
    pub flags: ChunkFlags,
}

impl Chunk {
    pub fn new(id: ChunkId, data: Bytes) -> Self {
        let checksum = HashValue::sha256(&data);
        Self {
            id,
            size: data.len() as u64,
            data,
            compressed_size: None,
            compression_algorithm: None,
            encryption_algorithm: None,
            checksum,
            flags: ChunkFlags::empty(),
        }
    }

    pub fn is_encrypted(&self) -> bool {
        self.encryption_algorithm.is_some()
    }

    pub fn is_compressed(&self) -> bool {
        self.compression_algorithm.is_some()
    }

    pub fn verify_integrity(&self) -> bool {
        self.checksum.verify(&self.data)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompressionAlgorithm {
    Zstd(i32),
    Lz4,
    None,
}

impl CompressionAlgorithm {
    pub fn zstd_level(level: i32) -> Self {
        Self::Zstd(level.clamp(1, 22))
    }
}

impl fmt::Display for CompressionAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompressionAlgorithm::Zstd(level) => write!(f, "zstd({})", level),
            CompressionAlgorithm::Lz4 => write!(f, "lz4"),
            CompressionAlgorithm::None => write!(f, "none"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EncryptionAlgorithm {
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl fmt::Display for EncryptionAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncryptionAlgorithm::Aes256Gcm => write!(f, "aes-256-gcm"),
            EncryptionAlgorithm::ChaCha20Poly1305 => write!(f, "chacha20-poly1305"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub mode: NodePermissions,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub content_hash: Option<HashValue>,
    pub metadata: NodeMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum NodeKind {
    File,
    Directory,
    Symlink,
    VirtualLink,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodePermissions {
    pub owner: String,
    pub group: String,
    pub mode: u32,
}

impl NodePermissions {
    pub fn default_for(owner: &str) -> Self {
        Self {
            owner: owner.to_string(),
            group: owner.to_string(),
            mode: 0o644,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NodeMetadata {
    pub labels: std::collections::HashMap<String, String>,
    pub attributes: std::collections::HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Archive {
    pub id: ArchiveId,
    pub name: String,
    pub manifest: ManifestRef,
    pub encrypted: bool,
    pub compression: CompressionAlgorithm,
    pub created_at: DateTime<Utc>,
    pub sealed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestRef {
    pub id: ManifestId,
    pub root_node: NodeId,
    pub chunk_count: u64,
    pub total_size: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub parent: Option<SnapshotId>,
    pub archive_id: ArchiveId,
    pub manifest: ManifestRef,
    pub timestamp: DateTime<Utc>,
    pub labels: std::collections::HashMap<String, String>,
    pub incremental: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalEntry {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub kind: JournalEntryKind,
    pub data: Vec<u8>,
    pub checksum: HashValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum JournalEntryKind {
    CreateNode,
    DeleteNode,
    ModifyNode,
    CreateChunk,
    DeleteChunk,
    CreateSnapshot,
    DeleteSnapshot,
    SealArchive,
    Checkpoint,
    ConfigChange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrityProof {
    pub chunk_id: ChunkId,
    pub expected_hash: HashValue,
    pub actual_hash: HashValue,
    pub valid: bool,
    pub verified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum StorageBackendKind {
    Local,
    Memory,
    S3,
    Gcs,
    Azure,
    Custom(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub kind: StorageBackendKind,
    pub path: Option<String>,
    pub endpoint: Option<String>,
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub credentials: Option<CredentialsRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialsRef {
    pub key_id: String,
    pub encrypted_key: Vec<u8>,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkDescriptor {
    pub id: ChunkId,
    pub offset: u64,
    pub size: u64,
    pub checksum: HashValue,
}

impl ChunkDescriptor {
    pub fn new(id: ChunkId, offset: u64, size: u64) -> Self {
        Self {
            id,
            offset,
            size,
            checksum: HashValue::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncDirection {
    Push,
    Pull,
    Bidirectional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    SourceWins,
    DestinationWins,
    LatestWins,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, strum::EnumString, Default)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ReplicationMode {
    Sync,
    Async,
    SemiSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum RecoveryAction {
    ReplayJournal,
    IntegrityScan,
    RepairChunks,
    RebuildIndex,
    FullReconstruction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
    pub source: String,
    pub payload: Vec<u8>,
    pub severity: EventSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum EventKind {
    ArchiveCreated,
    ArchiveSealed,
    SnapshotCreated,
    SnapshotDeleted,
    ChunkStored,
    ChunkDeleted,
    SyncStarted,
    SyncCompleted,
    SyncFailed,
    Replicated,
    IntegrityFailure,
    RecoveryStarted,
    RecoveryCompleted,
    ConfigChanged,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum EventSeverity {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub timestamp: DateTime<Utc>,
    pub total_chunks: u64,
    pub total_size: u64,
    pub dedup_size: u64,
    pub compressed_size: u64,
    pub chunks_stored: u64,
    pub chunks_deleted: u64,
    pub read_ops: u64,
    pub write_ops: u64,
    pub sync_ops: u64,
    pub errors: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryBlock {
    pub data: Vec<u8>,
    pub size: usize,
    pub pool_id: usize,
}

impl MemoryBlock {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0u8; size],
            size,
            pool_id: 0,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.size]
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data[..self.size]
    }
}

#[derive(Debug, Clone)]
pub struct TreeVerificationResult {
    pub root_id: NodeId,
    pub nodes_checked: u64,
    pub nodes_failed: u64,
    pub chunks_checked: u64,
    pub chunks_failed: u64,
    pub integrity_proofs: Vec<IntegrityProof>,
    pub passed: bool,
}

impl TreeVerificationResult {
    pub fn new(root_id: NodeId) -> Self {
        Self {
            root_id,
            nodes_checked: 0,
            nodes_failed: 0,
            chunks_checked: 0,
            chunks_failed: 0,
            integrity_proofs: Vec::new(),
            passed: true,
        }
    }
}

pub struct TaskHandle<T> {
    pub id: TaskId,
    pub(crate) completed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) result: std::sync::Arc<std::sync::Mutex<Option<AegisResult<T>>>>,
}

impl<T: Send + 'static> TaskHandle<T> {
    pub fn new(id: TaskId) -> Self {
        Self {
            id,
            completed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            result: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn complete(&self, result: AegisResult<T>) {
        let mut res = self.result.lock().unwrap();
        *res = Some(result);
        self.completed.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_completed(&self) -> bool {
        self.completed.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn await_completion(&self) -> AegisResult<T>
    where
        T: Clone,
    {
        loop {
            if self.is_completed() {
                let mut res = self.result.lock().unwrap();
                if let Some(r) = res.take() {
                    return r;
                }
                return Err(super::error::AegisError::Internal("task failed".into()));
            }
            std::thread::yield_now();
        }
    }

    pub fn cancel(&self) -> AegisResult<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncResult {
    pub files_transferred: u64,
    pub bytes_transferred: u64,
    pub conflicts_resolved: u64,
    pub failed_files: Vec<String>,
    pub duration_seconds: f64,
}

impl SyncResult {
    pub fn new() -> Self {
        Self {
            files_transferred: 0,
            bytes_transferred: 0,
            conflicts_resolved: 0,
            failed_files: Vec::new(),
            duration_seconds: 0.0,
        }
    }
}



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    pub in_progress: bool,
    pub progress_percent: f64,
    pub current_file: Option<String>,
    pub bytes_so_far: u64,
    pub errors_so_far: u64,
}

impl SyncStatus {
    pub fn idle() -> Self {
        Self {
            in_progress: false,
            progress_percent: 0.0,
            current_file: None,
            bytes_so_far: 0,
            errors_so_far: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationTarget {
    pub name: String,
    pub endpoint: String,
    pub credentials: Option<CredentialsRef>,
    pub backend: StorageBackendKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationResult {
    pub target: String,
    pub chunks_replicated: u64,
    pub bytes_replicated: u64,
    pub duration_seconds: f64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    pub mode: ReplicationMode,
    pub targets: Vec<ReplicationTarget>,
    pub bandwidth_limit_bps: u64,
    pub verify_replicas: bool,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            mode: ReplicationMode::Async,
            targets: Vec::new(),
            bandwidth_limit_bps: 0,
            verify_replicas: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationStatus {
    pub active_jobs: usize,
    pub completed_jobs: usize,
    pub failed_jobs: usize,
    pub total_bytes_replicated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub action: RecoveryAction,
    pub success: bool,
    pub entries_replayed: u64,
    pub chunks_repaired: u64,
    pub errors_encountered: Vec<String>,
    pub duration_seconds: f64,
}

impl RecoveryReport {
    pub fn new(action: RecoveryAction) -> Self {
        Self {
            action,
            success: false,
            entries_replayed: 0,
            chunks_repaired: 0,
            errors_encountered: Vec::new(),
            duration_seconds: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryStatus {
    pub in_progress: bool,
    pub progress_percent: f64,
    pub current_action: Option<RecoveryAction>,
    pub last_report: Option<RecoveryReport>,
}

impl RecoveryStatus {
    pub fn idle() -> Self {
        Self {
            in_progress: false,
            progress_percent: 0.0,
            current_action: None,
            last_report: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Credentials {
    Password { username: String, password: String },
    Token { token: String },
    KeyPair { public_key: Vec<u8>, private_key: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct AuthToken {
    pub session_id: SessionId,
    pub identity: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub permissions: Vec<String>,
}

impl AuthToken {
    pub fn new(identity: &str, permissions: Vec<String>, ttl_hours: i64) -> Self {
        let now = chrono::Utc::now();
        Self {
            session_id: SessionId::new(),
            identity: identity.to_string(),
            issued_at: now,
            expires_at: now + chrono::Duration::hours(ttl_hours),
            permissions,
        }
    }

    pub fn is_expired(&self) -> bool {
        chrono::Utc::now() > self.expires_at
    }
}

#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    pub name: String,
    pub encryption: Option<EncryptionAlgorithm>,
    pub compression: CompressionAlgorithm,
    pub chunk_size: u64,
    pub dedup_enabled: bool,
    pub sealed: bool,
    pub passphrase: Option<String>,
}

impl ArchiveConfig {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            encryption: None,
            compression: CompressionAlgorithm::Zstd(3),
            chunk_size: 65536,
            dedup_enabled: true,
            sealed: false,
            passphrase: None,
        }
    }

    pub fn validate(&self) -> AegisResult<()> {
        if self.name.trim().is_empty() {
            return Err(AegisError::InvalidConfig("archive name must not be empty".into()));
        }
        if self.chunk_size < 4096 || self.chunk_size > 16 * 1024 * 1024 {
            return Err(AegisError::InvalidConfig(format!(
                "chunk size {} out of range [4096, 16777216]",
                self.chunk_size
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotDiff {
    pub added: Vec<NodeId>,
    pub modified: Vec<(NodeId, NodeId)>,
    pub deleted: Vec<NodeId>,
    pub unchanged: Vec<NodeId>,
    pub size_delta: i64,
}

impl SnapshotDiff {
    pub fn new() -> Self {
        Self {
            added: Vec::new(),
            modified: Vec::new(),
            deleted: Vec::new(),
            unchanged: Vec::new(),
            size_delta: 0,
        }
    }
}



#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub id: ManifestId,
    pub archive_id: ArchiveId,
    pub parent_manifest: Option<ManifestId>,
    pub root_node: NodeId,
    pub chunk_list: Vec<ChunkDescriptor>,
    pub total_size: u64,
    pub chunk_count: u64,
    pub created_at: DateTime<Utc>,
    pub checksum: HashValue,
    pub metadata: std::collections::HashMap<String, String>,
}
