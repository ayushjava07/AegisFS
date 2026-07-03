#![no_main]

use std::sync::Arc;

use futures::Future;
use libfuzzer_sys::fuzz_target;
use std::pin::Pin;

use aegisfs::chunking::FixedSizeChunker;
use aegisfs::core::traits::ChunkStorage;
use aegisfs::core::id::ChunkId;
use aegisfs::core::types::Chunk;
use aegisfs::core::error::{AegisError, AegisResult};
use aegisfs::dedup::{DedupEngine, MemoryDedupIndex};

struct FuzzStorage;

impl ChunkStorage for FuzzStorage {
    fn store_chunk(&self, chunk: Chunk) -> Pin<Box<dyn Future<Output = AegisResult<ChunkId>> + Send>> {
        let id = chunk.id;
        Box::pin(async move { Ok(id) })
    }
    fn read_chunk(&self, _id: &ChunkId) -> Pin<Box<dyn Future<Output = AegisResult<Chunk>> + Send>> {
        Box::pin(async move { Err(AegisError::ChunkNotFound("fuzz".into())) })
    }
    fn delete_chunk(&self, _id: &ChunkId) -> Pin<Box<dyn Future<Output = AegisResult<()>> + Send>> {
        Box::pin(async move { Ok(()) })
    }
    fn has_chunk(&self, _id: &ChunkId) -> Pin<Box<dyn Future<Output = AegisResult<bool>> + Send>> {
        Box::pin(async move { Ok(false) })
    }
    fn list_chunks(&self) -> Pin<Box<dyn Future<Output = AegisResult<Vec<ChunkId>>> + Send>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
    fn total_size(&self) -> Pin<Box<dyn Future<Output = AegisResult<u64>> + Send>> {
        Box::pin(async move { Ok(0) })
    }
    fn chunk_count(&self) -> Pin<Box<dyn Future<Output = AegisResult<u64>> + Send>> {
        Box::pin(async move { Ok(0) })
    }
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
