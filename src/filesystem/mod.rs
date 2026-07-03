use std::sync::Arc;

use bytes::Bytes;
use chrono::Utc;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;
use crate::core::types::*;
use crate::dedup::DedupEngine;

const CHUNK_IDS_KEY: &str = "chunk_ids";

pub struct VirtualFileSystemImpl {
    metadata: Arc<dyn MetadataIndex>,
    storage: Arc<dyn ChunkStorage>,
    dedup: Arc<DedupEngine>,
}

impl VirtualFileSystemImpl {
    pub fn new(
        metadata: Arc<dyn MetadataIndex>,
        storage: Arc<dyn ChunkStorage>,
        dedup: Arc<DedupEngine>,
    ) -> Self {
        Self {
            metadata,
            storage,
            dedup,
        }
    }

    async fn delete_node_recursive(
        metadata: Arc<dyn MetadataIndex>,
        node_id: NodeId,
    ) -> AegisResult<()> {
        let child_ids: Vec<NodeId> = metadata
            .list_children(&node_id)
            .await?
            .into_iter()
            .map(|n| n.id)
            .collect();

        for child_id in child_ids {
            Box::pin(Self::delete_node_recursive(metadata.clone(), child_id)).await?;
        }

        let parent_id = metadata.get_parent(&node_id).await?;
        if let Some(parent_id) = parent_id {
            metadata.remove_child(&parent_id, &node_id).await?;
        }

        metadata.delete_node(&node_id).await?;

        Ok(())
    }
}

impl VirtualFileSystem for VirtualFileSystemImpl {
    fn create_node(
        &self,
        parent: &NodeId,
        name: &str,
        kind: NodeKind,
    ) -> BoxFuture<'_, AegisResult<NodeId>> {
        let parent = *parent;
        let name = name.to_string();
        let metadata = self.metadata.clone();

        Box::pin(async move {
            if name.is_empty() {
                return Err(AegisError::InvalidArgument(
                    "node name cannot be empty".into(),
                ));
            }

            let parent_node = metadata.get_node(&parent).await?;
            if parent_node.kind != NodeKind::Directory {
                return Err(AegisError::InvalidArgument(
                    "parent is not a directory".into(),
                ));
            }

            if let Some(_existing) = metadata.find_by_name(&parent, &name).await? {
                return Err(AegisError::AlreadyExists(format!(
                    "node '{}' already exists under parent",
                    name
                )));
            }

            let id = NodeId::new();
            let now = Utc::now();
            let node = Node {
                id,
                name,
                kind,
                size: 0,
                mode: NodePermissions::default_for("aegisfs"),
                created_at: now,
                modified_at: now,
                content_hash: None,
                metadata: NodeMetadata::default(),
            };

            metadata.put_node(node).await?;
            metadata.add_child(&parent, &id).await?;

            Ok(id)
        })
    }

    fn delete_node(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
        let metadata = self.metadata.clone();
        let node_id = *node_id;

        Box::pin(async move { Self::delete_node_recursive(metadata, node_id).await })
    }

    fn read_node(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Node>> {
        let metadata = self.metadata.clone();
        let node_id = *node_id;

        Box::pin(async move { metadata.get_node(&node_id).await })
    }

    fn write_node(&self, node_id: &NodeId, data: Bytes) -> BoxFuture<'_, AegisResult<()>> {
        let metadata = self.metadata.clone();
        let dedup = self.dedup.clone();
        let node_id = *node_id;

        Box::pin(async move {
            let mut node = metadata.get_node(&node_id).await?;
            if node.kind != NodeKind::File {
                return Err(AegisError::InvalidArgument(
                    "cannot write to a non-file node".into(),
                ));
            }

            let chunk_ids = dedup.ingest(&data).await?;
            let encoded = bincode::serialize(&chunk_ids)
                .map_err(|e| AegisError::SerializationError(e.to_string()))?;

            node.metadata
                .attributes
                .insert(CHUNK_IDS_KEY.to_string(), encoded);
            node.content_hash = Some(HashValue::sha256(&data));
            node.size = data.len() as u64;
            node.modified_at = Utc::now();

            metadata.put_node(node).await
        })
    }

    fn read_file(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Bytes>> {
        let metadata = self.metadata.clone();
        let storage = self.storage.clone();
        let node_id = *node_id;

        Box::pin(async move {
            let node = metadata.get_node(&node_id).await?;
            if node.kind != NodeKind::File {
                return Err(AegisError::InvalidArgument(
                    "cannot read a non-file node".into(),
                ));
            }

            let chunk_ids: Vec<ChunkId> = match node.metadata.attributes.get(CHUNK_IDS_KEY) {
                Some(bytes) => bincode::deserialize(bytes)
                    .map_err(|e| AegisError::DeserializationError(e.to_string()))?,
                None => return Ok(Bytes::new()),
            };

            let mut parts = Vec::with_capacity(chunk_ids.len());
            for chunk_id in &chunk_ids {
                let chunk = storage.read_chunk(chunk_id).await?;
                parts.push(chunk.data);
            }

            let total_len: usize = parts.iter().map(|b| b.len()).sum();
            let mut buf = Vec::with_capacity(total_len);
            for part in parts {
                buf.extend_from_slice(&part);
            }

            Ok(Bytes::from(buf))
        })
    }

    fn list_directory(&self, node_id: &NodeId) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
        let metadata = self.metadata.clone();
        let node_id = *node_id;

        Box::pin(async move {
            let dir_node = metadata.get_node(&node_id).await?;
            if dir_node.kind != NodeKind::Directory {
                return Err(AegisError::InvalidArgument(
                    "node is not a directory".into(),
                ));
            }

            metadata.list_children(&node_id).await
        })
    }

    fn resolve_path(&self, path: &str) -> BoxFuture<'_, AegisResult<NodeId>> {
        let metadata = self.metadata.clone();
        let path = path.to_string();

        Box::pin(async move {
            if !path.starts_with('/') {
                return Err(AegisError::InvalidArgument(
                    "path must be absolute (start with '/')".into(),
                ));
            }

            if path == "/" {
                return Ok(NodeId::root());
            }

            let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

            let mut current_id = NodeId::root();

            for component in &components {
                if let Some(child) = metadata.find_by_name(&current_id, component).await? {
                    current_id = child.id;
                } else {
                    return Err(AegisError::NodeNotFound(format!(
                        "path component '{}' not found in '{}'",
                        component, path
                    )));
                }
            }

            Ok(current_id)
        })
    }

    fn exists(&self, path: &str) -> BoxFuture<'_, AegisResult<bool>> {
        let metadata = self.metadata.clone();
        let path = path.to_string();

        Box::pin(async move {
            if !path.starts_with('/') {
                return Ok(false);
            }

            if path == "/" {
                return metadata.get_node(&NodeId::root()).await.map(|_| true);
            }

            let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

            let mut current_id = NodeId::root();

            for component in &components {
                if let Some(child) = metadata.find_by_name(&current_id, component).await? {
                    current_id = child.id;
                } else {
                    return Ok(false);
                }
            }

            Ok(true)
        })
    }
}

pub struct PathResolver {
    metadata: Arc<dyn MetadataIndex>,
}

impl PathResolver {
    pub fn new(metadata: Arc<dyn MetadataIndex>) -> Self {
        Self { metadata }
    }

    pub fn split_path(path: &str) -> Vec<String> {
        path.trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    pub async fn resolve(&self, path: &str) -> AegisResult<NodeId> {
        if !path.starts_with('/') {
            return Err(AegisError::InvalidArgument(
                "path must be absolute (start with '/')".into(),
            ));
        }

        if path == "/" {
            return Ok(NodeId::root());
        }

        let components = Self::split_path(path);
        let mut current_id = NodeId::root();

        for component in &components {
            if let Some(child) = self.metadata.find_by_name(&current_id, component).await? {
                current_id = child.id;
            } else {
                return Err(AegisError::NodeNotFound(format!(
                    "path component '{}' not found in '{}'",
                    component, path
                )));
            }
        }

        Ok(current_id)
    }

    pub async fn resolve_relative(&self, base: NodeId, path: &str) -> AegisResult<NodeId> {
        if path.is_empty() {
            return Ok(base);
        }

        let components = Self::split_path(path);
        let mut current_id = base;

        for component in &components {
            if component == ".." {
                return Err(AegisError::NotSupported(
                    "parent directory traversal not supported".into(),
                ));
            }
            if component == "." {
                continue;
            }

            if let Some(child) = self.metadata.find_by_name(&current_id, component).await? {
                current_id = child.id;
            } else {
                return Err(AegisError::NodeNotFound(format!(
                    "relative path component '{}' not found",
                    component
                )));
            }
        }

        Ok(current_id)
    }
}

pub struct NodeTreeWalker<'a> {
    metadata: &'a Arc<dyn MetadataIndex>,
}

impl<'a> NodeTreeWalker<'a> {
    pub fn new(metadata: &'a Arc<dyn MetadataIndex>) -> Self {
        Self { metadata }
    }

    pub async fn walk<F>(&self, node_id: NodeId, mut callback: F) -> AegisResult<()>
    where
        F: FnMut(&Node) -> AegisResult<()>,
    {
        self.walk_inner(node_id, &mut callback).await
    }

    async fn walk_inner<F>(&self, node_id: NodeId, callback: &mut F) -> AegisResult<()>
    where
        F: FnMut(&Node) -> AegisResult<()>,
    {
        let node = self.metadata.get_node(&node_id).await?;
        callback(&node)?;

        let children = self.metadata.list_children(&node_id).await?;
        for child in children {
            Box::pin(self.walk_inner(child.id, callback)).await?;
        }

        Ok(())
    }

    pub async fn walk_children<F>(&self, parent_id: NodeId, mut callback: F) -> AegisResult<()>
    where
        F: FnMut(&Node) -> AegisResult<()>,
    {
        let children = self.metadata.list_children(&parent_id).await?;
        for child in children {
            callback(&child)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub kind: NodeKind,
    pub node_id: NodeId,
    pub size: u64,
}

impl DirEntry {
    pub fn from_node(node: &Node) -> Self {
        Self {
            name: node.name.clone(),
            kind: node.kind,
            node_id: node.id,
            size: node.size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirEntryIter {
    entries: Vec<DirEntry>,
    pos: usize,
}

impl DirEntryIter {
    pub fn new(entries: Vec<DirEntry>) -> Self {
        Self { entries, pos: 0 }
    }
}

impl Iterator for DirEntryIter {
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.entries.len() {
            let entry = self.entries[self.pos].clone();
            self.pos += 1;
            Some(entry)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.entries.len() - self.pos;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for DirEntryIter {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::FixedSizeChunker;
    use crate::core::traits::DedupIndex;
    use crate::dedup::MemoryDedupIndex;
    use crate::metadata::MemoryMetadataIndex;
    use dashmap::DashMap;
    use std::sync::Arc;

    struct InMemoryChunkStorage {
        chunks: Arc<DashMap<ChunkId, Chunk>>,
    }

    impl InMemoryChunkStorage {
        fn new() -> Self {
            Self {
                chunks: Arc::new(DashMap::new()),
            }
        }
    }

    impl ChunkStorage for InMemoryChunkStorage {
        fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
            let id = chunk.id;
            self.chunks.insert(id, chunk);
            Box::pin(async move { Ok(id) })
        }

        fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
            let id = *id;
            let chunks = self.chunks.clone();
            Box::pin(async move {
                chunks
                    .get(&id)
                    .map(|r| r.clone())
                    .ok_or_else(|| AegisError::ChunkNotFound(id.to_string()))
            })
        }

        fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
            let id = *id;
            self.chunks.remove(&id);
            Box::pin(async move { Ok(()) })
        }

        fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
            let id = *id;
            let chunks = self.chunks.clone();
            Box::pin(async move { Ok(chunks.contains_key(&id)) })
        }

        fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
            let chunks = self.chunks.clone();
            Box::pin(async move { Ok(chunks.iter().map(|r| *r.key()).collect()) })
        }

        fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let chunks = self.chunks.clone();
            Box::pin(async move { Ok(chunks.iter().map(|r| r.size).sum()) })
        }

        fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let chunks = self.chunks.clone();
            Box::pin(async move { Ok(chunks.len() as u64) })
        }
    }

    fn setup_vfs() -> VirtualFileSystemImpl {
        let metadata = Arc::new(MemoryMetadataIndex::new());
        let storage = Arc::new(InMemoryChunkStorage::new()) as Arc<dyn ChunkStorage>;
        let chunker = Arc::new(FixedSizeChunker::new(1024)) as Arc<dyn Chunker>;
        let dedup_index = Arc::new(MemoryDedupIndex::new()) as Arc<dyn DedupIndex>;
        let dedup = Arc::new(DedupEngine::new(
            dedup_index,
            chunker.clone(),
            storage.clone(),
        ));
        VirtualFileSystemImpl::new(metadata, storage, dedup)
    }

    async fn ensure_root(vfs: &VirtualFileSystemImpl) {
        let root_id = NodeId::root();
        if vfs.read_node(&root_id).await.is_err() {
            let now = Utc::now();
            let root = Node {
                id: root_id,
                name: String::from("/"),
                kind: NodeKind::Directory,
                size: 0,
                mode: NodePermissions::default_for("aegisfs"),
                created_at: now,
                modified_at: now,
                content_hash: None,
                metadata: NodeMetadata::default(),
            };
            vfs.metadata.put_node(root).await.unwrap();
        }
    }

    async fn create_dir(vfs: &VirtualFileSystemImpl, parent: &NodeId, name: &str) -> NodeId {
        vfs.create_node(parent, name, NodeKind::Directory)
            .await
            .unwrap()
    }

    async fn create_file(vfs: &VirtualFileSystemImpl, parent: &NodeId, name: &str) -> NodeId {
        vfs.create_node(parent, name, NodeKind::File).await.unwrap()
    }

    #[tokio::test]
    async fn test_create_root_directory() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root = vfs.metadata.get_node(&NodeId::root()).await.unwrap();
        assert_eq!(root.name, "/");
        assert_eq!(root.kind, NodeKind::Directory);
    }

    #[tokio::test]
    async fn test_create_file_and_directory() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let dir_id = create_dir(&vfs, &root_id, "docs").await;
        let file_id = create_file(&vfs, &root_id, "readme.md").await;

        let dir = vfs.metadata.get_node(&dir_id).await.unwrap();
        assert_eq!(dir.name, "docs");
        assert_eq!(dir.kind, NodeKind::Directory);

        let file = vfs.metadata.get_node(&file_id).await.unwrap();
        assert_eq!(file.name, "readme.md");
        assert_eq!(file.kind, NodeKind::File);
    }

    #[tokio::test]
    async fn test_create_duplicate_name_fails() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        create_dir(&vfs, &root_id, "mydir").await;

        let result = vfs
            .create_node(&root_id, "mydir", NodeKind::Directory)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_duplicate_name_rejected() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        create_dir(&vfs, &root_id, "mydir").await;

        let result = vfs
            .create_node(&root_id, "mydir", NodeKind::Directory)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AegisError::AlreadyExists(_) => {}
            e => panic!("expected AlreadyExists, got {}", e),
        }
    }

    #[tokio::test]
    async fn test_write_and_read_file() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "test.txt").await;

        let data = Bytes::from("Hello, AegisFS!");
        vfs.write_node(&file_id, data.clone()).await.unwrap();

        let node = vfs.metadata.get_node(&file_id).await.unwrap();
        assert_eq!(node.size, 15);
        assert!(node.content_hash.is_some());

        let read_data = vfs.read_file(&file_id).await.unwrap();
        assert_eq!(read_data, Bytes::from("Hello, AegisFS!"));
    }

    #[tokio::test]
    async fn test_write_and_read_large_file() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "large.bin").await;

        let data = vec![0xABu8; 10000];
        let data_bytes = Bytes::from(data.clone());
        vfs.write_node(&file_id, data_bytes.clone()).await.unwrap();

        let read_data = vfs.read_file(&file_id).await.unwrap();
        assert_eq!(read_data.len(), 10000);
        assert_eq!(read_data.to_vec(), data);
    }

    #[tokio::test]
    async fn test_write_to_directory_fails() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let dir_id = create_dir(&vfs, &root_id, "mydir").await;

        let result = vfs.write_node(&dir_id, Bytes::from("data")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_from_directory_fails() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let dir_id = create_dir(&vfs, &root_id, "mydir").await;

        let result = vfs.read_file(&dir_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_node() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "delete_me.txt").await;

        assert!(vfs.metadata.get_node(&file_id).await.is_ok());
        vfs.delete_node(&file_id).await.unwrap();
        assert!(vfs.metadata.get_node(&file_id).await.is_err());
    }

    #[tokio::test]
    async fn test_delete_directory_recursively() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let outer = create_dir(&vfs, &root_id, "outer").await;
        let inner = create_dir(&vfs, &outer, "inner").await;
        let file_id = create_file(&vfs, &inner, "deep.txt").await;

        vfs.delete_node(&outer).await.unwrap();

        assert!(vfs.metadata.get_node(&outer).await.is_err());
        assert!(vfs.metadata.get_node(&inner).await.is_err());
        assert!(vfs.metadata.get_node(&file_id).await.is_err());
    }

    #[tokio::test]
    async fn test_list_directory() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        create_file(&vfs, &root_id, "a.txt").await;
        create_file(&vfs, &root_id, "b.txt").await;
        create_dir(&vfs, &root_id, "sub").await;

        let children = vfs.list_directory(&root_id).await.unwrap();
        assert_eq!(children.len(), 3);

        let names: Vec<&str> = children.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        assert!(names.contains(&"sub"));
    }

    #[tokio::test]
    async fn test_list_directory_empty() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let empty = create_dir(&vfs, &root_id, "empty").await;

        let children = vfs.list_directory(&empty).await.unwrap();
        assert!(children.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_path_root() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let id = vfs.resolve_path("/").await.unwrap();
        assert_eq!(id, NodeId::root());
    }

    #[tokio::test]
    async fn test_resolve_path_simple() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "readme.md").await;

        let resolved = vfs.resolve_path("/readme.md").await.unwrap();
        assert_eq!(resolved, file_id);
    }

    #[tokio::test]
    async fn test_resolve_path_nested() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let docs = create_dir(&vfs, &root_id, "docs").await;
        let api = create_dir(&vfs, &docs, "api").await;
        let file_id = create_file(&vfs, &api, "v1.md").await;

        let resolved = vfs.resolve_path("/docs/api/v1.md").await.unwrap();
        assert_eq!(resolved, file_id);
    }

    #[tokio::test]
    async fn test_resolve_path_nonexistent() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let result = vfs.resolve_path("/nonexistent/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_exists() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        create_file(&vfs, &root_id, "present.txt").await;

        assert!(vfs.exists("/").await.unwrap());
        assert!(vfs.exists("/present.txt").await.unwrap());
        assert!(!vfs.exists("/missing.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_exists_nested() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let a = create_dir(&vfs, &root_id, "a").await;
        let b = create_dir(&vfs, &a, "b").await;
        create_file(&vfs, &b, "c.txt").await;

        assert!(vfs.exists("/a/b/c.txt").await.unwrap());
        assert!(!vfs.exists("/a/b/d.txt").await.unwrap());
        assert!(!vfs.exists("/x/y/z").await.unwrap());
    }

    #[tokio::test]
    async fn test_empty_name_rejected() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let result = vfs.create_node(&root_id, "", NodeKind::File).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_relative_path_rejected() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let result = vfs.resolve_path("relative/path").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_content_roundtrip() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "roundtrip.bin").await;

        let original = Bytes::from(vec![0x42u8; 8192]);
        vfs.write_node(&file_id, original.clone()).await.unwrap();
        let read_back = vfs.read_file(&file_id).await.unwrap();

        assert_eq!(original.len(), read_back.len());
        assert_eq!(original, read_back);
    }

    #[tokio::test]
    async fn test_multiple_writes_overwrite() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "overwrite.txt").await;

        vfs.write_node(&file_id, Bytes::from("first write"))
            .await
            .unwrap();
        vfs.write_node(&file_id, Bytes::from("second write"))
            .await
            .unwrap();

        let data = vfs.read_file(&file_id).await.unwrap();
        assert_eq!(data, Bytes::from("second write"));
        assert_eq!(data.len(), 12);
    }

    #[tokio::test]
    async fn test_empty_file_content() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "empty.txt").await;

        vfs.write_node(&file_id, Bytes::new()).await.unwrap();
        let data = vfs.read_file(&file_id).await.unwrap();
        assert!(data.is_empty());
    }

    #[tokio::test]
    async fn test_create_in_non_directory_fails() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "not_a_dir.txt").await;

        let result = vfs.create_node(&file_id, "child", NodeKind::File).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_create() {
        let vfs = Arc::new(setup_vfs());
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let mut handles = Vec::new();

        for i in 0..10 {
            let vfs = vfs.clone();
            handles.push(tokio::spawn(async move {
                let name = format!("concurrent_{}.txt", i);
                vfs.create_node(&root_id, &name, NodeKind::File)
                    .await
                    .unwrap()
            }));
        }

        let mut ids = Vec::new();
        for h in handles {
            ids.push(h.await.unwrap());
        }

        assert_eq!(ids.len(), 10);
        let children = vfs.list_directory(&root_id).await.unwrap();
        assert_eq!(children.len(), 10);
    }

    #[tokio::test]
    async fn test_concurrent_writes() {
        let vfs = Arc::new(setup_vfs());
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "concurrent_write.bin").await;

        let mut handles = Vec::new();
        for i in 0..10 {
            let vfs = vfs.clone();
            let data = Bytes::from(vec![i as u8; 1000]);
            handles.push(tokio::spawn(
                async move { vfs.write_node(&file_id, data).await },
            ));
        }

        for h in handles {
            h.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn test_node_tree_walker() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let a = create_dir(&vfs, &root_id, "a").await;
        let b = create_dir(&vfs, &root_id, "b").await;
        create_file(&vfs, &a, "a1.txt").await;
        create_file(&vfs, &a, "a2.txt").await;
        create_file(&vfs, &b, "b1.txt").await;

        let walker = NodeTreeWalker::new(&vfs.metadata);
        let mut visited = Vec::new();

        walker
            .walk(root_id, |node| {
                visited.push(node.name.clone());
                Ok(())
            })
            .await
            .unwrap();

        assert!(visited.contains(&"a".to_string()));
        assert!(visited.contains(&"b".to_string()));
        assert!(visited.contains(&"a1.txt".to_string()));
        assert!(visited.contains(&"a2.txt".to_string()));
        assert!(visited.contains(&"b1.txt".to_string()));
    }

    #[tokio::test]
    async fn test_walk_children() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        create_file(&vfs, &root_id, "f1.txt").await;
        create_file(&vfs, &root_id, "f2.txt").await;
        create_dir(&vfs, &root_id, "d1").await;

        let walker = NodeTreeWalker::new(&vfs.metadata);
        let mut visited = Vec::new();

        walker
            .walk_children(root_id, |node| {
                visited.push(node.name.clone());
                Ok(())
            })
            .await
            .unwrap();

        assert_eq!(visited.len(), 3);
        assert!(visited.contains(&"f1.txt".to_string()));
        assert!(visited.contains(&"f2.txt".to_string()));
        assert!(visited.contains(&"d1".to_string()));
    }

    #[tokio::test]
    async fn test_path_resolver() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let docs = create_dir(&vfs, &root_id, "docs").await;
        let api = create_dir(&vfs, &docs, "api").await;
        let file_id = create_file(&vfs, &api, "reference.md").await;

        let resolver = PathResolver::new(vfs.metadata.clone());

        let resolved = resolver.resolve("/docs/api/reference.md").await.unwrap();
        assert_eq!(resolved, file_id);

        let resolved_root = resolver.resolve("/").await.unwrap();
        assert_eq!(resolved_root, NodeId::root());
    }

    #[tokio::test]
    async fn test_path_resolver_relative() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let docs = create_dir(&vfs, &root_id, "docs").await;
        let file_id = create_file(&vfs, &docs, "readme.md").await;

        let resolver = PathResolver::new(vfs.metadata.clone());

        let resolved = resolver.resolve_relative(docs, "readme.md").await.unwrap();
        assert_eq!(resolved, file_id);

        let resolved_self = resolver.resolve_relative(docs, "").await.unwrap();
        assert_eq!(resolved_self, docs);
    }

    #[tokio::test]
    async fn test_path_resolver_split() {
        let parts = PathResolver::split_path("/a/b/c");
        assert_eq!(parts, vec!["a", "b", "c"]);

        let parts = PathResolver::split_path("/");
        assert!(parts.is_empty());

        let parts = PathResolver::split_path("//a//b//");
        assert_eq!(parts, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn test_dir_entry_from_node() {
        let node = Node {
            id: NodeId::new(),
            name: "test.txt".to_string(),
            kind: NodeKind::File,
            size: 1024,
            mode: NodePermissions::default_for("test"),
            created_at: Utc::now(),
            modified_at: Utc::now(),
            content_hash: None,
            metadata: NodeMetadata::default(),
        };

        let entry = DirEntry::from_node(&node);
        assert_eq!(entry.name, "test.txt");
        assert_eq!(entry.kind, NodeKind::File);
        assert_eq!(entry.size, 1024);
    }

    #[tokio::test]
    async fn test_dir_entry_iter() {
        let entries = vec![
            DirEntry {
                name: "a.txt".to_string(),
                kind: NodeKind::File,
                node_id: NodeId::new(),
                size: 100,
            },
            DirEntry {
                name: "b.txt".to_string(),
                kind: NodeKind::File,
                node_id: NodeId::new(),
                size: 200,
            },
        ];

        let mut iter = DirEntryIter::new(entries.clone());
        assert_eq!(iter.len(), 2);
        assert_eq!(iter.next().unwrap().name, "a.txt");
        assert_eq!(iter.next().unwrap().name, "b.txt");
        assert!(iter.next().is_none());
    }

    #[tokio::test]
    async fn test_delete_missing_node() {
        let vfs = setup_vfs();
        let missing_id = NodeId::new();
        let result = vfs.delete_node(&missing_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_missing_node() {
        let vfs = setup_vfs();
        let missing_id = NodeId::new();
        let result = vfs.read_node(&missing_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_directory_on_file_fails() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "file.txt").await;

        let result = vfs.list_directory(&file_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_nested_directory_structure() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let level1 = create_dir(&vfs, &root_id, "level1").await;
        let level2 = create_dir(&vfs, &level1, "level2").await;
        let level3 = create_dir(&vfs, &level2, "level3").await;
        let file_id = create_file(&vfs, &level3, "deep.txt").await;

        let resolved = vfs
            .resolve_path("/level1/level2/level3/deep.txt")
            .await
            .unwrap();
        assert_eq!(resolved, file_id);

        let l1_children = vfs.list_directory(&root_id).await.unwrap();
        assert_eq!(l1_children.len(), 1);
        assert_eq!(l1_children[0].name, "level1");

        let l3_children = vfs.list_directory(&level3).await.unwrap();
        assert_eq!(l3_children.len(), 1);
        assert_eq!(l3_children[0].name, "deep.txt");
    }

    #[tokio::test]
    async fn test_multiple_files_same_content_dedup() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let f1 = create_file(&vfs, &root_id, "f1.txt").await;
        let f2 = create_file(&vfs, &root_id, "f2.txt").await;

        let data = Bytes::from("identical content");
        vfs.write_node(&f1, data.clone()).await.unwrap();
        vfs.write_node(&f2, data).await.unwrap();

        let r1 = vfs.read_file(&f1).await.unwrap();
        let r2 = vfs.read_file(&f2).await.unwrap();
        assert_eq!(r1, r2);
        assert_eq!(r1, Bytes::from("identical content"));
    }

    #[tokio::test]
    async fn test_file_size_tracking() {
        let vfs = setup_vfs();
        ensure_root(&vfs).await;

        let root_id = NodeId::root();
        let file_id = create_file(&vfs, &root_id, "size_test.bin").await;

        let data = Bytes::from(vec![0xFFu8; 5000]);
        vfs.write_node(&file_id, data).await.unwrap();

        let node = vfs.read_node(&file_id).await.unwrap();
        assert_eq!(node.size, 5000);
    }

    #[test]
    fn test_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<VirtualFileSystemImpl>();
        assert_send_sync::<PathResolver>();
    }
}
