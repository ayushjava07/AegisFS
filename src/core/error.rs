use thiserror::Error;

#[derive(Error, Debug)]
pub enum AegisError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("chunk not found: {0}")]
    ChunkNotFound(String),

    #[error("node not found: {0}")]
    NodeNotFound(String),

    #[error("archive not found: {0}")]
    ArchiveNotFound(String),

    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("integrity verification failed: {0}")]
    IntegrityVerificationFailed(String),

    #[error("encryption error: {0}")]
    EncryptionError(String),

    #[error("decryption error: {0}")]
    DecryptionError(String),

    #[error("compression error: {0}")]
    CompressionError(String),

    #[error("decompression error: {0}")]
    DecompressionError(String),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("deserialization error: {0}")]
    DeserializationError(String),

    #[error("storage backend error: {0}")]
    StorageBackendError(String),

    #[error("authentication error: {0}")]
    AuthenticationError(String),

    #[error("authorization error: {0}")]
    AuthorizationError(String),

    #[error("journal error: {0}")]
    JournalError(String),

    #[error("journal replay failed at sequence {sequence}: {message}")]
    JournalReplayFailed { sequence: u64, message: String },

    #[error("archive is sealed: {0}")]
    ArchiveSealed(String),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("plugin error: {0}")]
    PluginError(String),

    #[error("network error: {0}")]
    NetworkError(String),

    #[error("RPC error: code={code}, message={message}")]
    RpcError { code: i32, message: String },

    #[error("timeout")]
    Timeout,

    #[error("resource exhausted: {0}")]
    ResourceExhausted(String),

    #[error("not supported: {0}")]
    NotSupported(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("{0}")]
    Internal(String),

    #[error("recovery error: {0}")]
    RecoveryError(String),

    #[error("synchronization error: {0}")]
    SyncError(String),

    #[error("replication error: {0}")]
    ReplicationError(String),
}

pub type AegisResult<T> = Result<T, AegisError>;
