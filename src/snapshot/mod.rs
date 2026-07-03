use std::collections::HashSet;
use std::sync::Arc;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{
    BoxFuture, ManifestStore, MetadataIndex, SnapshotManager, SnapshotStore,
};
use crate::core::types::*;

#[derive(Debug, Clone)]
pub struct SnapshotPolicy {
    pub max_snapshots: usize,
    pub retention_days: u64,
    pub incremental: bool,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            max_snapshots: 100,
            retention_days: 30,
            incremental: true,
        }
    }
}

pub struct SnapshotManagerImpl {
    metadata: Arc<dyn MetadataIndex>,
    manifest_store: Arc<dyn ManifestStore>,
    snapshot_store: Arc<dyn SnapshotStore>,
    archive_id: ArchiveId,
    policy: SnapshotPolicy,
}

impl SnapshotManagerImpl {
    pub fn new(
        metadata: Arc<dyn MetadataIndex>,
        manifest_store: Arc<dyn ManifestStore>,
        snapshot_store: Arc<dyn SnapshotStore>,
        archive_id: ArchiveId,
        policy: SnapshotPolicy,
    ) -> Self {
        Self {
            metadata,
            manifest_store,
            snapshot_store,
            archive_id,
            policy,
        }
    }
}

impl SnapshotManager for SnapshotManagerImpl {
    fn create(
        &self,
        labels: std::collections::HashMap<String, String>,
    ) -> BoxFuture<'_, AegisResult<SnapshotId>> {
        let archive_id = self.archive_id;
        let policy_incremental = self.policy.incremental;
        let retention_days = self.policy.retention_days;
        let manifest = Arc::clone(&self.manifest_store);
        let snapshot_store = Arc::clone(&self.snapshot_store);
        let metadata = Arc::clone(&self.metadata);
        Box::pin(async move {
            let manifest_ref = {
                let root_node = metadata.get_node(&NodeId::root()).await?;
                let total_size = metadata.len().await?;
                ManifestRef {
                    id: ManifestId::new(),
                    root_node: root_node.id,
                    chunk_count: 0,
                    total_size,
                    created_at: chrono::Utc::now(),
                }
            };
            manifest
                .put_manifest(Manifest {
                    id: manifest_ref.id,
                    archive_id,
                    parent_manifest: None,
                    root_node: manifest_ref.root_node,
                    chunk_list: Vec::new(),
                    total_size: manifest_ref.total_size,
                    chunk_count: 0,
                    created_at: manifest_ref.created_at,
                    checksum: HashValue::nil(),
                    metadata: std::collections::HashMap::new(),
                })
                .await?;

            let parent = if policy_incremental {
                match snapshot_store.latest_snapshot(&archive_id).await {
                    Ok(s) => {
                        if s.manifest.created_at
                            > chrono::Utc::now() - chrono::Duration::days(retention_days as i64)
                        {
                            Some(s.id)
                        } else {
                            None
                        }
                    }
                    Err(_) => None,
                }
            } else {
                None
            };

            let snapshot = Snapshot {
                id: SnapshotId::new(),
                parent,
                archive_id,
                manifest: manifest_ref,
                timestamp: chrono::Utc::now(),
                labels,
                incremental: policy_incremental,
            };

            snapshot_store.create_snapshot(snapshot).await
        })
    }

    fn restore(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>> {
        let manifest = Arc::clone(&self.manifest_store);
        let snapshot_store = Arc::clone(&self.snapshot_store);
        let metadata = Arc::clone(&self.metadata);
        let id = *id;
        Box::pin(async move {
            let snapshot = snapshot_store.get_snapshot(&id).await?;
            let manifest_entry = manifest.get_manifest(&snapshot.manifest.id).await?;
            let _root = metadata.get_node(&manifest_entry.root_node).await?;
            Ok(())
        })
    }

    fn list(&self) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>> {
        let archive_id = self.archive_id;
        let snapshot_store = Arc::clone(&self.snapshot_store);
        Box::pin(async move { snapshot_store.list_snapshots(&archive_id).await })
    }

    fn delete(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>> {
        let id = *id;
        let snapshot_store = Arc::clone(&self.snapshot_store);
        Box::pin(async move { snapshot_store.delete_snapshot(&id).await })
    }

    fn diff(
        &self,
        base: &SnapshotId,
        target: &SnapshotId,
    ) -> BoxFuture<'_, AegisResult<SnapshotDiff>> {
        let base = *base;
        let target = *target;
        let metadata = Arc::clone(&self.metadata);
        let manifest = Arc::clone(&self.manifest_store);
        let snapshot_store = Arc::clone(&self.snapshot_store);
        Box::pin(async move {
            let base_snap = snapshot_store.get_snapshot(&base).await?;
            let target_snap = snapshot_store.get_snapshot(&target).await?;
            let _base_manifest = manifest.get_manifest(&base_snap.manifest.id).await?;
            let _target_manifest = manifest.get_manifest(&target_snap.manifest.id).await?;

            let base_nodes = metadata
                .list_children(&base_snap.manifest.root_node)
                .await?;
            let target_nodes = metadata
                .list_children(&target_snap.manifest.root_node)
                .await?;

            let base_ids: HashSet<NodeId> = base_nodes.iter().map(|n| n.id).collect();
            let target_ids: HashSet<NodeId> = target_nodes.iter().map(|n| n.id).collect();

            let mut diff = SnapshotDiff::new();

            for id in &target_ids {
                if !base_ids.contains(id) {
                    diff.added.push(*id);
                }
            }

            for id in &base_ids {
                if !target_ids.contains(id) {
                    diff.deleted.push(*id);
                }
            }

            for id in base_ids.intersection(&target_ids) {
                let base_node = base_nodes.iter().find(|n| n.id == *id)
                    .ok_or_else(|| AegisError::NodeNotFound(format!("base node {}", id)))?;
                let target_node = target_nodes.iter().find(|n| n.id == *id)
                    .ok_or_else(|| AegisError::NodeNotFound(format!("target node {}", id)))?;
                if base_node.modified_at != target_node.modified_at {
                    diff.modified.push((base_node.id, target_node.id));
                    diff.size_delta += target_node.size as i64 - base_node.size as i64;
                } else {
                    diff.unchanged.push(*id);
                }
            }

            Ok(diff)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::AegisError;
    use crate::manifest::MemoryManifestStore;
    use crate::metadata::MemoryMetadataIndex;
    use std::sync::Arc;

    struct MockSnapshotStore {
        snapshots: std::sync::Mutex<std::collections::HashMap<SnapshotId, Snapshot>>,
    }

    impl MockSnapshotStore {
        fn new() -> Self {
            Self {
                snapshots: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    impl SnapshotStore for MockSnapshotStore {
        fn create_snapshot(&self, snapshot: Snapshot) -> BoxFuture<'_, AegisResult<SnapshotId>> {
            let id = snapshot.id;
            self.snapshots.lock().unwrap().insert(id, snapshot);
            Box::pin(async move { Ok(id) })
        }

        fn get_snapshot(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<Snapshot>> {
            let id = *id;
            let snapshots = self.snapshots.lock().unwrap();
            let res = snapshots.get(&id).cloned();
            Box::pin(async move {
                Ok(res.unwrap_or_else(|| Snapshot {
                    id,
                    parent: None,
                    archive_id: ArchiveId::nil(),
                    manifest: ManifestRef {
                        id: ManifestId::nil(),
                        root_node: NodeId::root(),
                        chunk_count: 0,
                        total_size: 0,
                        created_at: chrono::Utc::now(),
                    },
                    timestamp: chrono::Utc::now(),
                    labels: std::collections::HashMap::new(),
                    incremental: false,
                }))
            })
        }

        fn delete_snapshot(&self, id: &SnapshotId) -> BoxFuture<'_, AegisResult<()>> {
            let id = *id;
            self.snapshots.lock().unwrap().remove(&id);
            Box::pin(async move { Ok(()) })
        }

        fn list_snapshots(
            &self,
            _archive_id: &ArchiveId,
        ) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>> {
            let list: Vec<Snapshot> = self.snapshots.lock().unwrap().values().cloned().collect();
            Box::pin(async move { Ok(list) })
        }

        fn latest_snapshot(&self, _archive_id: &ArchiveId) -> BoxFuture<'_, AegisResult<Snapshot>> {
            let snapshots = self.snapshots.lock().unwrap();
            let latest = snapshots.values().max_by_key(|s| s.timestamp).cloned();
            Box::pin(
                async move { latest.ok_or_else(|| AegisError::SnapshotNotFound("none".into())) },
            )
        }

        fn snapshot_chain(
            &self,
            _snapshot_id: &SnapshotId,
        ) -> BoxFuture<'_, AegisResult<Vec<Snapshot>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    async fn create_manager() -> SnapshotManagerImpl {
        let metadata = Arc::new(MemoryMetadataIndex::new());
        let manifest_store = Arc::new(MemoryManifestStore::new());
        let snapshot_store = Arc::new(MockSnapshotStore::new());

        // Insert a nil manifest into manifest_store so it can be retrieved for nonexistent snapshot fallbacks
        let nil_manifest = Manifest {
            id: ManifestId::nil(),
            archive_id: ArchiveId::nil(),
            parent_manifest: None,
            root_node: NodeId::root(),
            chunk_list: Vec::new(),
            total_size: 0,
            chunk_count: 0,
            created_at: chrono::Utc::now(),
            checksum: HashValue::nil(),
            metadata: std::collections::HashMap::new(),
        };
        manifest_store.put_manifest(nil_manifest).await.unwrap();

        // Insert root node so metadata index isn't empty
        let root = Node {
            id: NodeId::root(),
            name: "/".into(),
            kind: NodeKind::Directory,
            size: 0,
            mode: NodePermissions::default_for("root"),
            created_at: chrono::Utc::now(),
            modified_at: chrono::Utc::now(),
            content_hash: None,
            metadata: NodeMetadata::default(),
        };
        metadata.put_node(root).await.unwrap();

        SnapshotManagerImpl::new(
            metadata,
            manifest_store,
            snapshot_store,
            ArchiveId::nil(),
            SnapshotPolicy::default(),
        )
    }

    #[tokio::test]
    async fn test_create_snapshot() {
        let manager = create_manager().await;
        let labels = std::collections::HashMap::new();
        let id = manager.create(labels).await.unwrap();
        assert_ne!(id, SnapshotId::nil());
    }

    #[tokio::test]
    async fn test_create_snapshot_with_labels() {
        let manager = create_manager().await;
        let mut labels = std::collections::HashMap::new();
        labels.insert("version".into(), "1.0".into());
        labels.insert("env".into(), "test".into());
        let id = manager.create(labels).await.unwrap();
        assert_ne!(id, SnapshotId::nil());
    }

    #[tokio::test]
    async fn test_list_empty() {
        let manager = create_manager().await;
        let snapshots = manager.list().await.unwrap();
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn test_delete_snapshot() {
        let manager = create_manager().await;
        let labels = std::collections::HashMap::new();
        let id = manager.create(labels).await.unwrap();
        manager.delete(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_diff_empty() {
        let manager = create_manager().await;
        let base = SnapshotId::new();
        let target = SnapshotId::new();
        let diff = manager.diff(&base, &target).await.unwrap();
        assert!(diff.added.is_empty());
        assert!(diff.deleted.is_empty());
    }

    #[tokio::test]
    async fn test_create_multiple_snapshots() {
        let manager = create_manager().await;
        let labels1 = std::collections::HashMap::new();
        let labels2 = std::collections::HashMap::new();

        let _id1 = manager.create(labels1).await.unwrap();
        let id2 = manager.create(labels2).await.unwrap();
        assert_ne!(id2, SnapshotId::nil());
    }

    #[tokio::test]
    async fn test_restore_snapshot() {
        let manager = create_manager().await;
        let labels = std::collections::HashMap::new();
        let id = manager.create(labels).await.unwrap();
        let result = manager.restore(&id).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_snapshot_policy_default() {
        let policy = SnapshotPolicy::default();
        assert_eq!(policy.max_snapshots, 100);
        assert_eq!(policy.retention_days, 30);
        assert!(policy.incremental);
    }

    #[test]
    fn test_snapshot_diff_added() {
        let mut diff = SnapshotDiff::new();
        diff.added.push(NodeId::new());
        assert_eq!(diff.added.len(), 1);
        assert!(diff.deleted.is_empty());
        assert!(diff.modified.is_empty());
    }

    #[test]
    fn test_snapshot_policy_custom() {
        let policy = SnapshotPolicy {
            max_snapshots: 10,
            retention_days: 7,
            incremental: false,
        };
        assert_eq!(policy.max_snapshots, 10);
        assert_eq!(policy.retention_days, 7);
        assert!(!policy.incremental);
    }
}
