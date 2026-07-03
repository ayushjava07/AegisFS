use std::collections::HashSet;

use crate::core::error::AegisResult;
use crate::core::id::ChunkId;
use crate::core::traits::ChunkStorage;

#[derive(Debug, Clone, Default)]
pub struct GcStats {
    pub total_chunks: u64,
    pub referenced_chunks: u64,
    pub unreferenced_chunks: u64,
    pub deleted_chunks: u64,
    pub failed_deletions: u64,
    pub bytes_freed: u64,
}

#[derive(Debug, Clone)]
pub struct GcReport {
    pub stats: GcStats,
    pub deleted_ids: Vec<ChunkId>,
    pub failed_ids: Vec<(ChunkId, String)>,
}

pub struct GarbageCollector {
    live_ids: HashSet<ChunkId>,
}

impl GarbageCollector {
    pub fn new() -> Self {
        Self {
            live_ids: HashSet::new(),
        }
    }

    pub fn mark(&mut self, live_ids: &[ChunkId]) {
        self.live_ids.extend(live_ids.iter().copied());
    }

    pub fn mark_all(&mut self, ids: &[ChunkId]) {
        self.live_ids.extend(ids.iter().copied());
    }

    pub fn reset(&mut self) {
        self.live_ids.clear();
    }

    pub fn live_count(&self) -> usize {
        self.live_ids.len()
    }

    pub async fn sweep(&self, storage: &dyn ChunkStorage) -> AegisResult<GcReport> {
        let all_chunks = storage.list_chunks().await?;
        let total = all_chunks.len() as u64;

        let unreferenced: Vec<&ChunkId> = all_chunks
            .iter()
            .filter(|id| !self.live_ids.contains(id))
            .collect();

        let unreferenced_count = unreferenced.len() as u64;
        let referenced_count = total - unreferenced_count;

        let mut deleted_ids = Vec::new();
        let mut failed_ids = Vec::new();
        let mut deleted: u64 = 0;
        let mut failed: u64 = 0;
        let mut freed: u64 = 0;

        for chunk_id in unreferenced {
            match storage.delete_chunk(chunk_id).await {
                Ok(()) => {
                    deleted += 1;
                    freed += 32;
                    deleted_ids.push(*chunk_id);
                }
                Err(e) => {
                    failed += 1;
                    failed_ids.push((*chunk_id, e.to_string()));
                }
            }
        }

        Ok(GcReport {
            stats: GcStats {
                total_chunks: total,
                referenced_chunks: referenced_count,
                unreferenced_chunks: unreferenced_count,
                deleted_chunks: deleted,
                failed_deletions: failed,
                bytes_freed: freed,
            },
            deleted_ids,
            failed_ids,
        })
    }

    pub async fn dry_run(&self, storage: &dyn ChunkStorage) -> AegisResult<GcReport> {
        let all_chunks = storage.list_chunks().await?;
        let total = all_chunks.len() as u64;

        let unreferenced: Vec<&ChunkId> = all_chunks
            .iter()
            .filter(|id| !self.live_ids.contains(id))
            .collect();

        Ok(GcReport {
            stats: GcStats {
                total_chunks: total,
                referenced_chunks: total - unreferenced.len() as u64,
                unreferenced_chunks: unreferenced.len() as u64,
                deleted_chunks: 0,
                failed_deletions: 0,
                bytes_freed: 0,
            },
            deleted_ids: Vec::new(),
            failed_ids: Vec::new(),
        })
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Chunk;

    struct MockStorage {
        chunks: std::sync::Arc<parking_lot::Mutex<Vec<ChunkId>>>,
    }

    impl MockStorage {
        fn new(ids: Vec<ChunkId>) -> Self {
            Self {
                chunks: std::sync::Arc::new(parking_lot::Mutex::new(ids)),
            }
        }
    }

    use crate::core::error::AegisError;

    impl ChunkStorage for MockStorage {
        fn store_chunk(
            &self,
            _chunk: Chunk,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AegisResult<ChunkId>> + Send + '_>>
        {
            Box::pin(async { Err(AegisError::NotSupported("mock".into())) })
        }

        fn read_chunk(
            &self,
            _id: &ChunkId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AegisResult<Chunk>> + Send + '_>>
        {
            Box::pin(async { Err(AegisError::NotSupported("mock".into())) })
        }

        fn delete_chunk(
            &self,
            id: &ChunkId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AegisResult<()>> + Send + '_>> {
            let id = *id;
            let chunks = self.chunks.clone();
            Box::pin(async move {
                let mut guard = chunks.lock();
                guard.retain(|c| *c != id);
                Ok(())
            })
        }

        fn has_chunk(
            &self,
            _id: &ChunkId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AegisResult<bool>> + Send + '_>>
        {
            Box::pin(async { Ok(true) })
        }

        fn list_chunks(
            &self,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = AegisResult<Vec<ChunkId>>> + Send + '_>,
        > {
            let chunks = self.chunks.lock().clone();
            Box::pin(async move { Ok(chunks) })
        }

        fn total_size(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AegisResult<u64>> + Send + '_>>
        {
            Box::pin(async { Ok(0) })
        }

        fn chunk_count(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AegisResult<u64>> + Send + '_>>
        {
            let len = self.chunks.lock().len() as u64;
            Box::pin(async move { Ok(len) })
        }
    }

    #[tokio::test]
    async fn test_gc_sweep() {
        let ids: Vec<ChunkId> = (0..5).map(|i| ChunkId::from_data(&[i])).collect();
        let storage = MockStorage::new(ids.clone());
        let mut gc = GarbageCollector::new();
        gc.mark(&[ids[0], ids[1]]);
        let report = gc.sweep(&storage).await.unwrap();
        assert_eq!(report.stats.total_chunks, 5);
        assert_eq!(report.stats.referenced_chunks, 2);
        assert_eq!(report.stats.unreferenced_chunks, 3);
        assert_eq!(report.stats.deleted_chunks, 3);
    }

    #[tokio::test]
    async fn test_gc_dry_run() {
        let ids: Vec<ChunkId> = (0..3).map(|i| ChunkId::from_data(&[i])).collect();
        let storage = MockStorage::new(ids.clone());
        let mut gc = GarbageCollector::new();
        gc.mark(&ids[..2]);
        let report = gc.dry_run(&storage).await.unwrap();
        assert_eq!(report.stats.unreferenced_chunks, 1);
        assert_eq!(report.stats.deleted_chunks, 0);
    }
}
