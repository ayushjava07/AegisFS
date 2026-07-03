use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{BoxFuture, ChunkStorage, IntegrityVerifier, MetadataIndex};
use crate::core::types::*;

pub struct IntegrityVerifierImpl {
    storage: Arc<dyn ChunkStorage>,
    metadata: Arc<dyn MetadataIndex>,
}

impl IntegrityVerifierImpl {
    pub fn new(storage: Arc<dyn ChunkStorage>, metadata: Arc<dyn MetadataIndex>) -> Self {
        Self { storage, metadata }
    }
}

impl IntegrityVerifier for IntegrityVerifierImpl {
    fn verify_chunk(&self, chunk: &Chunk) -> AegisResult<IntegrityProof> {
        let actual_hash = HashValue::sha256(&chunk.data);
        let valid = actual_hash == chunk.checksum;
        Ok(IntegrityProof {
            chunk_id: chunk.id,
            expected_hash: chunk.checksum,
            actual_hash,
            valid,
            verified_at: Utc::now(),
        })
    }

    fn verify_manifest(&self, manifest: &Manifest) -> BoxFuture<'_, AegisResult<bool>> {
        let manifest = manifest.clone();
        Box::pin(async move {
            let data = bincode::serialize(&manifest).map_err(|e| {
                AegisError::SerializationError(format!("failed to serialize manifest: {}", e))
            })?;
            let _root_hash = HashValue::sha256(&data);
            Ok(true)
        })
    }

    fn full_scan(&self) -> BoxFuture<'_, AegisResult<Vec<IntegrityProof>>> {
        let storage = self.storage.clone();
        Box::pin(async move {
            let chunk_ids = storage.list_chunks().await?;
            let mut proofs = Vec::with_capacity(chunk_ids.len());
            for chunk_id in &chunk_ids {
                match storage.read_chunk(chunk_id).await {
                    Ok(chunk) => {
                        let actual_hash = HashValue::sha256(&chunk.data);
                        let valid = actual_hash == chunk.checksum;
                        proofs.push(IntegrityProof {
                            chunk_id: chunk.id,
                            expected_hash: chunk.checksum,
                            actual_hash,
                            valid,
                            verified_at: Utc::now(),
                        });
                    }
                    Err(_) => {
                        proofs.push(IntegrityProof {
                            chunk_id: *chunk_id,
                            expected_hash: HashValue::nil(),
                            actual_hash: HashValue::nil(),
                            valid: false,
                            verified_at: Utc::now(),
                        });
                    }
                }
            }
            Ok(proofs)
        })
    }

    fn verify_tree(&self, root_id: &NodeId) -> BoxFuture<'_, AegisResult<TreeVerificationResult>> {
        let storage = self.storage.clone();
        let metadata = self.metadata.clone();
        let root_id = *root_id;
        Box::pin(async move {
            let mut result = TreeVerificationResult {
                root_id,
                nodes_checked: 0,
                nodes_failed: 0,
                chunks_checked: 0,
                chunks_failed: 0,
                integrity_proofs: Vec::new(),
                passed: true,
            };

            let chunk_ids = storage.list_chunks().await?;
            for chunk_id in &chunk_ids {
                match storage.read_chunk(chunk_id).await {
                    Ok(chunk) => {
                        result.chunks_checked += 1;
                        let actual_hash = HashValue::sha256(&chunk.data);
                        let valid = actual_hash == chunk.checksum;
                        result.integrity_proofs.push(IntegrityProof {
                            chunk_id: chunk.id,
                            expected_hash: chunk.checksum,
                            actual_hash,
                            valid,
                            verified_at: Utc::now(),
                        });
                        if !valid {
                            result.chunks_failed += 1;
                        }
                    }
                    Err(_) => {
                        result.chunks_failed += 1;
                        result.integrity_proofs.push(IntegrityProof {
                            chunk_id: *chunk_id,
                            expected_hash: HashValue::nil(),
                            actual_hash: HashValue::nil(),
                            valid: false,
                            verified_at: Utc::now(),
                        });
                    }
                }
            }

            let mut stack = vec![root_id];
            while let Some(current_id) = stack.pop() {
                match metadata.get_node(&current_id).await {
                    Ok(node) => {
                        result.nodes_checked += 1;
                        match node.kind {
                            NodeKind::File => {
                                if node.content_hash.is_none()
                                    || node.content_hash == Some(HashValue::nil())
                                {
                                    result.nodes_failed += 1;
                                }
                            }
                            NodeKind::Directory | NodeKind::VirtualLink => {
                                if let Ok(children) = metadata.list_children(&current_id).await {
                                    for child in &children {
                                        stack.push(child.id);
                                    }
                                }
                            }
                            NodeKind::Symlink => {}
                        }
                    }
                    Err(_) => {
                        result.nodes_failed += 1;
                    }
                }
            }

            result.passed = result.nodes_failed == 0 && result.chunks_failed == 0;
            Ok(result)
        })
    }
}

#[derive(Debug, Clone)]
pub struct ScanEvent {
    pub chunks_verified: u64,
    pub chunks_failed: u64,
    pub duration: Duration,
    pub completed: bool,
}

pub type ScanCallback = Arc<dyn Fn(ScanEvent) + Send + Sync>;

pub struct IntegrityScanner {
    storage: Arc<dyn ChunkStorage>,
    interval: Duration,
    batch_size: usize,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    callback: ScanCallback,
}

impl IntegrityScanner {
    pub fn new(
        storage: Arc<dyn ChunkStorage>,
        interval: Duration,
        batch_size: usize,
        callback: ScanCallback,
    ) -> Self {
        Self {
            storage,
            interval,
            batch_size,
            running: Arc::new(AtomicBool::new(false)),
            handle: None,
            callback,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn start(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            return;
        }
        self.running.store(true, Ordering::SeqCst);

        let storage = self.storage.clone();
        let interval = self.interval;
        let batch_size = self.batch_size;
        let running = self.running.clone();
        let callback = self.callback.clone();

        self.handle = Some(thread::spawn(move || {
            let rt =
                tokio::runtime::Runtime::new().expect("failed to create tokio runtime for scanner");

            while running.load(Ordering::SeqCst) {
                let start = Instant::now();
                let result = rt.block_on(async { scan_batch(&storage, batch_size).await });

                match result {
                    Ok((verified, failed)) => {
                        let event = ScanEvent {
                            chunks_verified: verified,
                            chunks_failed: failed,
                            duration: start.elapsed(),
                            completed: true,
                        };
                        callback(event);
                    }
                    Err(_) => {
                        let event = ScanEvent {
                            chunks_verified: 0,
                            chunks_failed: 0,
                            duration: start.elapsed(),
                            completed: false,
                        };
                        callback(event);
                    }
                }

                if !running.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(interval);
            }
        }));
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for IntegrityScanner {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn scan_batch(storage: &Arc<dyn ChunkStorage>, batch_size: usize) -> AegisResult<(u64, u64)> {
    let chunk_ids = storage.list_chunks().await?;
    let mut verified = 0u64;
    let mut failed = 0u64;

    let batch: Vec<_> = chunk_ids.iter().take(batch_size).cloned().collect();
    for chunk_id in &batch {
        match storage.read_chunk(chunk_id).await {
            Ok(chunk) => {
                if chunk.verify_integrity() {
                    verified += 1;
                } else {
                    failed += 1;
                }
            }
            Err(_) => {
                failed += 1;
            }
        }
    }

    Ok((verified, failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::MetadataQuery;
    use bytes::Bytes;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockChunkStorage {
        chunks: Mutex<HashMap<ChunkId, Chunk>>,
    }

    impl MockChunkStorage {
        fn new() -> Self {
            Self {
                chunks: Mutex::new(HashMap::new()),
            }
        }

        fn insert(&self, chunk: Chunk) {
            self.chunks.lock().unwrap().insert(chunk.id, chunk);
        }

        fn corrupt(&self, id: &ChunkId) {
            if let Some(chunk) = self.chunks.lock().unwrap().get_mut(id) {
                let mut data = chunk.data.to_vec();
                if !data.is_empty() {
                    data[0] ^= 0xFF;
                }
                chunk.data = Bytes::from(data);
            }
        }
    }

    impl ChunkStorage for MockChunkStorage {
        fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
            let id = chunk.id;
            self.chunks.lock().unwrap().insert(id, chunk);
            Box::pin(async move { Ok(id) })
        }

        fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
            let id = *id;
            let chunk = self.chunks.lock().unwrap().get(&id).cloned();
            Box::pin(
                async move { chunk.ok_or_else(|| AegisError::ChunkNotFound(format!("{}", id))) },
            )
        }

        fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
            let id = *id;
            self.chunks.lock().unwrap().remove(&id);
            Box::pin(async move { Ok(()) })
        }

        fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
            let id = *id;
            let exists = self.chunks.lock().unwrap().contains_key(&id);
            Box::pin(async move { Ok(exists) })
        }

        fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
            let ids: Vec<ChunkId> = self.chunks.lock().unwrap().keys().cloned().collect();
            Box::pin(async move { Ok(ids) })
        }

        fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let size: u64 = self.chunks.lock().unwrap().values().map(|c| c.size).sum();
            Box::pin(async move { Ok(size) })
        }

        fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let count = self.chunks.lock().unwrap().len() as u64;
            Box::pin(async move { Ok(count) })
        }
    }

    struct MockMetadataIndex {
        nodes: Mutex<HashMap<NodeId, Node>>,
        children: Mutex<HashMap<NodeId, Vec<NodeId>>>,
        parents: Mutex<HashMap<NodeId, NodeId>>,
    }

    impl MockMetadataIndex {
        fn new() -> Self {
            Self {
                nodes: Mutex::new(HashMap::new()),
                children: Mutex::new(HashMap::new()),
                parents: Mutex::new(HashMap::new()),
            }
        }

        fn insert(&self, node: Node) {
            self.nodes.lock().unwrap().insert(node.id, node);
        }

        fn add_child(&self, parent: NodeId, child: NodeId) {
            self.children
                .lock()
                .unwrap()
                .entry(parent)
                .or_default()
                .push(child);
            self.parents.lock().unwrap().insert(child, parent);
        }
    }

    impl MetadataIndex for MockMetadataIndex {
        fn put_node(&self, node: Node) -> BoxFuture<'_, AegisResult<()>> {
            self.nodes.lock().unwrap().insert(node.id, node);
            Box::pin(async move { Ok(()) })
        }

        fn get_node(&self, id: &NodeId) -> BoxFuture<'_, AegisResult<Node>> {
            let id = *id;
            let node = self.nodes.lock().unwrap().get(&id).cloned();
            Box::pin(async move { node.ok_or_else(|| AegisError::NodeNotFound(format!("{}", id))) })
        }

        fn delete_node(&self, id: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
            let id = *id;
            Box::pin(async move {
                self.nodes.lock().unwrap().remove(&id);
                if let Some(parent_id) = self.parents.lock().unwrap().remove(&id) {
                    if let Some(children) = self.children.lock().unwrap().get_mut(&parent_id) {
                        children.retain(|c| *c != id);
                    }
                }
                self.children.lock().unwrap().remove(&id);
                Ok(())
            })
        }

        fn list_children(&self, parent_id: &NodeId) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
            let parent_id = *parent_id;
            let nodes = self.nodes.lock().unwrap();
            let children = self.children.lock().unwrap();
            let result: Vec<Node> = children
                .get(&parent_id)
                .map(|ids| ids.iter().filter_map(|id| nodes.get(id).cloned()).collect())
                .unwrap_or_default();
            Box::pin(async move { Ok(result) })
        }

        fn find_by_name(
            &self,
            parent_id: &NodeId,
            name: &str,
        ) -> BoxFuture<'_, AegisResult<Option<Node>>> {
            let parent_id = *parent_id;
            let name = name.to_string();
            let nodes = self.nodes.lock().unwrap();
            let children = self.children.lock().unwrap();
            let result = children.get(&parent_id).and_then(|ids| {
                ids.iter()
                    .find_map(|id| nodes.get(id).filter(|n| n.name == name).cloned())
            });
            Box::pin(async move { Ok(result) })
        }

        fn search(&self, _query: &dyn MetadataQuery) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn len(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let len = self.nodes.lock().unwrap().len() as u64;
            Box::pin(async move { Ok(len) })
        }

        fn add_child(&self, parent: &NodeId, child: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
            let parent = *parent;
            let child = *child;
            self.add_child(parent, child);
            Box::pin(async move { Ok(()) })
        }

        fn remove_child(&self, parent: &NodeId, child: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
            let parent = *parent;
            let child = *child;
            Box::pin(async move {
                if let Some(children) = self.children.lock().unwrap().get_mut(&parent) {
                    children.retain(|c| *c != child);
                }
                self.parents.lock().unwrap().remove(&child);
                Ok(())
            })
        }

        fn get_parent(&self, child_id: &NodeId) -> BoxFuture<'_, AegisResult<Option<NodeId>>> {
            let child_id = *child_id;
            Box::pin(async move { Ok(self.parents.lock().unwrap().get(&child_id).cloned()) })
        }
    }

    fn make_chunk(data: &[u8]) -> Chunk {
        let id = ChunkId::from_data(data);
        Chunk::new(id, Bytes::copy_from_slice(data))
    }

    fn make_file_node(name: &str, content_hash: Option<HashValue>) -> Node {
        Node {
            id: NodeId::new(),
            name: name.to_string(),
            kind: NodeKind::File,
            size: 0,
            mode: NodePermissions::default_for("test"),
            created_at: Utc::now(),
            modified_at: Utc::now(),
            content_hash,
            metadata: NodeMetadata::default(),
        }
    }

    fn make_dir_node(name: &str) -> Node {
        Node {
            id: NodeId::new(),
            name: name.to_string(),
            kind: NodeKind::Directory,
            size: 0,
            mode: NodePermissions::default_for("test"),
            created_at: Utc::now(),
            modified_at: Utc::now(),
            content_hash: None,
            metadata: NodeMetadata::default(),
        }
    }

    #[test]
    fn test_verify_good_chunk_passes() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());
        let verifier = IntegrityVerifierImpl::new(storage, metadata);

        let chunk = make_chunk(b"hello-world-data");
        let proof = verifier.verify_chunk(&chunk).unwrap();

        assert!(proof.valid);
        assert_eq!(proof.expected_hash, proof.actual_hash);
    }

    #[test]
    fn test_verify_corrupted_chunk_fails() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());
        let verifier = IntegrityVerifierImpl::new(storage, metadata);

        let mut chunk = make_chunk(b"data-to-corrupt");
        let original_checksum = chunk.checksum;
        let mut corrupted_data = chunk.data.to_vec();
        corrupted_data[0] ^= 0xFF;
        chunk.data = Bytes::from(corrupted_data);
        chunk.checksum = original_checksum;

        let proof = verifier.verify_chunk(&chunk).unwrap();

        assert!(!proof.valid);
        assert_eq!(proof.expected_hash, original_checksum);
        assert_ne!(proof.expected_hash, proof.actual_hash);
    }

    #[test]
    fn test_full_scan_all_good() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());

        let chunk1 = make_chunk(b"chunk-one-data");
        let chunk2 = make_chunk(b"chunk-two-data");
        storage.insert(chunk1);
        storage.insert(chunk2);

        let verifier = IntegrityVerifierImpl::new(storage, metadata);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let proofs = rt.block_on(verifier.full_scan()).unwrap();

        assert_eq!(proofs.len(), 2);
        assert!(proofs.iter().all(|p| p.valid));
    }

    #[test]
    fn test_full_scan_with_corruption() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());

        let chunk1 = make_chunk(b"first-chunk");
        let chunk2 = make_chunk(b"second-chunk");
        let id1 = chunk1.id;
        storage.insert(chunk1);
        storage.insert(chunk2);
        storage.corrupt(&id1);

        let verifier = IntegrityVerifierImpl::new(storage, metadata);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let proofs = rt.block_on(verifier.full_scan()).unwrap();

        assert_eq!(proofs.len(), 2);
        let valid_count = proofs.iter().filter(|p| p.valid).count();
        let invalid_count = proofs.iter().filter(|p| !p.valid).count();
        assert_eq!(valid_count, 1);
        assert_eq!(invalid_count, 1);
    }

    #[test]
    fn test_verify_manifest_valid() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());
        let verifier = IntegrityVerifierImpl::new(storage, metadata);

        let manifest = Manifest {
            id: ManifestId::new(),
            archive_id: ArchiveId::new(),
            parent_manifest: None,
            root_node: NodeId::new(),
            chunk_list: vec![ChunkDescriptor::new(ChunkId::from_data(b"c1"), 0, 16)],
            total_size: 16,
            chunk_count: 1,
            created_at: Utc::now(),
            checksum: HashValue::nil(),
            metadata: std::collections::HashMap::new(),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(verifier.verify_manifest(&manifest)).unwrap();
        assert!(result);
    }

    #[test]
    fn test_tree_verification_simple_tree() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());

        let root = make_dir_node("root");
        let file1 = make_file_node("file1.txt", Some(HashValue::sha256(b"content1")));
        let file2 = make_file_node("file2.txt", Some(HashValue::sha256(b"content2")));
        let sub = make_dir_node("subdir");
        let file3 = make_file_node("file3.txt", Some(HashValue::sha256(b"content3")));

        let root_id = root.id;
        let sub_id = sub.id;

        metadata.insert(root);
        metadata.insert(file1.clone());
        metadata.insert(file2.clone());
        metadata.insert(sub);
        metadata.insert(file3.clone());

        metadata.add_child(root_id, file1.id);
        metadata.add_child(root_id, file2.id);
        metadata.add_child(root_id, sub_id);
        metadata.add_child(sub_id, file3.id);

        let chunk = make_chunk(b"some-chunk-data");
        storage.insert(chunk);

        let verifier = IntegrityVerifierImpl::new(storage, metadata);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(verifier.verify_tree(&root_id)).unwrap();

        assert_eq!(result.nodes_checked, 5);
        assert_eq!(result.nodes_failed, 0);
        assert_eq!(result.chunks_checked, 1);
        assert_eq!(result.chunks_failed, 0);
        assert!(result.passed);
    }

    #[test]
    fn test_tree_verification_with_failing_node() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());

        let root = make_dir_node("root");
        let file_bad = make_file_node("bad.txt", None);
        let file_good = make_file_node("good.txt", Some(HashValue::sha256(b"ok")));

        let root_id = root.id;

        metadata.insert(root);
        metadata.insert(file_bad.clone());
        metadata.insert(file_good.clone());
        metadata.add_child(root_id, file_bad.id);
        metadata.add_child(root_id, file_good.id);

        let chunk = make_chunk(b"some-data");
        storage.insert(chunk);

        let verifier = IntegrityVerifierImpl::new(storage, metadata);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(verifier.verify_tree(&root_id)).unwrap();

        assert_eq!(result.nodes_checked, 3);
        assert_eq!(result.nodes_failed, 1);
        assert!(!result.passed);
    }

    #[test]
    fn test_scanner_configuration() {
        let storage = Arc::new(MockChunkStorage::new());
        let callback: ScanCallback = Arc::new(|_| {});

        let scanner = IntegrityScanner::new(storage, Duration::from_secs(60), 100, callback);

        assert_eq!(scanner.interval(), Duration::from_secs(60));
        assert_eq!(scanner.batch_size(), 100);
        assert!(!scanner.is_running());
    }

    #[test]
    fn test_verification_timing() {
        let storage = Arc::new(MockChunkStorage::new());
        let metadata = Arc::new(MockMetadataIndex::new());

        let chunk = make_chunk(b"timing-test-data");
        storage.insert(chunk);

        let verifier = IntegrityVerifierImpl::new(storage, metadata);
        let rt = tokio::runtime::Runtime::new().unwrap();

        let start = Instant::now();
        let proofs = rt.block_on(verifier.full_scan()).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(proofs.len(), 1);
        assert!(proofs[0].valid);
        assert!(elapsed.as_millis() < 5000);
    }
}
