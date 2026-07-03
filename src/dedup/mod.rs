pub mod index;
mod bloom;

use std::sync::Arc;

use crate::core::error::{AegisError, AegisResult};
use crate::core::id::ChunkId;
use crate::core::traits::{BoxFuture, ChunkStorage, Chunker, DedupIndex};
use crate::core::types::{Chunk, ChunkDescriptor, HashValue};

pub use index::MemoryDedupIndex;
pub use bloom::DedupBloomFilter;


pub struct DedupEngine {
    index: Arc<dyn DedupIndex>,
    chunker: Arc<dyn Chunker>,
    storage: Arc<dyn ChunkStorage>,
}

impl DedupEngine {
    pub fn new(
        index: Arc<dyn DedupIndex>,
        chunker: Arc<dyn Chunker>,
        storage: Arc<dyn ChunkStorage>,
    ) -> Self {
        Self {
            index,
            chunker,
            storage,
        }
    }

    pub async fn ingest(&self, data: &[u8]) -> AegisResult<Vec<ChunkId>> {
        let descriptors = self.chunker.chunk_data(data)?;
        let mut stored_ids = Vec::with_capacity(descriptors.len());

        for desc in &descriptors {
            let hash = HashValue::sha256(&data[desc.offset as usize..][..desc.size as usize]);
            if self.index.contains(&hash).await? {
                stored_ids.push(desc.id);
                continue;
            }

            let chunk_data = &data[desc.offset as usize..][..desc.size as usize];
            let chunk = Chunk::new(desc.id, bytes::Bytes::copy_from_slice(chunk_data));
            self.storage.store_chunk(chunk).await?;
            self.index.insert(&hash, &desc.id).await?;
            stored_ids.push(desc.id);
        }

        Ok(stored_ids)
    }

    pub async fn ingest_with_chunker(
        &self,
        data: &[u8],
        chunker: &dyn Chunker,
    ) -> AegisResult<Vec<ChunkId>> {
        let descriptors = chunker.chunk_data(data)?;
        let mut stored_ids = Vec::with_capacity(descriptors.len());

        for desc in &descriptors {
            let chunk_data = &data[desc.offset as usize..][..desc.size as usize];
            let hash = HashValue::sha256(chunk_data);

            if self.index.contains(&hash).await? {
                stored_ids.push(desc.id);
                continue;
            }

            let chunk = Chunk::new(desc.id, bytes::Bytes::copy_from_slice(chunk_data));
            self.storage.store_chunk(chunk).await?;
            self.index.insert(&hash, &desc.id).await?;
            stored_ids.push(desc.id);
        }

        Ok(stored_ids)
    }

    pub async fn find_duplicates(&self, data: &[u8]) -> AegisResult<Vec<ChunkDescriptor>> {
        let descriptors = self.chunker.chunk_data(data)?;
        let mut duplicates = Vec::new();

        for desc in &descriptors {
            let chunk_data = &data[desc.offset as usize..][..desc.size as usize];
            let hash = HashValue::sha256(chunk_data);
            if self.index.contains(&hash).await? {
                duplicates.push(desc.clone());
            }
        }

        Ok(duplicates)
    }

    pub fn index(&self) -> &Arc<dyn DedupIndex> {
        &self.index
    }

    pub fn chunker(&self) -> &Arc<dyn Chunker> {
        &self.chunker
    }

    pub fn storage(&self) -> &Arc<dyn ChunkStorage> {
        &self.storage
    }
}

#[derive(Debug, Clone)]
pub struct DedupStats {
    pub total_chunks: u64,
    pub unique_chunks: u64,
    pub duplicate_count: u64,
    pub dedup_ratio: f64,
    pub total_size: u64,
    pub deduped_size: u64,
}

impl DedupStats {
    pub fn new(
        total_chunks: u64,
        unique_chunks: u64,
        total_size: u64,
        deduped_size: u64,
    ) -> Self {
        let duplicate_count = total_chunks.saturating_sub(unique_chunks);
        let dedup_ratio = if total_chunks > 0 {
            duplicate_count as f64 / total_chunks as f64
        } else {
            0.0
        };
        Self {
            total_chunks,
            unique_chunks,
            duplicate_count,
            dedup_ratio,
            total_size,
            deduped_size,
        }
    }
}

pub struct NullDedupIndex;

impl DedupIndex for NullDedupIndex {
    fn insert(&self, _hash: &HashValue, _chunk_id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
        Box::pin(async { Ok(true) })
    }

    fn lookup(&self, _hash: &HashValue) -> BoxFuture<'_, AegisResult<Option<ChunkId>>> {
        Box::pin(async { Ok(None) })
    }

    fn contains(&self, _hash: &HashValue) -> BoxFuture<'_, AegisResult<bool>> {
        Box::pin(async { Ok(false) })
    }

    fn remove(&self, _hash: &HashValue) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn len(&self) -> BoxFuture<'_, AegisResult<u64>> {
        Box::pin(async { Ok(0) })
    }

    fn clear(&self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::FixedSizeChunker;
    use crate::core::traits::DedupIndex;
    use std::sync::Arc;

    struct MockStorage;

    impl ChunkStorage for MockStorage {
        fn store_chunk(&self, _chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
            Box::pin(async { Ok(ChunkId::nil()) })
        }

        fn read_chunk(&self, _id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
            Box::pin(async { Err(AegisError::ChunkNotFound("mock".into())) })
        }

        fn delete_chunk(&self, _id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
            Box::pin(async { Ok(()) })
        }

        fn has_chunk(&self, _id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
            Box::pin(async { Ok(false) })
        }

        fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    #[tokio::test]
    async fn test_dedup_engine_ingest() {
        let index = Arc::new(MemoryDedupIndex::new());
        let chunker = Arc::new(FixedSizeChunker::new(1024));
        let storage = Arc::new(MockStorage);
        let engine = DedupEngine::new(index, chunker, storage);

        let data = vec![0u8; 4096];
        let ids = engine.ingest(&data).await.unwrap();
        assert_eq!(ids.len(), 4);
    }

    #[tokio::test]
    async fn test_dedup_engine_duplicate() {
        let index = Arc::new(MemoryDedupIndex::new());
        let chunker = Arc::new(FixedSizeChunker::new(1024));
        let storage = Arc::new(MockStorage);
        let engine = DedupEngine::new(index.clone(), chunker, storage);

        let data = vec![0u8; 1024];
        let ids1 = engine.ingest(&data).await.unwrap();
        assert_eq!(ids1.len(), 1);

        let ids2 = engine.ingest(&data).await.unwrap();
        assert_eq!(ids2.len(), 1);
        assert_eq!(ids1[0], ids2[0]);
    }

    #[test]
    fn test_dedup_stats() {
        let stats = DedupStats::new(100, 60, 1_000_000, 600_000);
        assert_eq!(stats.total_chunks, 100);
        assert_eq!(stats.unique_chunks, 60);
        assert_eq!(stats.duplicate_count, 40);
        assert!((stats.dedup_ratio - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_null_dedup_index() {
        let index = NullDedupIndex;
        let hash = HashValue::sha256(b"test");
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            assert!(index.contains(&hash).await.unwrap() == false);
            assert!(index.insert(&hash, &ChunkId::nil()).await.unwrap());
            assert!(index.contains(&hash).await.unwrap() == false);
            assert!(index.remove(&hash).await.is_ok());
            assert_eq!(index.len().await.unwrap(), 0);
        });
    }

    #[tokio::test]
    async fn test_dedup_engine_empty() {
        let index = Arc::new(MemoryDedupIndex::new());
        let chunker = Arc::new(FixedSizeChunker::new(1024));
        let storage = Arc::new(MockStorage);
        let engine = DedupEngine::new(index, chunker, storage);

        let ids = engine.ingest(&[]).await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn test_find_duplicates() {
        let index = Arc::new(MemoryDedupIndex::new());
        let chunker = Arc::new(FixedSizeChunker::new(64));
        let storage = Arc::new(MockStorage);
        let engine = DedupEngine::new(index.clone(), chunker.clone(), storage);

        let data = vec![0xABu8; 256];
        engine.ingest(&data).await.unwrap();
        let dups = engine.find_duplicates(&data).await.unwrap();
        // All chunks should be found as duplicates since the data is identical
        assert!(!dups.is_empty(), "duplicates should be found");
        assert!(dups.len() <= 4, "at most 4 chunks in 256 bytes with 64-byte chunker");
    }
}
