use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use uuid::Uuid;

use super::error::AegisResult;
use super::types::*;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ChunkStorage: Send + Sync {
    fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>>;
    fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>>;
    fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>>;
    fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>>;
    fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>>;
    fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>>;
    fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>>;
}

pub trait Chunker: Send + Sync {
    fn chunk_data(&self, data: &[u8]) -> AegisResult<Vec<ChunkDescriptor>>;
    fn find_chunk_boundaries(&self, data: &[u8]) -> AegisResult<Vec<u64>>;
    fn estimate_chunk_count(&self, data_size: u64) -> u64;
    fn average_chunk_size(&self) -> u64;
    fn min_chunk_size(&self) -> u64;
    fn max_chunk_size(&self) -> u64;
}

#[allow(clippy::len_without_is_empty)]
pub trait DedupIndex: Send + Sync {
    fn insert(&self, hash: &HashValue, chunk_id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>>;
    fn lookup(&self, hash: &HashValue) -> BoxFuture<'_, AegisResult<Option<ChunkId>>>;
    fn contains(&self, hash: &HashValue) -> BoxFuture<'_, AegisResult<bool>>;
    fn remove(&self, hash: &HashValue) -> BoxFuture<'_, AegisResult<()>>;
    fn len(&self) -> BoxFuture<'_, AegisResult<u64>>;
    fn clear(&self) -> BoxFuture<'_, AegisResult<()>>;
}

pub trait EncryptionProvider: Send + Sync {
    fn encrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>>;
    fn decrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>>;
    fn algorithm(&self) -> EncryptionAlgorithm;
    fn key_identifier(&self) -> &[u8];
}

pub trait CompressionProvider: Send + Sync {
    fn compress(&self, data: &[u8]) -> AegisResult<Vec<u8>>;
    fn decompress(&self, data: &[u8]) -> AegisResult<Vec<u8>>;
    fn algorithm(&self) -> CompressionAlgorithm;
}

pub trait Hasher: Send + Sync {
    fn hash(&self, data: &[u8]) -> HashValue;
    fn hash_stream(&self, reader: &mut dyn std::io::Read) -> AegisResult<HashValue>;
}

#[allow(clippy::len_without_is_empty)]
pub trait MetadataIndex: Send + Sync {
    fn put_node(&self, node: Node) -> BoxFuture<'_, AegisResult<()>>;
    fn get_node(&self, id: &NodeId) -> BoxFuture<'_, AegisResult<Node>>;
    fn delete_node(&self, id: &NodeId) -> BoxFuture<'_, AegisResult<()>>;
    fn list_children(&self, parent_id: &NodeId) -> BoxFuture<'_, AegisResult<Vec<Node>>>;
    fn find_by_name(&self, parent_id: &NodeId, name: &str) -> BoxFuture<'_, AegisResult<Option<Node>>>;
    fn search(&self, query: &dyn MetadataQuery) -> BoxFuture<'_, AegisResult<Vec<Node>>>;
    fn len(&self) -> BoxFuture<'_, AegisResult<u64>>;
}

pub trait MetadataQuery {
    fn name_filter(&self) -> Option<&str>;
    fn kind_filter(&self) -> Option<NodeKind>;
    fn label_filter(&self) -> Option<&std::collections::HashMap<String, String>>;
    fn limit(&self) -> Option<usize>;
    fn offset(&self) -> usize;
}

pub trait JournalStore: Send + Sync {
    fn append(&self, entry: JournalEntry) -> BoxFuture<'_, AegisResult<u64>>;
    fn read_after(&self, sequence: u64, limit: usize) -> BoxFuture<'_, AegisResult<Vec<JournalEntry>>>;
    fn latest_sequence(&self) -> BoxFuture<'_, AegisResult<u64>>;
    fn truncate(&self, before_sequence: u64) -> BoxFuture<'_, AegisResult<()>>;
    fn replay(&self, handler: Box<dyn JournalHandler + Send>) -> BoxFuture<'_, AegisResult<u64>>;
}

pub trait JournalHandler {
    fn handle(&mut self, entry: &JournalEntry) -> AegisResult<()>;
}

pub trait VirtualFileSystem: Send + Sync {
    fn create_node(&self, parent: &NodeId, name: &str, kind: NodeKind) -> BoxFuture<'_, AegisResult<NodeId>>;
    fn delete_node(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<()>>;
    fn read_node(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Node>>;
    fn write_node(&self, node_id: &NodeId, data: Bytes) -> BoxFuture<'_, AegisResult<()>>;
    fn read_file(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Bytes>>;
    fn list_directory(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Vec<Node>>>;
    fn resolve_path(&self, path: &str) -> BoxFuture<'_, AegisResult<NodeId>>;
    fn exists(&self, path: &str) -> BoxFuture<'_, AegisResult<bool>>;
}

pub trait PolicyEngine: Send + Sync {
    fn evaluate_retention(&self, snapshots: &[Snapshot]) -> AegisResult<Vec<SnapshotId>>;
    fn evaluate_gc(&self, chunks: &[ChunkId], referenced: &[ChunkId]) -> AegisResult<Vec<ChunkId>>;
    fn meets_requirements(&self, archive: &Archive) -> AegisResult<bool>;
}

pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn initialize(&self) -> AegisResult<()>;
    fn shutdown(&self) -> AegisResult<()>;
}

pub trait PluginRegistry: Send + Sync {
    fn register(&self, plugin: Arc<dyn Plugin>) -> AegisResult<()>;
    fn unregister(&self, name: &str) -> AegisResult<()>;
    fn get(&self, name: &str) -> AegisResult<Arc<dyn Plugin>>;
    fn list(&self) -> AegisResult<Vec<String>>;
}

pub trait RpcService: Send + Sync {
    fn call(&self, method: &str, request: Vec<u8>) -> BoxFuture<'_, AegisResult<Vec<u8>>>;
}

pub trait NetworkTransport: Send + Sync {
    fn connect(&self, endpoint: &str) -> BoxFuture<'_, AegisResult<Box<dyn Connection>>>;
    fn bind(&self, address: &str) -> BoxFuture<'_, AegisResult<Box<dyn Listener>>>;
}

pub trait Connection: Send {
    fn send(&mut self, data: Bytes) -> BoxFuture<'_, AegisResult<()>>;
    fn receive(&mut self) -> BoxFuture<'_, AegisResult<Bytes>>;
    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>>;
}

pub trait Listener: Send {
    fn accept(&mut self) -> BoxFuture<'_, AegisResult<Box<dyn Connection>>>;
    fn local_addr(&self) -> AegisResult<String>;
}

pub trait Serializer: Send + Sync {
    fn serialize<T: serde::Serialize + ?Sized>(&self, value: &T) -> AegisResult<Vec<u8>>;
    fn deserialize<T: serde::de::DeserializeOwned>(&self, data: &[u8]) -> AegisResult<T>;
}

pub trait StreamingReader: Send {
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<usize>>;
    fn read_exact<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<()>>;
    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>>;
}

pub trait StreamingWriter: Send {
    fn write<'a>(&'a mut self, buf: &'a [u8]) -> BoxFuture<'a, AegisResult<usize>>;
    fn flush(&mut self) -> BoxFuture<'_, AegisResult<()>>;
    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>>;
}

pub trait ManifestStore: Send + Sync {
    fn put_manifest(&self, manifest: Manifest) -> BoxFuture<'_, AegisResult<ManifestId>>;
    fn get_manifest(&self, id: &ManifestId) -> BoxFuture<'_, AegisResult<Manifest>>;
    fn delete_manifest(&self, id: &ManifestId) -> BoxFuture<'_, AegisResult<()>>;
    fn list_manifests(&self) -> BoxFuture<'_, AegisResult<Vec<ManifestId>>>;
    fn latest_manifest(&self, archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Manifest>>;
}

pub trait SnapshotStore: Send + Sync {
    fn create_snapshot(&self, snapshot: Snapshot) -> BoxFuture<'_, AegisResult<SnapshotId>>;
    fn get_snapshot(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<Snapshot>>;
    fn delete_snapshot(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>>;
    fn list_snapshots(&self, archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>>;
    fn latest_snapshot(&self, archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Snapshot>>;
    fn snapshot_chain(&self, snapshot_id: &SnapshotId) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>>;
}

pub trait SyncEngine: Send + Sync {
    fn sync_to_remote(&self, archive_id: &ArchiveId, direction: SyncDirection) -> BoxFuture<'_, AegisResult<SyncResult>>;
    fn sync_snapshot(&self, snapshot_id: &SnapshotId, target: &str) -> BoxFuture<'_, AegisResult<SyncResult>>;
    fn status(&self) -> BoxFuture<'_, AegisResult<SyncStatus>>;
    fn cancel(&self) -> BoxFuture<'_, AegisResult<()>>;
}

pub trait ReplicationEngine: Send + Sync {
    fn replicate(&self, archive_id: &ArchiveId, target: &ReplicationTarget) -> BoxFuture<'_, AegisResult<ReplicationResult>>;
    fn configure_replication(&self, config: ReplicationConfig) -> BoxFuture<'_, AegisResult<()>>;
    fn status(&self) -> BoxFuture<'_, AegisResult<ReplicationStatus>>;
}

pub trait EventBus: Send + Sync {
    fn publish(&self, event: Event) -> BoxFuture<'_, AegisResult<()>>;
    fn subscribe(&self, kind: EventKind) -> BoxFuture<'_, AegisResult<Box<dyn EventReceiver>>>;
    fn unsubscribe(&self, kind: EventKind, receiver_id: Uuid) -> BoxFuture<'_, AegisResult<()>>;
}

pub trait EventReceiver: Send {
    fn recv(&mut self) -> BoxFuture<'_, Option<Event>>;
    fn try_recv(&mut self) -> Option<Event>;
}

pub trait RecoveryManager: Send + Sync {
    fn recover(&self, action: RecoveryAction) -> BoxFuture<'_, AegisResult<RecoveryReport>>;
    fn needs_recovery(&self) -> BoxFuture<'_, AegisResult<bool>>;
    fn status(&self) -> BoxFuture<'_, AegisResult<RecoveryStatus>>;
}

pub trait AuthProvider: Send + Sync {
    fn authenticate<'a>(&'a self, credentials: &'a Credentials) -> BoxFuture<'a, AegisResult<AuthToken>>;
    fn authorize<'a>(&'a self, token: &'a AuthToken, action: &'a str, resource: &'a str) -> BoxFuture<'a, AegisResult<bool>>;
    fn revoke<'a>(&'a self, token: &'a AuthToken) -> BoxFuture<'a, AegisResult<()>>;
}

pub trait ArchiveManager: Send + Sync {
    fn create_archive(&self, name: &str, config: ArchiveConfig) -> BoxFuture<'_, AegisResult<ArchiveId>>;
    fn open_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<Box<dyn ArchiveHandle>>>;
    fn delete_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<()>>;
    fn seal_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<()>>;
    fn list_archives(&self) -> BoxFuture<'_, AegisResult<Vec<Archive>>>;
    fn get_archive(&self, id: &ArchiveId) -> BoxFuture<'_, AegisResult<Archive>>;
}

pub trait ArchiveHandle: Send + Sync {
    fn id(&self) -> ArchiveId;
    fn filesystem(&self) -> Box<dyn VirtualFileSystem>;
    fn snapshot(&self) -> Box<dyn SnapshotManager>;
    fn manifest(&self) -> Box<dyn ManifestStore>;
    fn integrity(&self) -> Box<dyn IntegrityVerifier>;
    fn is_closed(&self) -> bool;
    fn close(&self) -> BoxFuture<'_, AegisResult<()>>;
}

pub trait SnapshotManager: Send + Sync {
    fn create(&self, labels: std::collections::HashMap<String, String>) -> BoxFuture<'_, AegisResult<SnapshotId>>;
    fn restore(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>>;
    fn list(&self) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>>;
    fn delete(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>>;
    fn diff(&self, base: &SnapshotId, target: &SnapshotId) -> BoxFuture<'_, AegisResult<SnapshotDiff>>;
}

pub trait IntegrityVerifier: Send + Sync {
    fn verify_chunk(&self, chunk: &Chunk) -> AegisResult<IntegrityProof>;
    fn verify_manifest(&self, manifest: &Manifest) -> BoxFuture<'_, AegisResult<bool>>;
    fn full_scan(&self) -> BoxFuture<'_, AegisResult<Vec<IntegrityProof>>>;
    fn verify_tree(&self, root_id: &NodeId) -> BoxFuture<'_, AegisResult<TreeVerificationResult>>;
}

pub trait MemoryPool: Send + Sync {
    fn allocate(&self, size: usize) -> AegisResult<MemoryBlock>;
    fn deallocate(&self, block: MemoryBlock);
    fn reset(&self);
    fn capacity(&self) -> usize;
    fn used(&self) -> usize;
}

pub trait Scheduler: Send + Sync {
    fn submit<T, F>(&self, task: F) -> BoxFuture<'_, AegisResult<TaskHandle<T>>>
    where
        T: Send + 'static,
        F: Future<Output = AegisResult<T>> + Send + 'static;
    fn schedule(&self, task: BoxFuture<'static, AegisResult<()>>, priority: TaskPriority) -> BoxFuture<'_, AegisResult<TaskId>>;
    fn cancel(&self, task_id: TaskId) -> BoxFuture<'_, AegisResult<()>>;
    fn shutdown(&self) -> BoxFuture<'_, AegisResult<()>>;
}

#[allow(clippy::len_without_is_empty)]
pub trait CacheBackend<K, V>: Send + Sync
where
    K: Send + Sync,
    V: Send + Sync + Clone,
{
    fn get(&self, key: &K) -> BoxFuture<'_, AegisResult<Option<V>>>;
    fn insert(&self, key: K, value: V) -> BoxFuture<'_, AegisResult<()>>;
    fn remove(&self, key: &K) -> BoxFuture<'_, AegisResult<()>>;
    fn clear(&self) -> BoxFuture<'_, AegisResult<()>>;
    fn len(&self) -> BoxFuture<'_, AegisResult<usize>>;
}

pub trait StorageBackend: Send + Sync {
    fn write(&self, path: &str, data: Bytes) -> BoxFuture<'_, AegisResult<()>>;
    fn read(&self, path: &str) -> BoxFuture<'_, AegisResult<Bytes>>;
    fn delete(&self, path: &str) -> BoxFuture<'_, AegisResult<()>>;
    fn exists(&self, path: &str) -> BoxFuture<'_, AegisResult<bool>>;
    fn list(&self, prefix: &str) -> BoxFuture<'_, AegisResult<Vec<String>>>;
    fn kind(&self) -> StorageBackendKind;
}
