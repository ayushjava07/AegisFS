#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;

use aegisfs::chunking::FixedSizeChunker;
use aegisfs::core::traits::{ChunkStorage, DedupIndex};
use aegisfs::core::id::ChunkId;
use aegisfs::core::types::Chunk;
use aegisfs::core::error::{AegisError, AegisResult};
use aegisfs::dedup::{DedupEngine, MemoryDedupIndex};

struct FuzzStorage;

#[async_trait::async_trait]
impl ChunkStorage for FuzzStorage {
    async fn store_chunk(&self, chunk: Chunk) -> AegisResult<ChunkId> {
        Ok(chunk.id)
    }
    async fn read_chunk(&self, _id: &ChunkId) -> AegisResult<Chunk> {
        Err(AegisError::ChunkNotFound("fuzz".into()))
    }
    async fn delete_chunk(&self, _id: &ChunkId) -> AegisResult<()> { Ok(()) }
    async fn has_chunk(&self, _id: &ChunkId) -> AegisResult<bool> { Ok(false) }
    async fn list_chunks(&self) -> AegisResult<Vec<ChunkId>> { Ok(Vec::new()) }
    async fn total_size(&self) -> AegisResult<u64> { Ok(0) }
    async fn chunk_count(&self) -> AegisResult<u64> { Ok(0) }
}

fuzz_target!(|data: &[u8]| {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let index = Arc::new(MemoryDedupIndex::new());
    let chunker = Arc::new(FixedSizeChunker::new(64));
    let storage = Arc::new(FuzzStorage);
    let engine = DedupEngine::new(index, chunker, storage);

    let _ = rt.block_on(engine.ingest(data));
    let _ = rt.block_on(engine.find_duplicates(data));
});
