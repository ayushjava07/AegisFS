use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};

use aegisfs::chunking::FixedSizeChunker;
use aegisfs::core::id::{ChunkId, HashValue};
use aegisfs::core::traits::{Chunker, DedupIndex};
use aegisfs::dedup::MemoryDedupIndex;

fn bench_dedup_insert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let index = Arc::new(MemoryDedupIndex::new());
    let chunker = FixedSizeChunker::new(64);
    let data = vec![0x42u8; 100_000];
    let chunks = chunker.chunk_data(&data).unwrap();
    let checksums: Vec<HashValue> = chunks.iter().map(|c| c.checksum).collect();
    let ids: Vec<ChunkId> = chunks.iter().map(|c| c.id).collect();

    c.bench_function("dedup_insert_1562_chunks", |b| {
        b.iter(|| {
            for i in 0..ids.len() {
                let _ = rt.block_on(index.insert(&checksums[i], &ids[i]));
            }
        });
    });
}

fn bench_dedup_lookup(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let index = Arc::new(MemoryDedupIndex::new());
    let chunker = FixedSizeChunker::new(64);
    let data = vec![0x42u8; 100_000];
    let chunks = chunker.chunk_data(&data).unwrap();
    let checksums: Vec<HashValue> = chunks.iter().map(|c| c.checksum).collect();
    let ids: Vec<ChunkId> = chunks.iter().map(|c| c.id).collect();
    for i in 0..ids.len() {
        let _ = rt.block_on(index.insert(&checksums[i], &ids[i]));
    }

    c.bench_function("dedup_lookup_1562_chunks", |b| {
        b.iter(|| {
            for checksum in &checksums {
                let _ = rt.block_on(index.contains(checksum));
            }
        });
    });
}

criterion_group!(benches, bench_dedup_insert, bench_dedup_lookup);
criterion_main!(benches);
