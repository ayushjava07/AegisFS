use std::collections::HashMap;

use chrono::Utc;
use dashmap::DashMap;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{BoxFuture, ManifestStore};
use crate::core::types::*;

pub struct ManifestBuilder {
    archive_id: ArchiveId,
    parent_manifest: Option<ManifestId>,
    root_node: NodeId,
    chunk_list: Vec<ChunkDescriptor>,
    total_size: u64,
    metadata: HashMap<String, String>,
}

impl ManifestBuilder {
    pub fn new(archive_id: ArchiveId) -> Self {
        Self {
            archive_id,
            parent_manifest: None,
            root_node: NodeId::nil(),
            chunk_list: Vec::new(),
            total_size: 0,
            metadata: HashMap::new(),
        }
    }

    pub fn with_parent(mut self, manifest_id: ManifestId) -> Self {
        self.parent_manifest = Some(manifest_id);
        self
    }

    pub fn with_root_node(mut self, node_id: NodeId) -> Self {
        self.root_node = node_id;
        self
    }

    pub fn add_chunk(mut self, descriptor: ChunkDescriptor) -> Self {
        self.total_size += descriptor.size;
        self.chunk_list.push(descriptor);
        self
    }

    pub fn add_chunks(mut self, descriptors: Vec<ChunkDescriptor>) -> Self {
        for d in &descriptors {
            self.total_size += d.size;
        }
        self.chunk_list.extend(descriptors);
        self
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    pub fn build(self) -> Manifest {
        let id = ManifestId::new();
        let created_at = Utc::now();
        let chunk_count = self.chunk_list.len() as u64;

        let mut manifest = Manifest {
            id,
            archive_id: self.archive_id,
            parent_manifest: self.parent_manifest,
            root_node: self.root_node,
            chunk_list: self.chunk_list,
            total_size: self.total_size,
            chunk_count,
            created_at,
            checksum: HashValue::nil(),
            metadata: self.metadata,
        };

        let serialized = serde_json::to_vec(&manifest)
            .unwrap_or_else(|_| Vec::new());
        if !serialized.is_empty() {
            manifest.checksum = HashValue::sha256(&serialized);
        }

        manifest
    }
}

pub struct MemoryManifestStore {
    store: DashMap<ManifestId, Manifest>,
}

impl MemoryManifestStore {
    pub fn new() -> Self {
        Self {
            store: DashMap::new(),
        }
    }
}

impl Default for MemoryManifestStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ManifestStore for MemoryManifestStore {
    fn put_manifest(&self, manifest: Manifest) -> BoxFuture<'_, AegisResult<ManifestId>> {
        let store = &self.store;
        Box::pin(async move {
            let id = manifest.id;
            store.insert(id, manifest);
            Ok(id)
        })
    }

    fn get_manifest(&self, id: &ManifestId) -> BoxFuture<'_, AegisResult<Manifest>> {
        let store = &self.store;
        let id = *id;
        Box::pin(async move {
            store
                .get(&id)
                .map(|r| r.clone())
                .ok_or_else(|| AegisError::Internal(format!("manifest not found: {}", id)))
        })
    }

    fn delete_manifest(&self, id: &ManifestId) -> BoxFuture<'_, AegisResult<()>> {
        let store = &self.store;
        let id = *id;
        Box::pin(async move {
            store
                .remove(&id)
                .ok_or_else(|| AegisError::Internal(format!("manifest not found: {}", id)))?;
            Ok(())
        })
    }

    fn list_manifests(&self) -> BoxFuture<'_, AegisResult<Vec<ManifestId>>> {
        let store = &self.store;
        Box::pin(async move {
            let ids: Vec<ManifestId> = store.iter().map(|r| *r.key()).collect();
            Ok(ids)
        })
    }

    fn latest_manifest(&self, archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Manifest>> {
        let store = &self.store;
        let archive_id = *archive_id;
        Box::pin(async move {
            let latest = store
                .iter()
                .filter(|r| r.archive_id == archive_id)
                .max_by_key(|r| r.created_at)
                .map(|r| r.clone());

            latest.ok_or_else(|| {
                AegisError::Internal(format!("no manifest found for archive: {}", archive_id))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk_descriptor(offset: u64, size: u64) -> ChunkDescriptor {
        ChunkDescriptor::new(ChunkId::nil(), offset, size)
    }

    #[test]
    fn test_build_manifest() {
        let archive_id = ArchiveId::new();
        let parent_id = ManifestId::new();
        let root_node = NodeId::root();
        let chunk = make_chunk_descriptor(0, 1024);

        let manifest = ManifestBuilder::new(archive_id)
            .with_parent(parent_id)
            .with_root_node(root_node)
            .add_chunk(chunk.clone())
            .with_metadata("key1", "value1")
            .with_metadata("key2", "value2")
            .build();

        assert_eq!(manifest.archive_id, archive_id);
        assert_eq!(manifest.parent_manifest, Some(parent_id));
        assert_eq!(manifest.root_node, root_node);
        assert_eq!(manifest.chunk_list.len(), 1);
        assert_eq!(manifest.chunk_list[0], chunk);
        assert_eq!(manifest.total_size, 1024);
        assert_eq!(manifest.chunk_count, 1);
        assert_ne!(manifest.checksum, HashValue::nil());
        assert_eq!(manifest.metadata.get("key1").unwrap(), "value1");
        assert_eq!(manifest.metadata.get("key2").unwrap(), "value2");
    }

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let store = MemoryManifestStore::new();
        let archive_id = ArchiveId::new();
        let manifest = ManifestBuilder::new(archive_id).build();
        let id = manifest.id;

        let stored_id = store.put_manifest(manifest.clone()).await.unwrap();
        assert_eq!(stored_id, id);

        let retrieved = store.get_manifest(&id).await.unwrap();
        assert_eq!(retrieved, manifest);
    }

    #[tokio::test]
    async fn test_delete_manifest() {
        let store = MemoryManifestStore::new();
        let archive_id = ArchiveId::new();
        let manifest = ManifestBuilder::new(archive_id).build();
        let id = manifest.id;

        store.put_manifest(manifest).await.unwrap();
        store.delete_manifest(&id).await.unwrap();

        let result = store.get_manifest(&id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_manifests() {
        let store = MemoryManifestStore::new();
        let archive_id = ArchiveId::new();
        let m1 = ManifestBuilder::new(archive_id).build();
        let m2 = ManifestBuilder::new(archive_id).build();
        let m3 = ManifestBuilder::new(archive_id).build();

        store.put_manifest(m1).await.unwrap();
        store.put_manifest(m2).await.unwrap();
        store.put_manifest(m3).await.unwrap();

        let ids = store.list_manifests().await.unwrap();
        assert_eq!(ids.len(), 3);
    }

    #[tokio::test]
    async fn test_latest_manifest() {
        let store = MemoryManifestStore::new();
        let archive_id = ArchiveId::new();
        let other_id = ArchiveId::new();

        let m1 = ManifestBuilder::new(archive_id).build();
        let m2 = ManifestBuilder::new(archive_id).build();
        let m3 = ManifestBuilder::new(other_id).build();

        store.put_manifest(m1.clone()).await.unwrap();
        store.put_manifest(m2.clone()).await.unwrap();
        store.put_manifest(m3).await.unwrap();

        let latest = store.latest_manifest(&archive_id).await.unwrap();
        assert_eq!(latest.id, m2.id);
        assert_eq!(latest.archive_id, archive_id);
    }

    #[test]
    fn test_builder_defaults() {
        let archive_id = ArchiveId::new();
        let manifest = ManifestBuilder::new(archive_id).build();

        assert_eq!(manifest.archive_id, archive_id);
        assert!(manifest.parent_manifest.is_none());
        assert_eq!(manifest.root_node, NodeId::nil());
        assert!(manifest.chunk_list.is_empty());
        assert_eq!(manifest.total_size, 0);
        assert_eq!(manifest.chunk_count, 0);
        assert!(manifest.metadata.is_empty());
        assert_ne!(manifest.checksum, HashValue::nil());
    }

    #[test]
    fn test_checksum_verification() {
        let archive_id = ArchiveId::new();
        let manifest = ManifestBuilder::new(archive_id)
            .add_chunk(make_chunk_descriptor(0, 512))
            .build();

        assert_ne!(manifest.checksum, HashValue::nil());

        let serialized = serde_json::to_vec(&{
            let mut m = manifest.clone();
            m.checksum = HashValue::nil();
            m
        })
        .unwrap();
        let expected = HashValue::sha256(&serialized);
        assert_eq!(manifest.checksum, expected);
    }

    #[test]
    fn test_parent_manifest_linking() {
        let archive_id = ArchiveId::new();
        let parent = ManifestBuilder::new(archive_id).build();
        let child = ManifestBuilder::new(archive_id)
            .with_parent(parent.id)
            .build();

        assert_eq!(child.parent_manifest, Some(parent.id));
        assert!(parent.parent_manifest.is_none());
    }

    #[test]
    fn test_large_chunk_list() {
        let archive_id = ArchiveId::new();
        let mut builder = ManifestBuilder::new(archive_id);
        let count = 10_000;

        let chunks: Vec<ChunkDescriptor> = (0..count)
            .map(|i| make_chunk_descriptor(i as u64 * 1024, 1024))
            .collect();

        builder = builder.add_chunks(chunks);
        let manifest = builder.build();

        assert_eq!(manifest.chunk_list.len(), count);
        assert_eq!(manifest.chunk_count, count as u64);
        assert_eq!(manifest.total_size, count as u64 * 1024);
        assert_ne!(manifest.checksum, HashValue::nil());
    }

    #[tokio::test]
    async fn test_latest_manifest_no_match() {
        let store = MemoryManifestStore::new();
        let archive_id = ArchiveId::new();
        let result = store.latest_manifest(&archive_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let store = MemoryManifestStore::new();
        let id = ManifestId::new();
        let result = store.delete_manifest(&id).await;
        assert!(result.is_err());
    }
}
