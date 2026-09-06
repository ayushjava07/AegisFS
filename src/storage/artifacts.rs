//! Content-addressable storage for large workflow payloads, task artifacts, and outputs.
//!
//! Blobs are addressed by the cryptographic SHA-256 hash of their contents,
//! enabling automatic deduplication and end-to-end integrity verification.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Errors encountered in artifact storage operations.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// An underlying filesystem I/O error occurred.
    #[error("artifact I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A payload failed checksum verification.
    #[error("integrity verification failed: expected sha256 '{expected}', calculated '{actual}'")]
    IntegrityMismatch {
        /// Expected SHA-256 digest in lowercase hexadecimal.
        expected: String,
        /// Actual calculated digest.
        actual: String,
    },

    /// An artifact exceeds maximum allowed size.
    #[error("artifact size ({size} bytes) exceeds maximum limit ({limit} bytes)")]
    SizeLimitExceeded {
        /// Size of the payload.
        size: usize,
        /// Maximum allowed size.
        limit: usize,
    },

    /// The requested artifact was not found.
    #[error("artifact '{0}' not found")]
    NotFound(String),
}

/// Metadata describing a stored artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDescriptor {
    /// Cryptographic identifier (SHA-256 digest).
    pub id: String,
    /// Owning tenant.
    pub tenant: String,
    /// Logical filename or artifact label.
    pub name: String,
    /// MIME content-type (e.g. `application/json`, `application/octet-stream`).
    pub content_type: String,
    /// Exact byte size of the payload.
    pub size_bytes: usize,
    /// Epoch timestamp when the artifact was persisted.
    pub created_at_ms: i64,
}

/// Abstract interface for storing and retrieving content-addressable blobs.
pub trait ArtifactStore: Send + Sync {
    /// Stores an artifact blob, returning its descriptor.
    fn put(
        &self,
        tenant: &str,
        name: &str,
        data: &[u8],
        content_type: &str,
        now_ms: i64,
    ) -> Result<ArtifactDescriptor, ArtifactError>;

    /// Fetches the raw content bytes for an artifact by its content ID.
    fn get(&self, artifact_id: &str) -> Result<Option<Vec<u8>>, ArtifactError>;

    /// Returns the metadata descriptor for an artifact by ID.
    fn describe(&self, artifact_id: &str) -> Result<Option<ArtifactDescriptor>, ArtifactError>;

    /// Deletes an artifact by ID, returning whether the artifact existed.
    fn delete(&self, artifact_id: &str) -> Result<bool, ArtifactError>;

    /// Lists all artifact descriptors associated with a tenant.
    fn list_by_tenant(&self, tenant: &str) -> Result<Vec<ArtifactDescriptor>, ArtifactError>;
}

/// Thread-safe in-memory content-addressable artifact store.
#[derive(Debug, Default)]
pub struct MemoryArtifactStore {
    blobs: RwLock<HashMap<String, Vec<u8>>>,
    descriptors: RwLock<HashMap<String, ArtifactDescriptor>>,
}

impl MemoryArtifactStore {
    /// Creates a new empty in-memory artifact store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ArtifactStore for MemoryArtifactStore {
    fn put(
        &self,
        tenant: &str,
        name: &str,
        data: &[u8],
        content_type: &str,
        now_ms: i64,
    ) -> Result<ArtifactDescriptor, ArtifactError> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let id = hex::encode(hasher.finalize());

        let descriptor = ArtifactDescriptor {
            id: id.clone(),
            tenant: tenant.to_owned(),
            name: name.to_owned(),
            content_type: content_type.to_owned(),
            size_bytes: data.len(),
            created_at_ms: now_ms,
        };

        self.blobs.write().insert(id.clone(), data.to_vec());
        self.descriptors.write().insert(id, descriptor.clone());

        Ok(descriptor)
    }

    fn get(&self, artifact_id: &str) -> Result<Option<Vec<u8>>, ArtifactError> {
        Ok(self.blobs.read().get(artifact_id).cloned())
    }

    fn describe(&self, artifact_id: &str) -> Result<Option<ArtifactDescriptor>, ArtifactError> {
        Ok(self.descriptors.read().get(artifact_id).cloned())
    }

    fn delete(&self, artifact_id: &str) -> Result<bool, ArtifactError> {
        let mut blobs = self.blobs.write();
        let mut descriptors = self.descriptors.write();
        let removed = blobs.remove(artifact_id).is_some();
        descriptors.remove(artifact_id);
        Ok(removed)
    }

    fn list_by_tenant(&self, tenant: &str) -> Result<Vec<ArtifactDescriptor>, ArtifactError> {
        let descriptors = self.descriptors.read();
        let list = descriptors
            .values()
            .filter(|d| d.tenant == tenant)
            .cloned()
            .collect();
        Ok(list)
    }
}

/// Local filesystem content-addressable artifact store with two-level prefix sharding.
#[derive(Debug)]
pub struct DiskArtifactStore {
    root_dir: PathBuf,
    metadata: RwLock<HashMap<String, ArtifactDescriptor>>,
}

impl DiskArtifactStore {
    /// Opens or initializes a disk artifact store at `root_dir`.
    pub fn open(root_dir: impl AsRef<Path>) -> Result<Self, ArtifactError> {
        let path = root_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            root_dir: path,
            metadata: RwLock::new(HashMap::new()),
        })
    }

    fn blob_path(&self, id: &str) -> PathBuf {
        // Shard by first 2 and second 2 hex characters: root/aa/bb/aabb...
        let (prefix1, remainder) = id.split_at(2.min(id.len()));
        let (prefix2, _) = remainder.split_at(2.min(remainder.len()));
        self.root_dir.join(prefix1).join(prefix2).join(id)
    }
}

impl ArtifactStore for DiskArtifactStore {
    fn put(
        &self,
        tenant: &str,
        name: &str,
        data: &[u8],
        content_type: &str,
        now_ms: i64,
    ) -> Result<ArtifactDescriptor, ArtifactError> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let id = hex::encode(hasher.finalize());

        let target_path = self.blob_path(&id);
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&target_path)?;
        file.write_all(data)?;
        file.flush()?;

        let descriptor = ArtifactDescriptor {
            id: id.clone(),
            tenant: tenant.to_owned(),
            name: name.to_owned(),
            content_type: content_type.to_owned(),
            size_bytes: data.len(),
            created_at_ms: now_ms,
        };

        self.metadata.write().insert(id, descriptor.clone());
        Ok(descriptor)
    }

    fn get(&self, artifact_id: &str) -> Result<Option<Vec<u8>>, ArtifactError> {
        let target_path = self.blob_path(artifact_id);
        if !target_path.exists() {
            return Ok(None);
        }

        let mut file = File::open(&target_path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        // Verify integrity
        let mut hasher = Sha256::new();
        hasher.update(&buf);
        let actual = hex::encode(hasher.finalize());
        if actual != artifact_id {
            return Err(ArtifactError::IntegrityMismatch {
                expected: artifact_id.to_owned(),
                actual,
            });
        }

        Ok(Some(buf))
    }

    fn describe(&self, artifact_id: &str) -> Result<Option<ArtifactDescriptor>, ArtifactError> {
        Ok(self.metadata.read().get(artifact_id).cloned())
    }

    fn delete(&self, artifact_id: &str) -> Result<bool, ArtifactError> {
        let target_path = self.blob_path(artifact_id);
        self.metadata.write().remove(artifact_id);
        if target_path.exists() {
            std::fs::remove_file(&target_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn list_by_tenant(&self, tenant: &str) -> Result<Vec<ArtifactDescriptor>, ArtifactError> {
        let list = self
            .metadata
            .read()
            .values()
            .filter(|d| d.tenant == tenant)
            .cloned()
            .collect();
        Ok(list)
    }
}
