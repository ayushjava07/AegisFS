use criterion::{black_box, criterion_group, criterion_main, Criterion};

use aegisfs::chunking::{ChunkerConfig, ContentDefinedChunker, FixedSizeChunker};
use aegisfs::core::traits::Chunker;

fn bench_fixed_chunker(c: &mut Criterion) {
    let chunker = FixedSizeChunker::new(16384);
    let data = vec![0xABu8; 1_000_000];

    c.bench_function("fixed_chunker_1mb", |b| {
        b.iter(|| chunker.chunk_data(black_box(&data)).unwrap());
    });
}

fn bench_cdc_chunker(c: &mut Criterion) {
    let config = ChunkerConfig {
        min_size: 4096,
        max_size: 65536,
        target_size: 16384,
        bits: 13,
        window_size: 48,
    };
    let chunker = ContentDefinedChunker::new(config);
    let data = vec![0xABu8; 1_000_000];

    c.bench_function("cdc_chunker_1mb", |b| {
        b.iter(|| chunker.chunk_data(black_box(&data)).unwrap());
    });
}

fn bench_cdc_repetitive(c: &mut Criterion) {
    let config = ChunkerConfig {
        min_size: 4096,
        max_size: 65536,
        target_size: 16384,
        bits: 13,
        window_size: 48,
    };
    let chunker = ContentDefinedChunker::new(config);
    let data: Vec<u8> = (0..1_000_000).map(|i| (i % 251) as u8).collect();

    c.bench_function("cdc_chunker_repetitive_1mb", |b| {
        b.iter(|| chunker.chunk_data(black_box(&data)).unwrap());
    });
}

criterion_group!(
    benches,
    bench_fixed_chunker,
    bench_cdc_chunker,
    bench_cdc_repetitive
);
criterion_main!(benches);
