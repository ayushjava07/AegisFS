use std::sync::Arc;
use std::time::Instant;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;
use crate::core::types::*;

pub struct RecoveryManagerImpl {
    journal: Arc<dyn JournalStore>,
    storage: Arc<dyn ChunkStorage>,
    metadata: Arc<dyn MetadataIndex>,
    status: tokio::sync::Mutex<RecoveryStatus>,
}

impl RecoveryManagerImpl {
    pub fn new(
        journal: Arc<dyn JournalStore>,
        storage: Arc<dyn ChunkStorage>,
        metadata: Arc<dyn MetadataIndex>,
    ) -> Self {
        Self {
            journal,
            storage,
            metadata,
            status: tokio::sync::Mutex::new(RecoveryStatus {
                in_progress: false,
                progress_percent: 0.0,
                current_action: None,
                last_report: None,
            }),
        }
    }
}

impl RecoveryManager for RecoveryManagerImpl {
    fn recover(&self, action: RecoveryAction) -> BoxFuture<'_, AegisResult<RecoveryReport>> {
        Box::pin(async move {
            {
                let mut s = self.status.lock().await;
                s.in_progress = true;
                s.current_action = Some(action);
                s.progress_percent = 0.0;
            }

            let start = Instant::now();
            let mut entries_replayed = 0u64;
            let chunks_repaired = 0u64;
            let mut errors: Vec<String> = Vec::new();

            let result = match action {
                RecoveryAction::ReplayJournal => {
                    struct ReplayHandler;
                    impl JournalHandler for ReplayHandler {
                        fn handle(&mut self, _entry: &JournalEntry) -> AegisResult<()> {
                            Ok(())
                        }
                    }
                    match self.journal.replay(Box::new(ReplayHandler)).await {
                        Ok(count) => {
                            entries_replayed = count;
                            {
                                let mut s = self.status.lock().await;
                                s.progress_percent = 50.0;
                            }
                            Ok(())
                        }
                        Err(e) => {
                            errors.push(format!("journal replay failed: {}", e));
                            Err(e)
                        }
                    }
                }
                RecoveryAction::IntegrityScan => {
                    let chunks = self.storage.list_chunks().await.unwrap_or_default();
                    let total = chunks.len().max(1);
                    for (i, chunk_id) in chunks.iter().enumerate() {
                        match self.storage.read_chunk(chunk_id).await {
                            Ok(chunk) => {
                                if !chunk.verify_integrity() {
                                    errors.push(format!(
                                        "integrity check failed for chunk {}",
                                        chunk_id
                                    ));
                                }
                            }
                            Err(e) => {
                                errors.push(format!("failed to read chunk {}: {}", chunk_id, e));
                            }
                        }
                        {
                            let mut s = self.status.lock().await;
                            s.progress_percent = ((i + 1) as f64 / total as f64) * 100.0;
                        }
                    }
                    Ok(())
                }
                RecoveryAction::RepairChunks => {
                    let chunks = self.storage.list_chunks().await.unwrap_or_default();
                    for chunk_id in &chunks {
                        match self.storage.read_chunk(chunk_id).await {
                            Ok(chunk) => {
                                if !chunk.verify_integrity() {
                                    errors.push(format!(
                                        "chunk {} is corrupt and cannot be repaired from backup",
                                        chunk_id
                                    ));
                                }
                            }
                            Err(e) => {
                                errors.push(format!("failed to read chunk {}: {}", chunk_id, e));
                            }
                        }
                    }
                    Ok(())
                }
                RecoveryAction::RebuildIndex => {
                    let chunks = self.storage.list_chunks().await.unwrap_or_default();
                    for chunk_id in &chunks {
                        if let Ok(chunk) = self.storage.read_chunk(chunk_id).await {
                            let _ = self
                                .metadata
                                .put_node(Node {
                                    id: NodeId::new(),
                                    name: format!("recovered-{}", chunk_id),
                                    kind: NodeKind::File,
                                    size: chunk.size,
                                    mode: NodePermissions::default_for("root"),
                                    created_at: chrono::Utc::now(),
                                    modified_at: chrono::Utc::now(),
                                    content_hash: Some(chunk.checksum),
                                    metadata: NodeMetadata::default(),
                                })
                                .await;
                            entries_replayed += 1;
                        }
                    }
                    Ok(())
                }
                RecoveryAction::FullReconstruction => {
                    let seq = self.journal.latest_sequence().await?;
                    struct FullHandler;
                    impl JournalHandler for FullHandler {
                        fn handle(&mut self, _entry: &JournalEntry) -> AegisResult<()> {
                            Ok(())
                        }
                    }
                    match self.journal.replay(Box::new(FullHandler)).await {
                        Ok(count) => {
                            entries_replayed = count;
                            if count < seq {
                                errors.push(format!(
                                    "only replayed {} of {} journal entries",
                                    count, seq
                                ));
                            }
                            Ok(())
                        }
                        Err(e) => {
                            errors.push(format!("full reconstruction failed: {}", e));
                            Err(e)
                        }
                    }
                }
            };

            let duration = start.elapsed().as_secs_f64();
            let success = result.is_ok() && errors.is_empty();

            let report = RecoveryReport {
                action,
                success,
                entries_replayed,
                chunks_repaired,
                errors_encountered: errors,
                duration_seconds: duration,
            };

            {
                let mut s = self.status.lock().await;
                s.in_progress = false;
                s.progress_percent = 100.0;
                s.current_action = None;
                s.last_report = Some(report.clone());
            }

            if success {
                Ok(report)
            } else {
                Err(AegisError::RecoveryError(
                    "recovery completed with errors".into(),
                ))
            }
        })
    }

    fn needs_recovery(&self) -> BoxFuture<'_, AegisResult<bool>> {
        Box::pin(async move {
            let seq = self.journal.latest_sequence().await?;
            Ok(seq > 0)
        })
    }

    fn status(&self) -> BoxFuture<'_, AegisResult<RecoveryStatus>> {
        Box::pin(async move {
            let s = self.status.lock().await;
            Ok(s.clone())
        })
    }
}

pub struct IntegrityScanner {
    storage: Arc<dyn ChunkStorage>,
}

impl IntegrityScanner {
    pub fn new(storage: Arc<dyn ChunkStorage>) -> Self {
        Self { storage }
    }

    pub async fn scan_all(&self) -> Vec<IntegrityProof> {
        let mut results = Vec::new();
        let chunks = match self.storage.list_chunks().await {
            Ok(list) => list,
            Err(_) => return results,
        };
        for chunk_id in &chunks {
            if let Ok(chunk) = self.storage.read_chunk(chunk_id).await {
                let actual = HashValue::sha256(&chunk.data);
                let valid = actual == chunk.checksum;
                results.push(IntegrityProof {
                    chunk_id: *chunk_id,
                    expected_hash: chunk.checksum,
                    actual_hash: actual,
                    valid,
                    verified_at: chrono::Utc::now(),
                });
            }
        }
        results
    }

    pub async fn scan_range(&self, start: u64, end: u64) -> Vec<IntegrityProof> {
        let all = self.scan_all().await;
        all.into_iter()
            .skip(start as usize)
            .take((end.saturating_sub(start)) as usize)
            .collect()
    }

    pub async fn repair_chunk(&self, id: &ChunkId) -> AegisResult<bool> {
        let chunk = self.storage.read_chunk(id).await?;
        if chunk.verify_integrity() {
            return Ok(false);
        }
        Err(AegisError::RecoveryError(format!(
            "chunk {} is corrupted and cannot be repaired without source data",
            id
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct MockJournalStore {
        entries: Arc<parking_lot::Mutex<Vec<JournalEntry>>>,
    }

    impl MockJournalStore {
        fn new() -> Self {
            Self {
                entries: Arc::new(parking_lot::Mutex::new(vec![])),
            }
        }
    }

    impl JournalStore for MockJournalStore {
        fn append(&self, entry: JournalEntry) -> BoxFuture<'_, AegisResult<u64>> {
            let entries = self.entries.clone();
            Box::pin(async move {
                let mut list = entries.lock();
                let seq = list.len() as u64 + 1;
                let mut e = entry;
                e.sequence = seq;
                list.push(e);
                Ok(seq)
            })
        }
        fn read_after(
            &self,
            sequence: u64,
            limit: usize,
        ) -> BoxFuture<'_, AegisResult<Vec<JournalEntry>>> {
            let entries = self.entries.clone();
            Box::pin(async move {
                let list = entries.lock();
                let result: Vec<_> = list
                    .iter()
                    .skip(sequence as usize)
                    .take(limit)
                    .cloned()
                    .collect();
                Ok(result)
            })
        }
        fn latest_sequence(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let entries = self.entries.clone();
            Box::pin(async move {
                let list = entries.lock();
                Ok(list.len() as u64)
            })
        }
        fn truncate(&self, _before_sequence: u64) -> BoxFuture<'_, AegisResult<()>> {
            Box::pin(async { Ok(()) })
        }
        fn replay(
            &self,
            mut handler: Box<dyn JournalHandler + Send>,
        ) -> BoxFuture<'_, AegisResult<u64>> {
            let entries = self.entries.clone();
            Box::pin(async move {
                let list = entries.lock();
                let mut count = 0u64;
                for entry in list.iter() {
                    handler.handle(entry)?;
                    count += 1;
                }
                Ok(count)
            })
        }
    }

    pub(crate) struct MockChunkStorage {
        chunks: Arc<parking_lot::Mutex<HashMap<ChunkId, Chunk>>>,
    }

    impl MockChunkStorage {
        pub(crate) fn new() -> Self {
            Self {
                chunks: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            }
        }
    }

    impl ChunkStorage for MockChunkStorage {
        fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
            let chunks = self.chunks.clone();
            Box::pin(async move {
                let id = chunk.id;
                chunks.lock().insert(id, chunk);
                Ok(id)
            })
        }
        fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
            let chunks = self.chunks.clone();
            let id = *id;
            Box::pin(async move {
                chunks
                    .lock()
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AegisError::ChunkNotFound(id.to_string()))
            })
        }
        fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
            let chunks = self.chunks.clone();
            let id = *id;
            Box::pin(async move {
                chunks.lock().remove(&id);
                Ok(())
            })
        }
        fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
            let chunks = self.chunks.clone();
            let id = *id;
            Box::pin(async move { Ok(chunks.lock().contains_key(&id)) })
        }
        fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
            let chunks = self.chunks.clone();
            Box::pin(async move { Ok(chunks.lock().keys().cloned().collect()) })
        }
        fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let chunks = self.chunks.clone();
            Box::pin(async move {
                let total: u64 = chunks.lock().values().map(|c| c.size).sum();
                Ok(total)
            })
        }
        fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let chunks = self.chunks.clone();
            Box::pin(async move { Ok(chunks.lock().len() as u64) })
        }
    }

    struct MockMetadataIndex;

    impl MetadataIndex for MockMetadataIndex {
        fn put_node(&self, _node: Node) -> BoxFuture<'_, AegisResult<()>> {
            Box::pin(async { Ok(()) })
        }
        fn get_node(&self, _id: &NodeId) -> BoxFuture<'_, AegisResult<Node>> {
            Box::pin(async { Err(AegisError::NodeNotFound("mock".into())) })
        }
        fn delete_node(&self, _id: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
            Box::pin(async { Ok(()) })
        }
        fn list_children(&self, _parent_id: &NodeId) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn find_by_name(
            &self,
            _parent_id: &NodeId,
            _name: &str,
        ) -> BoxFuture<'_, AegisResult<Option<Node>>> {
            Box::pin(async { Ok(None) })
        }
        fn search(&self, _query: &dyn MetadataQuery) -> BoxFuture<'_, AegisResult<Vec<Node>>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn len(&self) -> BoxFuture<'_, AegisResult<u64>> {
            Box::pin(async { Ok(0) })
        }
        fn add_child(&self, _parent: &NodeId, _child: &NodeId) -> BoxFuture<'_, AegisResult<()>> {
            Box::pin(async { Ok(()) })
        }
        fn remove_child(
            &self,
            _parent: &NodeId,
            _child: &NodeId,
        ) -> BoxFuture<'_, AegisResult<()>> {
            Box::pin(async { Ok(()) })
        }
        fn get_parent(&self, _child_id: &NodeId) -> BoxFuture<'_, AegisResult<Option<NodeId>>> {
            Box::pin(async { Ok(None) })
        }
    }

    fn make_manager() -> RecoveryManagerImpl {
        RecoveryManagerImpl::new(
            Arc::new(MockJournalStore::new()),
            Arc::new(MockChunkStorage::new()),
            Arc::new(MockMetadataIndex),
        )
    }

    #[tokio::test]
    async fn test_needs_recovery_empty_journal() {
        let manager = make_manager();
        let needs = manager.needs_recovery().await.unwrap();
        assert!(!needs);
    }

    #[tokio::test]
    async fn test_needs_recovery_with_entries() {
        let journal = Arc::new(MockJournalStore::new());
        journal
            .append(JournalEntry {
                sequence: 0,
                timestamp: chrono::Utc::now(),
                kind: JournalEntryKind::CreateNode,
                data: vec![],
                checksum: HashValue::nil(),
            })
            .await
            .unwrap();
        let manager = RecoveryManagerImpl::new(
            journal,
            Arc::new(MockChunkStorage::new()),
            Arc::new(MockMetadataIndex),
        );
        let needs = manager.needs_recovery().await.unwrap();
        assert!(needs);
    }

    #[tokio::test]
    async fn test_recovery_action_replay() {
        let journal = Arc::new(MockJournalStore::new());
        journal
            .append(JournalEntry {
                sequence: 0,
                timestamp: chrono::Utc::now(),
                kind: JournalEntryKind::CreateNode,
                data: vec![],
                checksum: HashValue::nil(),
            })
            .await
            .unwrap();
        let manager = RecoveryManagerImpl::new(
            journal,
            Arc::new(MockChunkStorage::new()),
            Arc::new(MockMetadataIndex),
        );
        let report = manager
            .recover(RecoveryAction::ReplayJournal)
            .await
            .unwrap();
        assert!(report.success);
        assert_eq!(report.entries_replayed, 1);
        assert_eq!(report.action, RecoveryAction::ReplayJournal);
        assert!(report.duration_seconds >= 0.0);
    }

    #[tokio::test]
    async fn test_recovery_action_integrity_scan() {
        let storage = Arc::new(MockChunkStorage::new());
        let data = Bytes::from("test data");
        let chunk = Chunk::new(ChunkId::from_data(&data), data);
        storage.store_chunk(chunk).await.unwrap();

        let manager = RecoveryManagerImpl::new(
            Arc::new(MockJournalStore::new()),
            storage,
            Arc::new(MockMetadataIndex),
        );
        let report = manager
            .recover(RecoveryAction::IntegrityScan)
            .await
            .unwrap();
        assert!(report.success);
    }

    #[tokio::test]
    async fn test_status_updates() {
        let manager = make_manager();
        let status = manager.status().await.unwrap();
        assert!(!status.in_progress);
        assert!(status.last_report.is_none());
    }
}
