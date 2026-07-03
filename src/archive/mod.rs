use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;
use crate::core::types::*;

struct ArchiveEntry {
    archive: Archive,
    #[allow(dead_code)]
    config: ArchiveConfig,
    deleted: bool,
    fs: Arc<dyn VirtualFileSystem>,
    snapshot: Arc<dyn SnapshotManager>,
    manifest: Arc<dyn ManifestStore>,
    integrity: Arc<dyn IntegrityVerifier>,
}

struct ArcVfs(Arc<dyn VirtualFileSystem>);

impl VirtualFileSystem for ArcVfs {
    fn create_node(
        &self,
        parent: &NodeId,
        name: &str,
        kind: NodeKind,
    ) -> BoxFuture<'_, AegisResult<NodeId>> {
        self.0.create_node(parent, name, kind)
    }
    fn delete_node(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
        self.0.delete_node(node_id)
    }
    fn read_node(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Node>> {
        self.0.read_node(node_id)
    }
    fn write_node(&self, node_id: &NodeId, data: bytes::Bytes) -> BoxFuture<'_, AegisResult<()>> {
        self.0.write_node(node_id, data)
    }
    fn read_file(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<bytes::Bytes>> {
        self.0.read_file(node_id)
    }
    fn list_directory(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
        self.0.list_directory(node_id)
    }
    fn resolve_path(&self, path: &str) -> BoxFuture<'_, AegisResult<NodeId>> {
        self.0.resolve_path(path)
    }
    fn exists(&self, path: &str) -> BoxFuture<'_, AegisResult<bool>> {
        self.0.exists(path)
    }
}

struct ArcSnapshot(Arc<dyn SnapshotManager>);

impl SnapshotManager for ArcSnapshot {
    fn create(
        &self,
        labels: std::collections::HashMap<String, String>,
    ) -> BoxFuture<'_, AegisResult<SnapshotId>> {
        self.0.create(labels)
    }
    fn restore(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>> {
        self.0.restore(id)
    }
    fn list(&self) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>> {
        self.0.list()
    }
    fn delete(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>> {
        self.0.delete(id)
    }
    fn diff(
        &self,
        base: &SnapshotId,
        target: &SnapshotId,
    ) -> BoxFuture<'_, AegisResult<SnapshotDiff>> {
        self.0.diff(base, target)
    }
}

struct ArcManifest(Arc<dyn ManifestStore>);

impl ManifestStore for ArcManifest {
    fn put_manifest(&self, manifest: Manifest) -> BoxFuture<'_, AegisResult<ManifestId>> {
        self.0.put_manifest(manifest)
    }
    fn get_manifest(&self, id: &ManifestId) -> BoxFuture<'_, AegisResult<Manifest>> {
        self.0.get_manifest(id)
    }
    fn delete_manifest(&self, id: &ManifestId) -> BoxFuture<'_, AegisResult<()>> {
        self.0.delete_manifest(id)
    }
    fn list_manifests(&self) -> BoxFuture<'_, AegisResult<Vec<ManifestId>>> {
        self.0.list_manifests()
    }
    fn latest_manifest(&self, archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Manifest>> {
        self.0.latest_manifest(archive_id)
    }
}

struct ArcIntegrity(Arc<dyn IntegrityVerifier>);

impl IntegrityVerifier for ArcIntegrity {
    fn verify_chunk(&self, chunk: &Chunk) -> AegisResult<IntegrityProof> {
        self.0.verify_chunk(chunk)
    }
    fn verify_manifest(&self, manifest: &Manifest) -> BoxFuture<'_, AegisResult<bool>> {
        self.0.verify_manifest(manifest)
    }
    fn full_scan(&self) -> BoxFuture<'_, AegisResult<Vec<IntegrityProof>>> {
        self.0.full_scan()
    }
    fn verify_tree(&self, root_id: &NodeId) -> BoxFuture<'_, AegisResult<TreeVerificationResult>> {
        self.0.verify_tree(root_id)
    }
}

pub struct ArchiveManagerImpl {
    config: crate::config::AegisConfig,
    archives: DashMap<ArchiveId, ArchiveEntry>,
}

impl ArchiveManagerImpl {
    #[allow(deprecated)]
    pub fn new() -> Self {
        Self::new_in_memory()
    }

    pub fn new_with_config(config: crate::config::AegisConfig) -> Self {
        Self {
            config,
            archives: DashMap::new(),
        }
    }

    pub fn new_in_memory() -> Self {
        let mut config = crate::config::AegisConfig::default();
        config.storage.kind = StorageBackendKind::Memory;
        Self {
            config,
            archives: DashMap::new(),
        }
    }
}

impl Default for ArchiveManagerImpl {
    fn default() -> Self {
        Self::new_in_memory()
    }
}

impl ArchiveManager for ArchiveManagerImpl {
    fn create_archive(
        &self,
        name: &str,
        config: ArchiveConfig,
    ) -> BoxFuture<'_, AegisResult<ArchiveId>> {
        let name = name.to_string();
        let global_config = self.config.clone();
        Box::pin(async move {
            let mut config = config;
            config.name.clone_from(&name);
            if config.name.trim().is_empty() {
                return Err(AegisError::InvalidConfig(
                    "archive name must not be empty".into(),
                ));
            }
            if config.chunk_size < 4096 || config.chunk_size > 16 * 1024 * 1024 {
                return Err(AegisError::InvalidConfig(format!(
                    "chunk size {} out of range [4096, 16777216]",
                    config.chunk_size
                )));
            }

            if self
                .archives
                .iter()
                .any(|e| e.archive.name == name && !e.deleted)
            {
                return Err(AegisError::AlreadyExists(format!(
                    "archive with name '{}' already exists",
                    name
                )));
            }

            let id = ArchiveId::new();

            // Set up encryption provider
            let encryption_provider: Option<Arc<dyn EncryptionProvider>> =
                if let Some(algo) = config.encryption {
                    let passphrase = config.passphrase.as_deref().unwrap_or("default_passphrase");
                    let salt = id.as_uuid().as_bytes();
                    let derivation =
                        crate::crypto::KeyDerivation::new("AegisFS Archive Key Derivation");
                    let key = derivation.derive_key(passphrase, salt);
                    let key_id = id.as_uuid().as_bytes().to_vec();

                    let provider: Arc<dyn EncryptionProvider> = match algo {
                        EncryptionAlgorithm::Aes256Gcm => {
                            #[cfg(feature = "aes-encryption")]
                            {
                                Arc::new(crate::crypto::Aes256GcmProvider::new(key, key_id))
                            }
                            #[cfg(not(feature = "aes-encryption"))]
                            {
                                return Err(AegisError::EncryptionError(
                                    "AES-256-GCM feature not enabled".into(),
                                ));
                            }
                        }
                        EncryptionAlgorithm::ChaCha20Poly1305 => {
                            #[cfg(feature = "chacha-encryption")]
                            {
                                Arc::new(crate::crypto::ChaCha20Poly1305Provider::new(key, key_id))
                            }
                            #[cfg(not(feature = "chacha-encryption"))]
                            {
                                return Err(AegisError::EncryptionError(
                                    "ChaCha20-Poly1305 feature not enabled".into(),
                                ));
                            }
                        }
                    };
                    Some(provider)
                } else {
                    None
                };

            // Set up compression provider
            let compression_provider: Option<Arc<dyn CompressionProvider>> =
                match config.compression {
                    CompressionAlgorithm::None => None,
                    CompressionAlgorithm::Zstd(level) => {
                        Some(Arc::new(crate::compression::ZstdCompression::new(level)))
                    }
                    CompressionAlgorithm::Lz4 => {
                        Some(Arc::new(crate::compression::Lz4Compression::new()))
                    }
                };

            // Set up underlying chunk storage (local disk or memory)
            let base_storage: Arc<dyn ChunkStorage> =
                if global_config.storage.kind == StorageBackendKind::Local {
                    let storage_path = global_config
                        .storage
                        .path
                        .clone()
                        .unwrap_or_else(|| "/var/lib/aegisfs/data".to_string());
                    let base_path = std::path::PathBuf::from(storage_path).join(id.to_string());
                    Arc::new(DiskChunkStorage::new(base_path)?)
                } else {
                    Arc::new(MemoryChunkStorage::new())
                };

            // Wrap with compression/encryption layer
            let chunk_storage: Arc<dyn ChunkStorage> =
                Arc::new(EncryptedCompressedChunkStorage::new(
                    base_storage,
                    encryption_provider.clone(),
                    compression_provider,
                ));

            let manifest_store = Arc::new(crate::manifest::MemoryManifestStore::new());
            let metadata = Arc::new(crate::metadata::MemoryMetadataIndex::new());

            // Initialize the root node in the metadata store
            let now = chrono::Utc::now();
            let root = Node {
                id: NodeId::root(),
                name: String::from("/"),
                kind: NodeKind::Directory,
                size: 0,
                mode: NodePermissions::default_for("aegisfs"),
                created_at: now,
                modified_at: now,
                content_hash: None,
                metadata: NodeMetadata::default(),
            };
            metadata.put_node(root).await?;

            let chunker: Arc<dyn Chunker> =
                Arc::new(crate::chunking::FixedSizeChunker::new(config.chunk_size));

            let dedup_index: Arc<dyn DedupIndex> = Arc::new(crate::dedup::MemoryDedupIndex::new());
            let dedup_engine = Arc::new(crate::dedup::DedupEngine::new(
                dedup_index,
                chunker.clone(),
                chunk_storage.clone(),
            ));

            let fs: Arc<dyn VirtualFileSystem> =
                Arc::new(crate::filesystem::VirtualFileSystemImpl::new(
                    metadata.clone(),
                    chunk_storage.clone(),
                    dedup_engine.clone(),
                ));

            let snapshot_store: Arc<dyn SnapshotStore> = Arc::new(MemorySnapshotStore::new());

            let snapshot: Arc<dyn SnapshotManager> =
                Arc::new(crate::snapshot::SnapshotManagerImpl::new(
                    metadata.clone(),
                    manifest_store.clone(),
                    snapshot_store,
                    id,
                    crate::snapshot::SnapshotPolicy::default(),
                ));

            let integrity: Arc<dyn IntegrityVerifier> =
                Arc::new(crate::verification::IntegrityVerifierImpl::new(
                    chunk_storage.clone(),
                    metadata.clone(),
                ));

            let now = chrono::Utc::now();
            let archive = Archive {
                id,
                name: name.clone(),
                manifest: ManifestRef {
                    id: ManifestId::nil(),
                    root_node: NodeId::nil(),
                    chunk_count: 0,
                    total_size: 0,
                    created_at: now,
                },
                encrypted: config.encryption.is_some(),
                compression: config.compression,
                created_at: now,
                sealed: config.sealed,
            };

            self.archives.insert(
                id,
                ArchiveEntry {
                    archive,
                    config,
                    deleted: false,
                    fs,
                    snapshot,
                    manifest: manifest_store,
                    integrity,
                },
            );

            Ok(id)
        })
    }

    fn open_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<Box<dyn ArchiveHandle>>> {
        let id = *id;
        Box::pin(async move {
            let entry = self
                .archives
                .get(&id)
                .ok_or_else(|| AegisError::ArchiveNotFound(id.to_string()))?;

            if entry.deleted {
                return Err(AegisError::ArchiveNotFound(id.to_string()));
            }

            let handle: Box<dyn ArchiveHandle> = Box::new(ArchiveHandleImpl {
                id,
                sealed: entry.archive.sealed,
                fs: entry.fs.clone(),
                snapshot: entry.snapshot.clone(),
                manifest: entry.manifest.clone(),
                integrity: entry.integrity.clone(),
                closed: AtomicBool::new(false),
            });

            Ok(handle)
        })
    }

    fn delete_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<()>> {
        let id = *id;
        Box::pin(async move {
            let mut entry = self
                .archives
                .get_mut(&id)
                .ok_or_else(|| AegisError::ArchiveNotFound(id.to_string()))?;

            if entry.deleted {
                return Err(AegisError::ArchiveNotFound(id.to_string()));
            }

            entry.deleted = true;
            Ok(())
        })
    }

    fn seal_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<()>> {
        let id = *id;
        Box::pin(async move {
            let mut entry = self
                .archives
                .get_mut(&id)
                .ok_or_else(|| AegisError::ArchiveNotFound(id.to_string()))?;

            if entry.deleted {
                return Err(AegisError::ArchiveNotFound(id.to_string()));
            }

            if entry.archive.sealed {
                return Err(AegisError::ArchiveSealed(format!(
                    "archive {} is already sealed",
                    id
                )));
            }

            entry.archive.sealed = true;
            Ok(())
        })
    }

    fn list_archives(&self) -> BoxFuture<'_, AegisResult<Vec<Archive>>> {
        Box::pin(async move {
            let archives: Vec<Archive> = self
                .archives
                .iter()
                .filter(|e| !e.deleted)
                .map(|e| e.archive.clone())
                .collect();
            Ok(archives)
        })
    }

    fn get_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<Archive>> {
        let id = *id;
        Box::pin(async move {
            let entry = self
                .archives
                .get(&id)
                .ok_or_else(|| AegisError::ArchiveNotFound(id.to_string()))?;

            if entry.deleted {
                return Err(AegisError::ArchiveNotFound(id.to_string()));
            }

            Ok(entry.archive.clone())
        })
    }
}

pub struct ArchiveHandleImpl {
    id: ArchiveId,
    sealed: bool,
    fs: Arc<dyn VirtualFileSystem>,
    snapshot: Arc<dyn SnapshotManager>,
    manifest: Arc<dyn ManifestStore>,
    integrity: Arc<dyn IntegrityVerifier>,
    closed: AtomicBool,
}

impl ArchiveHandleImpl {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed
    }
}

impl ArchiveHandle for ArchiveHandleImpl {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn id(&self) -> ArchiveId {
        self.id
    }

    fn filesystem(&self) -> Box<dyn VirtualFileSystem> {
        Box::new(ArcVfs(self.fs.clone()))
    }

    fn snapshot(&self) -> Box<dyn SnapshotManager> {
        Box::new(ArcSnapshot(self.snapshot.clone()))
    }

    fn manifest(&self) -> Box<dyn ManifestStore> {
        Box::new(ArcManifest(self.manifest.clone()))
    }

    fn integrity(&self) -> Box<dyn IntegrityVerifier> {
        Box::new(ArcIntegrity(self.integrity.clone()))
    }

    fn close(&self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        })
    }
}

struct MemoryChunkStorage {
    chunks: std::sync::Mutex<std::collections::HashMap<ChunkId, Chunk>>,
}

impl MemoryChunkStorage {
    fn new() -> Self {
        Self {
            chunks: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl ChunkStorage for MemoryChunkStorage {
    fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
        let id = chunk.id;
        let mut guard = self.chunks.lock().unwrap();
        guard.insert(id, chunk);
        Box::pin(async move { Ok(id) })
    }
    fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
        let id = *id;
        let guard = self.chunks.lock().unwrap();
        let result = guard
            .get(&id)
            .cloned()
            .ok_or_else(|| AegisError::ChunkNotFound(id.to_string()));
        Box::pin(async move { result })
    }
    fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
        let id = *id;
        let mut guard = self.chunks.lock().unwrap();
        guard.remove(&id);
        Box::pin(async move { Ok(()) })
    }
    fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
        let id = *id;
        let guard = self.chunks.lock().unwrap();
        let exists = guard.contains_key(&id);
        Box::pin(async move { Ok(exists) })
    }
    fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
        let guard = self.chunks.lock().unwrap();
        let ids: Vec<ChunkId> = guard.keys().copied().collect();
        Box::pin(async move { Ok(ids) })
    }
    fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
        let guard = self.chunks.lock().unwrap();
        let total: u64 = guard.values().map(|c| c.size).sum();
        Box::pin(async move { Ok(total) })
    }
    fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
        let guard = self.chunks.lock().unwrap();
        let count = guard.len() as u64;
        Box::pin(async move { Ok(count) })
    }
}

pub struct DiskChunkStorage {
    base_path: std::path::PathBuf,
}

impl DiskChunkStorage {
    pub fn new<P: Into<std::path::PathBuf>>(base_path: P) -> AegisResult<Self> {
        let base_path = base_path.into();
        std::fs::create_dir_all(&base_path)?;
        Ok(Self { base_path })
    }

    fn chunk_path(&self, id: &ChunkId) -> std::path::PathBuf {
        let id_str = id.to_string();
        let prefix = if id_str.len() >= 2 {
            &id_str[0..2]
        } else {
            "xx"
        };
        self.base_path.join(prefix).join(id_str)
    }
}

impl ChunkStorage for DiskChunkStorage {
    fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
        let id = chunk.id;
        let path = self.chunk_path(&id);
        Box::pin(async move {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            let temp_path = path.with_extension("tmp");
            tokio::fs::write(&temp_path, &chunk.data).await?;

            if let Err(e) = tokio::fs::rename(&temp_path, &path).await {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(AegisError::Io(e));
            }

            Ok(id)
        })
    }

    fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
        let id = *id;
        let path = self.chunk_path(&id);
        Box::pin(async move {
            let data = tokio::fs::read(&path).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    AegisError::ChunkNotFound(id.to_string())
                } else {
                    AegisError::Io(e)
                }
            })?;
            Ok(Chunk::new(id, bytes::Bytes::from(data)))
        })
    }

    fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
        let id = *id;
        let path = self.chunk_path(&id);
        Box::pin(async move {
            tokio::fs::remove_file(&path).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    AegisError::ChunkNotFound(id.to_string())
                } else {
                    AegisError::Io(e)
                }
            })?;
            Ok(())
        })
    }

    fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
        let id = *id;
        let path = self.chunk_path(&id);
        Box::pin(async move { Ok(tokio::fs::metadata(&path).await.is_ok()) })
    }

    fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
        let base = self.base_path.clone();
        Box::pin(async move {
            let mut ids = Vec::new();
            if tokio::fs::metadata(&base).await.is_err() {
                return Ok(ids);
            }

            let mut entries = tokio::fs::read_dir(&base).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_dir() {
                    let mut sub_entries = tokio::fs::read_dir(&path).await?;
                    while let Some(sub_entry) = sub_entries.next_entry().await? {
                        let file_name = sub_entry.file_name();
                        let file_str = file_name.to_string_lossy();
                        if let Ok(id) = file_str.parse::<ChunkId>() {
                            ids.push(id);
                        }
                    }
                }
            }
            Ok(ids)
        })
    }

    fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
        let base = self.base_path.clone();
        Box::pin(async move {
            let mut total = 0;
            if tokio::fs::metadata(&base).await.is_err() {
                return Ok(0);
            }

            let mut entries = tokio::fs::read_dir(&base).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_dir() {
                    let mut sub_entries = tokio::fs::read_dir(&path).await?;
                    while let Some(sub_entry) = sub_entries.next_entry().await? {
                        let metadata = sub_entry.metadata().await?;
                        total += metadata.len();
                    }
                }
            }
            Ok(total)
        })
    }

    fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
        let base = self.base_path.clone();
        Box::pin(async move {
            let mut count = 0;
            if tokio::fs::metadata(&base).await.is_err() {
                return Ok(0);
            }

            let mut entries = tokio::fs::read_dir(&base).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_dir() {
                    let mut sub_entries = tokio::fs::read_dir(&path).await?;
                    while sub_entries.next_entry().await?.is_some() {
                        count += 1;
                    }
                }
            }
            Ok(count)
        })
    }
}

pub struct EncryptedCompressedChunkStorage {
    underlying: Arc<dyn ChunkStorage>,
    encryption: Option<Arc<dyn EncryptionProvider>>,
    compression: Option<Arc<dyn CompressionProvider>>,
}

impl EncryptedCompressedChunkStorage {
    pub fn new(
        underlying: Arc<dyn ChunkStorage>,
        encryption: Option<Arc<dyn EncryptionProvider>>,
        compression: Option<Arc<dyn CompressionProvider>>,
    ) -> Self {
        Self {
            underlying,
            encryption,
            compression,
        }
    }
}

impl ChunkStorage for EncryptedCompressedChunkStorage {
    fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
        let underlying = self.underlying.clone();
        let encryption = self.encryption.clone();
        let compression = self.compression.clone();
        let id = chunk.id;
        let data = chunk.data;
        Box::pin(async move {
            let compressed_data = if let Some(ref comp) = compression {
                comp.compress(&data)?
            } else {
                data.to_vec()
            };

            let encrypted_data = if let Some(ref enc) = encryption {
                enc.encrypt(&compressed_data)?
            } else {
                compressed_data
            };

            let compressed_size = if compression.is_some() {
                Some(encrypted_data.len() as u64)
            } else {
                None
            };
            let compression_algorithm = compression.as_ref().map(|c| c.algorithm());
            let encryption_algorithm = encryption.as_ref().map(|e| e.algorithm());

            let mut chunk_to_store = Chunk::new(id, bytes::Bytes::from(encrypted_data));
            chunk_to_store.compressed_size = compressed_size;
            chunk_to_store.compression_algorithm = compression_algorithm;
            chunk_to_store.encryption_algorithm = encryption_algorithm;
            chunk_to_store.checksum = chunk.checksum;
            underlying.store_chunk(chunk_to_store).await
        })
    }

    fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
        let underlying = self.underlying.clone();
        let encryption = self.encryption.clone();
        let compression = self.compression.clone();
        let id = *id;
        Box::pin(async move {
            let stored_chunk = underlying.read_chunk(&id).await?;

            let decrypted_data = if let Some(ref enc) = encryption {
                if let Some(algo) = stored_chunk.encryption_algorithm {
                    if enc.algorithm() != algo {
                        return Err(AegisError::DecryptionError(format!(
                            "mismatched encryption algorithm: expected {}, got {}",
                            algo,
                            enc.algorithm()
                        )));
                    }
                }
                enc.decrypt(&stored_chunk.data)?
            } else {
                stored_chunk.data.to_vec()
            };

            let decompressed_data = if let Some(ref comp) = compression {
                if let Some(algo) = stored_chunk.compression_algorithm {
                    if comp.algorithm() != algo {
                        return Err(AegisError::DecompressionError(format!(
                            "mismatched compression algorithm: expected {:?}, got {:?}",
                            algo,
                            comp.algorithm()
                        )));
                    }
                }
                comp.decompress(&decrypted_data)?
            } else {
                decrypted_data
            };

            let mut returned_chunk = Chunk::new(id, bytes::Bytes::from(decompressed_data));
            returned_chunk.checksum = stored_chunk.checksum;
            returned_chunk.encryption_algorithm = encryption.as_ref().map(|e| e.algorithm());
            returned_chunk.compression_algorithm = compression.as_ref().map(|c| c.algorithm());
            Ok(returned_chunk)
        })
    }

    fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
        self.underlying.delete_chunk(id)
    }

    fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
        self.underlying.has_chunk(id)
    }

    fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
        self.underlying.list_chunks()
    }

    fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
        self.underlying.total_size()
    }

    fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
        self.underlying.chunk_count()
    }
}

struct MemorySnapshotStore {
    snapshots: DashMap<SnapshotId, Snapshot>,
    archive_index: DashMap<ArchiveId, Vec<SnapshotId>>,
}

impl MemorySnapshotStore {
    fn new() -> Self {
        Self {
            snapshots: DashMap::new(),
            archive_index: DashMap::new(),
        }
    }
}

impl SnapshotStore for MemorySnapshotStore {
    fn create_snapshot(&self, snapshot: Snapshot) -> BoxFuture<'_, AegisResult<SnapshotId>> {
        let id = snapshot.id;
        let archive_id = snapshot.archive_id;
        self.snapshots.insert(id, snapshot);
        self.archive_index.entry(archive_id).or_default().push(id);
        Box::pin(async move { Ok(id) })
    }

    fn get_snapshot(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<Snapshot>> {
        let id = *id;
        Box::pin(async move {
            self.snapshots
                .get(&id)
                .map(|r| r.clone())
                .ok_or_else(|| AegisError::SnapshotNotFound(id.to_string()))
        })
    }

    fn delete_snapshot(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>> {
        let id = *id;
        Box::pin(async move {
            let entry = self
                .snapshots
                .remove(&id)
                .ok_or_else(|| AegisError::SnapshotNotFound(id.to_string()))?;
            let archive_id = entry.1.archive_id;
            if let Some(mut ids) = self.archive_index.get_mut(&archive_id) {
                ids.retain(|i| *i != id);
            }
            Ok(())
        })
    }

    fn list_snapshots(&self, archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>> {
        let archive_id = *archive_id;
        Box::pin(async move {
            let snapshots: Vec<Snapshot> = self
                .archive_index
                .get(&archive_id)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| self.snapshots.get(id).map(|r| r.clone()))
                        .collect()
                })
                .unwrap_or_default();
            Ok(snapshots)
        })
    }

    fn latest_snapshot(&self, archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Snapshot>> {
        let archive_id = *archive_id;
        Box::pin(async move {
            let latest = self
                .archive_index
                .get(&archive_id)
                .and_then(|ids| {
                    ids.iter()
                        .filter_map(|id| self.snapshots.get(id))
                        .max_by_key(|s| s.timestamp)
                        .map(|r| r.clone())
                })
                .ok_or_else(|| AegisError::SnapshotNotFound(archive_id.to_string()));
            latest
        })
    }

    fn snapshot_chain(
        &self,
        snapshot_id: &SnapshotId,
    ) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>> {
        let snapshot_id = *snapshot_id;
        Box::pin(async move {
            let mut chain = Vec::new();
            let mut current = Some(snapshot_id);
            while let Some(id) = current {
                if let Some(snapshot) = self.snapshots.get(&id) {
                    chain.push(snapshot.clone());
                    current = snapshot.parent;
                } else {
                    break;
                }
            }
            Ok(chain)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config(name: &str) -> ArchiveConfig {
        ArchiveConfig {
            name: name.to_string(),
            encryption: None,
            compression: CompressionAlgorithm::Zstd(3),
            chunk_size: 64 * 1024,
            dedup_enabled: true,
            sealed: false,
            passphrase: None,
        }
    }

    #[test]
    fn test_config_validation_valid() {
        let config = default_config("test-archive");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validation_empty_name() {
        let config = ArchiveConfig::new("");
        assert!(matches!(
            config.validate(),
            Err(AegisError::InvalidConfig(_))
        ));
    }

    #[test]
    fn test_config_validation_chunk_size_too_small() {
        let mut config = default_config("test");
        config.chunk_size = 0;
        assert!(matches!(
            config.validate(),
            Err(AegisError::InvalidConfig(_))
        ));
    }

    #[test]
    fn test_config_validation_chunk_size_too_large() {
        let mut config = default_config("test");
        config.chunk_size = 32 * 1024 * 1024;
        assert!(matches!(
            config.validate(),
            Err(AegisError::InvalidConfig(_))
        ));
    }

    #[test]
    fn test_config_new_defaults() {
        let config = ArchiveConfig::new("my-archive");
        assert_eq!(config.name, "my-archive");
        assert!(config.encryption.is_none());
        assert_eq!(config.compression, CompressionAlgorithm::Zstd(3));
        assert_eq!(config.chunk_size, 65536);
        assert!(config.dedup_enabled);
        assert!(!config.sealed);
    }

    #[test]
    fn test_create_archive_default_config() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("test-default");
        let id = rt
            .block_on(manager.create_archive("test-default", config))
            .unwrap();
        assert_ne!(id, ArchiveId::nil());
    }

    #[test]
    fn test_create_archive_with_encryption() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = ArchiveConfig {
            name: "encrypted-archive".into(),
            encryption: Some(EncryptionAlgorithm::Aes256Gcm),
            compression: CompressionAlgorithm::Lz4,
            chunk_size: 128 * 1024,
            dedup_enabled: false,
            sealed: false,
            passphrase: Some("test_password".into()),
        };
        let id = rt
            .block_on(manager.create_archive("encrypted-archive", config))
            .unwrap();
        assert_ne!(id, ArchiveId::nil());

        let archive = rt.block_on(manager.get_archive(&id)).unwrap();
        assert!(archive.encrypted);
        assert_eq!(archive.compression, CompressionAlgorithm::Lz4);
    }

    #[test]
    fn test_create_archive_sealed() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = ArchiveConfig {
            name: "sealed-archive".into(),
            encryption: None,
            compression: CompressionAlgorithm::Zstd(1),
            chunk_size: 4096,
            dedup_enabled: true,
            sealed: true,
            passphrase: None,
        };
        let id = rt
            .block_on(manager.create_archive("sealed-archive", config))
            .unwrap();
        let archive = rt.block_on(manager.get_archive(&id)).unwrap();
        assert!(archive.sealed);
    }

    #[test]
    fn test_open_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("openable");
        let id = rt
            .block_on(manager.create_archive("openable", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        assert_eq!(handle.id(), id);
        assert!(!handle.is_closed());
    }

    #[test]
    fn test_open_nonexistent_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let id = ArchiveId::new();
        let result = rt.block_on(manager.open_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_delete_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("deletable");
        let id = rt
            .block_on(manager.create_archive("deletable", config))
            .unwrap();
        rt.block_on(manager.delete_archive(&id)).unwrap();
        let get_result = rt.block_on(manager.get_archive(&id));
        assert!(matches!(get_result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_delete_nonexistent_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let id = ArchiveId::new();
        let result = rt.block_on(manager.delete_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_delete_already_deleted_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("double-delete");
        let id = rt
            .block_on(manager.create_archive("double-delete", config))
            .unwrap();
        rt.block_on(manager.delete_archive(&id)).unwrap();
        let result = rt.block_on(manager.delete_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_seal_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("sealable");
        let id = rt
            .block_on(manager.create_archive("sealable", config))
            .unwrap();
        rt.block_on(manager.seal_archive(&id)).unwrap();
        let archive = rt.block_on(manager.get_archive(&id)).unwrap();
        assert!(archive.sealed);
    }

    #[test]
    fn test_seal_already_sealed_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = ArchiveConfig {
            name: "pre-sealed".into(),
            encryption: None,
            compression: CompressionAlgorithm::None,
            chunk_size: 4096,
            dedup_enabled: false,
            sealed: true,
            passphrase: None,
        };
        let id = rt
            .block_on(manager.create_archive("pre-sealed", config))
            .unwrap();
        let result = rt.block_on(manager.seal_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveSealed(_))));
    }

    #[test]
    fn test_seal_nonexistent_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let id = ArchiveId::new();
        let result = rt.block_on(manager.seal_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_sealed_archive_handle_reports_sealed() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = ArchiveConfig {
            name: "sealed-handle".into(),
            encryption: None,
            compression: CompressionAlgorithm::None,
            chunk_size: 4096,
            dedup_enabled: false,
            sealed: true,
            passphrase: None,
        };
        let id = rt
            .block_on(manager.create_archive("sealed-handle", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        assert!(!handle.is_closed());
    }

    #[test]
    fn test_list_archives() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();

        let config_a = default_config("archive-a");
        let config_b = default_config("archive-b");
        let config_c = default_config("archive-c");

        rt.block_on(manager.create_archive("archive-a", config_a))
            .unwrap();
        rt.block_on(manager.create_archive("archive-b", config_b))
            .unwrap();
        rt.block_on(manager.create_archive("archive-c", config_c))
            .unwrap();

        let archives = rt.block_on(manager.list_archives()).unwrap();
        assert_eq!(archives.len(), 3);
    }

    #[test]
    fn test_list_archives_excludes_deleted() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();

        let config_a = default_config("archive-x");
        let config_b = default_config("archive-y");

        let id_x = rt
            .block_on(manager.create_archive("archive-x", config_a))
            .unwrap();
        rt.block_on(manager.create_archive("archive-y", config_b))
            .unwrap();

        rt.block_on(manager.delete_archive(&id_x)).unwrap();

        let archives = rt.block_on(manager.list_archives()).unwrap();
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].name, "archive-y");
    }

    #[test]
    fn test_list_archives_empty() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let archives = rt.block_on(manager.list_archives()).unwrap();
        assert!(archives.is_empty());
    }

    #[test]
    fn test_get_archive_by_id() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("gettable");
        let id = rt
            .block_on(manager.create_archive("gettable", config))
            .unwrap();
        let archive = rt.block_on(manager.get_archive(&id)).unwrap();
        assert_eq!(archive.id, id);
        assert_eq!(archive.name, "gettable");
    }

    #[test]
    fn test_get_nonexistent_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let id = ArchiveId::new();
        let result = rt.block_on(manager.get_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_get_deleted_archive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("deleted-get");
        let id = rt
            .block_on(manager.create_archive("deleted-get", config))
            .unwrap();
        rt.block_on(manager.delete_archive(&id)).unwrap();
        let result = rt.block_on(manager.get_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("unique");
        rt.block_on(manager.create_archive("unique", config))
            .unwrap();
        let dup_config = default_config("unique");
        let result = rt.block_on(manager.create_archive("unique", dup_config));
        assert!(matches!(result, Err(AegisError::AlreadyExists(_))));
    }

    #[test]
    fn test_duplicate_name_allowed_after_delete() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("reusable");
        let id = rt
            .block_on(manager.create_archive("reusable", config))
            .unwrap();
        rt.block_on(manager.delete_archive(&id)).unwrap();

        let config2 = default_config("reusable");
        let id2 = rt
            .block_on(manager.create_archive("reusable", config2))
            .unwrap();
        assert_ne!(id2, ArchiveId::nil());
    }

    #[test]
    fn test_archive_handle_filesystem() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("handle-fs");
        let id = rt
            .block_on(manager.create_archive("handle-fs", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        let _fs = handle.filesystem();
    }

    #[test]
    fn test_archive_handle_snapshot() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("handle-snap");
        let id = rt
            .block_on(manager.create_archive("handle-snap", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        let _snap = handle.snapshot();
    }

    #[test]
    fn test_archive_handle_manifest() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("handle-man");
        let id = rt
            .block_on(manager.create_archive("handle-man", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        let _man = handle.manifest();
    }

    #[test]
    fn test_archive_handle_integrity() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("handle-int");
        let id = rt
            .block_on(manager.create_archive("handle-int", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        let _int = handle.integrity();
    }

    #[test]
    fn test_archive_handle_close() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("closeable");
        let id = rt
            .block_on(manager.create_archive("closeable", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        rt.block_on(handle.close()).unwrap();
    }

    #[test]
    fn test_create_archive_multiple() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        for i in 0..10 {
            let config = default_config(&format!("multi-{}", i));
            let id = rt
                .block_on(manager.create_archive(&format!("multi-{}", i), config))
                .unwrap();
            assert_ne!(id, ArchiveId::nil());
        }
        let archives = rt.block_on(manager.list_archives()).unwrap();
        assert_eq!(archives.len(), 10);
    }

    #[test]
    fn test_archive_with_all_compression_algorithms() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();

        let algorithms = [
            CompressionAlgorithm::None,
            CompressionAlgorithm::Lz4,
            CompressionAlgorithm::Zstd(1),
            CompressionAlgorithm::Zstd(22),
        ];

        for (i, algo) in algorithms.iter().enumerate() {
            let config = ArchiveConfig {
                name: format!("compression-{}", i),
                encryption: None,
                compression: *algo,
                chunk_size: 4096,
                dedup_enabled: false,
                sealed: false,
                passphrase: None,
            };
            let id = rt
                .block_on(manager.create_archive(&format!("compression-{}", i), config))
                .unwrap();
            let archive = rt.block_on(manager.get_archive(&id)).unwrap();
            assert_eq!(archive.compression, *algo);
        }
    }

    #[test]
    fn test_archive_with_different_encryption() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();

        let config_aes = ArchiveConfig {
            name: "aes-archive".into(),
            encryption: Some(EncryptionAlgorithm::Aes256Gcm),
            compression: CompressionAlgorithm::None,
            chunk_size: 4096,
            dedup_enabled: false,
            sealed: false,
            passphrase: Some("test_password".into()),
        };
        let id_aes = rt
            .block_on(manager.create_archive("aes-archive", config_aes))
            .unwrap();
        let archive_aes = rt.block_on(manager.get_archive(&id_aes)).unwrap();
        assert!(archive_aes.encrypted);

        let config_cha = ArchiveConfig {
            name: "chacha-archive".into(),
            encryption: Some(EncryptionAlgorithm::ChaCha20Poly1305),
            compression: CompressionAlgorithm::None,
            chunk_size: 4096,
            dedup_enabled: false,
            sealed: false,
            passphrase: Some("test_password".into()),
        };
        let id_cha = rt
            .block_on(manager.create_archive("chacha-archive", config_cha))
            .unwrap();
        let archive_cha = rt.block_on(manager.get_archive(&id_cha)).unwrap();
        assert!(archive_cha.encrypted);
    }

    #[test]
    fn test_open_deleted_archive_fails() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("open-deleted");
        let id = rt
            .block_on(manager.create_archive("open-deleted", config))
            .unwrap();
        rt.block_on(manager.delete_archive(&id)).unwrap();
        let result = rt.block_on(manager.open_archive(&id));
        assert!(matches!(result, Err(AegisError::ArchiveNotFound(_))));
    }

    #[test]
    fn test_archive_handle_subscription_to_all_subsystems() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("all-subsystems");
        let id = rt
            .block_on(manager.create_archive("all-subsystems", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();

        let _fs = handle.filesystem();
        let _snap = handle.snapshot();
        let _man = handle.manifest();
        let _int = handle.integrity();
    }

    #[test]
    fn test_archive_handle_id_matches() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ArchiveManagerImpl::new();
        let config = default_config("id-match");
        let id = rt
            .block_on(manager.create_archive("id-match", config))
            .unwrap();
        let handle = rt.block_on(manager.open_archive(&id)).unwrap();
        assert_eq!(handle.id(), id);
    }

    #[test]
    fn test_disk_chunk_storage_encryption_and_compression() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();

        // 1. Instantiate the DiskChunkStorage backend
        let disk_storage = Arc::new(DiskChunkStorage::new(path).unwrap());

        // 2. Generate key derivation & providers
        let passphrase = "my_secure_passphrase_for_testing";
        let archive_id = ArchiveId::new();
        let salt = archive_id.as_uuid().as_bytes();
        let derivation = crate::crypto::KeyDerivation::new("AegisFS Archive Key Derivation");
        let key = derivation.derive_key(passphrase, salt);
        let key_id = archive_id.as_uuid().as_bytes().to_vec();

        let encryption_provider: Arc<dyn EncryptionProvider> =
            Arc::new(crate::crypto::Aes256GcmProvider::new(key, key_id));

        let compression_provider: Arc<dyn CompressionProvider> =
            Arc::new(crate::compression::ZstdCompression::new(3));

        // 3. Wrap disk storage with our EncryptedCompressedChunkStorage layer
        let secure_storage = EncryptedCompressedChunkStorage::new(
            disk_storage.clone(),
            Some(encryption_provider),
            Some(compression_provider),
        );

        // 4. Create a test chunk with realistic data
        let original_data =
            b"Verify encryption, compression, and disk persistence flow thoroughly!";
        let chunk_id = ChunkId::from_data(original_data);
        let chunk = Chunk::new(chunk_id, bytes::Bytes::from_static(original_data));

        rt.block_on(async {
            // 5. Store chunk
            let stored_id = secure_storage.store_chunk(chunk).await.unwrap();
            assert_eq!(stored_id, chunk_id);

            // 6. Verify chunk is persisted on disk (and that it is actually encrypted, i.e., not equal to original_data)
            let raw_chunk_on_disk = disk_storage.read_chunk(&chunk_id).await.unwrap();
            assert_ne!(raw_chunk_on_disk.data.as_ref(), original_data);

            // 7. Read chunk back through the secure layer (decrypted and decompressed)
            let retrieved_chunk = secure_storage.read_chunk(&chunk_id).await.unwrap();
            assert_eq!(retrieved_chunk.data.as_ref(), original_data);

            // 8. Test list, total_size, chunk_count, has_chunk, and delete_chunk
            assert!(secure_storage.has_chunk(&chunk_id).await.unwrap());
            assert_eq!(secure_storage.chunk_count().await.unwrap(), 1);
            assert!(secure_storage.total_size().await.unwrap() > 0);

            let chunk_list = secure_storage.list_chunks().await.unwrap();
            assert_eq!(chunk_list, vec![chunk_id]);

            secure_storage.delete_chunk(&chunk_id).await.unwrap();
            assert!(!secure_storage.has_chunk(&chunk_id).await.unwrap());
            assert_eq!(secure_storage.chunk_count().await.unwrap(), 0);
        });
    }
}
