use std::collections::HashMap;

use dashmap::DashMap;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{BoxFuture, MetadataIndex, MetadataQuery};
use crate::core::types::*;

pub struct MemoryMetadataIndex {
    nodes: DashMap<NodeId, Node>,
    children: DashMap<NodeId, Vec<NodeId>>,
}

impl MemoryMetadataIndex {
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
            children: DashMap::new(),
        }
    }

    pub fn add_child(&self, parent: &NodeId, child: &NodeId) {
        self.children.entry(*parent).or_default().push(*child);
    }

    pub fn remove_child(&self, parent: &NodeId, child: &NodeId) {
        if let Some(mut children) = self.children.get_mut(parent) {
            children.retain(|c| c != child);
        }
    }
}

impl Default for MemoryMetadataIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataIndex for MemoryMetadataIndex {
    fn put_node(&self, node: Node) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move {
            self.nodes.insert(node.id, node);
            Ok(())
        })
    }

    fn get_node(&self, id: &NodeId) -> BoxFuture<'_, AegisResult<Node>> {
        let id = *id;
        Box::pin(async move {
            self.nodes
                .get(&id)
                .map(|r| r.clone())
                .ok_or_else(|| AegisError::NodeNotFound(id.to_string()))
        })
    }

    fn delete_node(&self, id: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
        let id = *id;
        Box::pin(async move {
            if !self.nodes.contains_key(&id) {
                return Err(AegisError::NodeNotFound(id.to_string()));
            }
            self.nodes.remove(&id);

            let parent_keys: Vec<NodeId> = self.children.iter().map(|r| *r.key()).collect();
            for parent_key in parent_keys {
                if let Some(mut children) = self.children.get_mut(&parent_key) {
                    children.retain(|c| *c != id);
                }
            }
            self.children.remove(&id);
            Ok(())
        })
    }

    fn list_children(&self, parent_id: &NodeId) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
        let parent_id = *parent_id;
        Box::pin(async move {
            let child_ids = self
                .children
                .get(&parent_id)
                .map(|r| r.clone())
                .unwrap_or_default();
            let mut result = Vec::with_capacity(child_ids.len());
            for child_id in &child_ids {
                if let Some(node) = self.nodes.get(child_id) {
                    result.push(node.clone());
                }
            }
            Ok(result)
        })
    }

    fn find_by_name(&self, parent_id: &NodeId, name: &str) -> BoxFuture<'_, AegisResult<Option<Node>>> {
        let parent_id = *parent_id;
        let name = name.to_string();
        Box::pin(async move {
            let child_ids = self
                .children
                .get(&parent_id)
                .map(|r| r.clone())
                .unwrap_or_default();
            for child_id in &child_ids {
                if let Some(node) = self.nodes.get(child_id) {
                    if node.name == name {
                        return Ok(Some(node.clone()));
                    }
                }
            }
            Ok(None)
        })
    }

    fn search(&self, query: &dyn MetadataQuery) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
        let name_filter = query.name_filter().map(|s| s.to_string());
        let kind_filter = query.kind_filter();
        let label_filter = query.label_filter().cloned();
        let offset = query.offset();
        let limit = query.limit();
        Box::pin(async move {
            let mut results: Vec<Node> = self.nodes.iter().map(|r| r.clone()).collect();

            if let Some(ref name) = name_filter {
                results.retain(|n| n.name.contains(name.as_str()));
            }

            if let Some(kind) = kind_filter {
                results.retain(|n| n.kind == kind);
            }

            if let Some(ref labels) = label_filter {
                results.retain(|n| {
                    labels
                        .iter()
                        .all(|(k, v)| n.metadata.labels.get(k) == Some(v))
                });
            }

            if offset > 0 && offset < results.len() {
                results = results.split_off(offset);
            } else if offset >= results.len() {
                return Ok(Vec::new());
            }

            if let Some(limit) = limit {
                results.truncate(limit);
            }

            Ok(results)
        })
    }

    fn len(&self) -> BoxFuture<'_, AegisResult<u64>> {
        Box::pin(async move { Ok(self.nodes.len() as u64) })
    }
}

pub struct MetadataQueryBuilder {
    name_filter: Option<String>,
    kind_filter: Option<NodeKind>,
    label_filter: Option<HashMap<String, String>>,
    limit: Option<usize>,
    offset: usize,
}

impl MetadataQueryBuilder {
    pub fn new() -> Self {
        Self {
            name_filter: None,
            kind_filter: None,
            label_filter: None,
            limit: None,
            offset: 0,
        }
    }

    pub fn with_name_filter(mut self, name: &str) -> Self {
        self.name_filter = Some(name.to_string());
        self
    }

    pub fn with_kind_filter(mut self, kind: NodeKind) -> Self {
        self.kind_filter = Some(kind);
        self
    }

    pub fn with_label(mut self, key: &str, value: &str) -> Self {
        self.label_filter
            .get_or_insert_with(HashMap::new)
            .insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    pub fn build(self) -> MetadataQueryImpl {
        MetadataQueryImpl {
            name_filter: self.name_filter,
            kind_filter: self.kind_filter,
            label_filter: self.label_filter,
            limit: self.limit,
            offset: self.offset,
        }
    }
}

impl Default for MetadataQueryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MetadataQueryImpl {
    name_filter: Option<String>,
    kind_filter: Option<NodeKind>,
    label_filter: Option<HashMap<String, String>>,
    limit: Option<usize>,
    offset: usize,
}

impl MetadataQuery for MetadataQueryImpl {
    fn name_filter(&self) -> Option<&str> {
        self.name_filter.as_deref()
    }

    fn kind_filter(&self) -> Option<NodeKind> {
        self.kind_filter
    }

    fn label_filter(&self) -> Option<&HashMap<String, String>> {
        self.label_filter.as_ref()
    }

    fn limit(&self) -> Option<usize> {
        self.limit
    }

    fn offset(&self) -> usize {
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::MetadataIndex;
    use std::sync::Arc;
    use chrono::Utc;

    fn create_node(id: NodeId, name: &str, kind: NodeKind) -> Node {
        Node {
            id,
            name: name.to_string(),
            kind,
            size: 0,
            mode: NodePermissions::default_for("test"),
            created_at: Utc::now(),
            modified_at: Utc::now(),
            content_hash: None,
            metadata: NodeMetadata::default(),
        }
    }

    fn create_node_with_labels(
        id: NodeId,
        name: &str,
        kind: NodeKind,
        labels: &[(&str, &str)],
    ) -> Node {
        let mut node = create_node(id, name, kind);
        for (k, v) in labels {
            node.metadata.labels.insert(k.to_string(), v.to_string());
        }
        node
    }

    #[tokio::test]
    async fn test_insert_and_retrieve() {
        let index = MemoryMetadataIndex::new();
        let id = NodeId::new();
        let node = create_node(id, "test.txt", NodeKind::File);

        index.put_node(node.clone()).await.unwrap();
        let retrieved = index.get_node(&id).await.unwrap();
        assert_eq!(retrieved.id, id);
        assert_eq!(retrieved.name, "test.txt");
        assert_eq!(retrieved.kind, NodeKind::File);
    }

    #[tokio::test]
    async fn test_update_existing_node() {
        let index = MemoryMetadataIndex::new();
        let id = NodeId::new();
        let mut node = create_node(id, "old.txt", NodeKind::File);
        index.put_node(node.clone()).await.unwrap();

        node.name = "new.txt".to_string();
        node.size = 1024;
        index.put_node(node).await.unwrap();

        let retrieved = index.get_node(&id).await.unwrap();
        assert_eq!(retrieved.name, "new.txt");
        assert_eq!(retrieved.size, 1024);
    }

    #[tokio::test]
    async fn test_delete_node() {
        let index = MemoryMetadataIndex::new();
        let id = NodeId::new();
        let node = create_node(id, "delete_me.txt", NodeKind::File);
        index.put_node(node).await.unwrap();
        assert!(index.get_node(&id).await.is_ok());

        index.delete_node(&id).await.unwrap();
        assert!(index.get_node(&id).await.is_err());

        let err = index.get_node(&id).await.unwrap_err();
        match err {
            AegisError::NodeNotFound(_) => {}
            _ => panic!("expected NodeNotFound"),
        }
    }

    #[tokio::test]
    async fn test_delete_missing_node() {
        let index = MemoryMetadataIndex::new();
        let id = NodeId::new();
        let result = index.delete_node(&id).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AegisError::NodeNotFound(_) => {}
            _ => panic!("expected NodeNotFound"),
        }
    }

    #[tokio::test]
    async fn test_list_children() {
        let index = MemoryMetadataIndex::new();
        let parent_id = NodeId::new();
        let child1 = create_node(NodeId::new(), "child1.txt", NodeKind::File);
        let child2 = create_node(NodeId::new(), "child2.txt", NodeKind::File);

        index.put_node(child1.clone()).await.unwrap();
        index.put_node(child2.clone()).await.unwrap();
        index.add_child(&parent_id, &child1.id);
        index.add_child(&parent_id, &child2.id);

        let children = index.list_children(&parent_id).await.unwrap();
        assert_eq!(children.len(), 2);
        let names: Vec<&str> = children.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"child1.txt"));
        assert!(names.contains(&"child2.txt"));
    }

    #[tokio::test]
    async fn test_list_children_empty() {
        let index = MemoryMetadataIndex::new();
        let parent_id = NodeId::new();
        let children = index.list_children(&parent_id).await.unwrap();
        assert!(children.is_empty());
    }

    #[tokio::test]
    async fn test_find_by_name() {
        let index = MemoryMetadataIndex::new();
        let parent_id = NodeId::new();
        let child = create_node(NodeId::new(), "target.txt", NodeKind::File);

        index.put_node(child.clone()).await.unwrap();
        index.add_child(&parent_id, &child.id);

        let found = index.find_by_name(&parent_id, "target.txt").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, child.id);

        let not_found = index
            .find_by_name(&parent_id, "nonexistent.txt")
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_search_name_filter() {
        let index = MemoryMetadataIndex::new();
        let ids: Vec<NodeId> = (0..5).map(|_| NodeId::new()).collect();
        let names = [
            "alpha.txt",
            "beta.txt",
            "gamma.txt",
            "delta.txt",
            "alpha_old.txt",
        ];

        for (i, name) in names.iter().enumerate() {
            let node = create_node(ids[i], name, NodeKind::File);
            index.put_node(node).await.unwrap();
        }

        let query = MetadataQueryBuilder::new().with_name_filter("alpha").build();
        let results = index.search(&query).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|n| n.name.contains("alpha")));
    }

    #[tokio::test]
    async fn test_search_kind_filter() {
        let index = MemoryMetadataIndex::new();
        let file_node = create_node(NodeId::new(), "file.txt", NodeKind::File);
        let dir_node = create_node(NodeId::new(), "dir", NodeKind::Directory);
        let link_node = create_node(NodeId::new(), "link", NodeKind::Symlink);

        index.put_node(file_node).await.unwrap();
        index.put_node(dir_node).await.unwrap();
        index.put_node(link_node).await.unwrap();

        let query = MetadataQueryBuilder::new()
            .with_kind_filter(NodeKind::Directory)
            .build();
        let results = index.search(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, NodeKind::Directory);
    }

    #[tokio::test]
    async fn test_search_limit_and_offset() {
        let index = MemoryMetadataIndex::new();
        for i in 0..10 {
            let node = create_node(NodeId::new(), &format!("item_{}.txt", i), NodeKind::File);
            index.put_node(node).await.unwrap();
        }

        let query = MetadataQueryBuilder::new()
            .with_limit(3)
            .with_offset(5)
            .build();
        let results = index.search(&query).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_search_offset_beyond_len() {
        let index = MemoryMetadataIndex::new();
        for i in 0..3 {
            let node = create_node(NodeId::new(), &format!("item_{}.txt", i), NodeKind::File);
            index.put_node(node).await.unwrap();
        }

        let query = MetadataQueryBuilder::new().with_offset(10).build();
        let results = index.search(&query).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_with_labels() {
        let index = MemoryMetadataIndex::new();
        let node1 = create_node_with_labels(
            NodeId::new(),
            "config.json",
            NodeKind::File,
            &[("env", "production"), ("type", "config")],
        );
        let node2 = create_node_with_labels(
            NodeId::new(),
            "log.txt",
            NodeKind::File,
            &[("env", "staging"), ("type", "log")],
        );
        let node3 = create_node_with_labels(
            NodeId::new(),
            "deploy.yaml",
            NodeKind::File,
            &[("env", "production"), ("type", "deploy")],
        );

        index.put_node(node1).await.unwrap();
        index.put_node(node2).await.unwrap();
        index.put_node(node3).await.unwrap();

        let query = MetadataQueryBuilder::new().with_label("env", "production").build();
        let results = index.search(&query).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|n| n.metadata.labels.get("env") == Some(&"production".to_string())));

        let query = MetadataQueryBuilder::new()
            .with_label("env", "production")
            .with_label("type", "config")
            .build();
        let results = index.search(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "config.json");
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let index = Arc::new(MemoryMetadataIndex::new());
        let mut handles = Vec::new();

        for i in 0..20 {
            let idx = index.clone();
            handles.push(tokio::spawn(async move {
                let id = NodeId::new();
                let node = create_node(id, &format!("concurrent_{}.txt", i), NodeKind::File);
                idx.put_node(node).await.unwrap();
                id
            }));
        }

        let mut ids = Vec::new();
        for h in handles {
            ids.push(h.await.unwrap());
        }

        assert_eq!(index.len().await.unwrap(), 20);

        for id in &ids {
            let node = index.get_node(id).await.unwrap();
            assert!(node.name.starts_with("concurrent_"));
        }
    }

    #[tokio::test]
    async fn test_concurrent_read_write() {
        let index = Arc::new(MemoryMetadataIndex::new());
        let id = NodeId::new();
        let node = create_node(id, "shared.txt", NodeKind::File);
        index.put_node(node).await.unwrap();

        let mut handles = Vec::new();
        for i in 0..10 {
            let idx = index.clone();
            handles.push(tokio::spawn(async move {
                let node = idx.get_node(&id).await.unwrap();
                assert_eq!(node.name, "shared.txt");
                node.size + i
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_empty_store() {
        let index = MemoryMetadataIndex::new();
        assert_eq!(index.len().await.unwrap(), 0);

        let query = MetadataQueryBuilder::new().build();
        let results = index.search(&query).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_missing_node() {
        let index = MemoryMetadataIndex::new();
        let id = NodeId::new();
        let result = index.get_node(&id).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AegisError::NodeNotFound(_) => {}
            _ => panic!("expected NodeNotFound"),
        }
    }

    #[tokio::test]
    async fn test_root_node() {
        let index = MemoryMetadataIndex::new();
        let root_id = NodeId::root();
        let root = create_node(root_id, "/", NodeKind::Directory);
        index.put_node(root).await.unwrap();

        let retrieved = index.get_node(&root_id).await.unwrap();
        assert_eq!(retrieved.id, root_id);
        assert_eq!(retrieved.name, "/");
        assert_eq!(retrieved.kind, NodeKind::Directory);
    }

    #[tokio::test]
    async fn test_query_builder_default() {
        let query = MetadataQueryBuilder::new().build();
        assert!(query.name_filter().is_none());
        assert!(query.kind_filter().is_none());
        assert!(query.label_filter().is_none());
        assert!(query.limit().is_none());
        assert_eq!(query.offset(), 0);
    }

    #[tokio::test]
    async fn test_query_builder_full() {
        let query = MetadataQueryBuilder::new()
            .with_name_filter("test")
            .with_kind_filter(NodeKind::File)
            .with_label("env", "prod")
            .with_label("version", "1")
            .with_limit(10)
            .with_offset(5)
            .build();

        assert_eq!(query.name_filter(), Some("test"));
        assert_eq!(query.kind_filter(), Some(NodeKind::File));

        let labels = query.label_filter().unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels.get("env"), Some(&"prod".to_string()));
        assert_eq!(labels.get("version"), Some(&"1".to_string()));

        assert_eq!(query.limit(), Some(10));
        assert_eq!(query.offset(), 5);
    }

    #[tokio::test]
    async fn test_delete_node_removes_from_children() {
        let index = MemoryMetadataIndex::new();
        let parent_id = NodeId::new();
        let child_id = NodeId::new();
        let child = create_node(child_id, "orphan.txt", NodeKind::File);

        index.put_node(child).await.unwrap();
        index.add_child(&parent_id, &child_id);

        index.delete_node(&child_id).await.unwrap();
        let children = index.list_children(&parent_id).await.unwrap();
        assert!(children.is_empty());
    }

    #[tokio::test]
    async fn test_search_combined_filters() {
        let index = MemoryMetadataIndex::new();
        let nodes = vec![
            create_node_with_labels(
                NodeId::new(),
                "readme.md",
                NodeKind::File,
                &[("lang", "markdown")],
            ),
            create_node_with_labels(
                NodeId::new(),
                "src",
                NodeKind::Directory,
                &[("type", "source")],
            ),
            create_node_with_labels(
                NodeId::new(),
                "main.rs",
                NodeKind::File,
                &[("lang", "rust"), ("type", "source")],
            ),
            create_node_with_labels(
                NodeId::new(),
                "lib.rs",
                NodeKind::File,
                &[("lang", "rust"), ("type", "source")],
            ),
        ];

        for n in nodes {
            index.put_node(n).await.unwrap();
        }

        let query = MetadataQueryBuilder::new()
            .with_kind_filter(NodeKind::File)
            .with_label("lang", "rust")
            .build();
        let results = index.search(&query).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|n| n.name.ends_with(".rs")));
    }

    #[test]
    fn test_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MemoryMetadataIndex>();
        assert_send_sync::<MetadataQueryBuilder>();
        assert_send_sync::<MetadataQueryImpl>();
    }
}
