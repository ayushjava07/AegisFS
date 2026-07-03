use aegisfs::core::id::ChunkId;
use aegisfs::core::traits::DedupIndex;
use aegisfs::core::types::HashValue;
use aegisfs::dedup::{DedupBloomFilter, MemoryDedupIndex};
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_bloom_filter(c: &mut Criterion) {
    let mut bloom = DedupBloomFilter::new(10000, 0.01);
    let hash = HashValue::sha256(b"benchmarking-hash-value");

    c.bench_function("bloom_filter_insert", |b| {
        b.iter(|| {
            bloom.insert(&hash);
        })
    });

    c.bench_function("bloom_filter_contains", |b| {
        b.iter(|| {
            let _ = bloom.contains(&hash);
        })
    });
}

fn bench_dedup_index(c: &mut Criterion) {
    let index = MemoryDedupIndex::new();
    let hash = HashValue::sha256(b"benchmarking-hash-value");
    let chunk_id = ChunkId::nil();

    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("dedup_index_insert", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = index.insert(&hash, &chunk_id).await.unwrap();
            })
        })
    });

    c.bench_function("dedup_index_contains", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = index.contains(&hash).await.unwrap();
            })
        })
    });
}

criterion_group!(benches, bench_bloom_filter, bench_dedup_index);
criterion_main!(benches);
