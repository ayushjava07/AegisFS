use aegisfs::chunking::{ChunkerConfig, ContentDefinedChunker, FixedSizeChunker};
use aegisfs::core::traits::Chunker;
use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn bench_chunkers(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunkers");

    let size = 256 * 1024;
    let data: Vec<u8> = (0..size).map(|i| (i ^ (i >> 8)) as u8).collect();
    group.throughput(Throughput::Bytes(size as u64));

    group.bench_function("fixed_size_4k", |b| {
        let chunker = FixedSizeChunker::new(4096);
        b.iter(|| chunker.chunk_data(&data).unwrap())
    });

    group.bench_function("content_defined_16k", |b| {
        let config = ChunkerConfig::default();
        let chunker = ContentDefinedChunker::new(config);
        b.iter(|| chunker.chunk_data(&data).unwrap())
    });

    group.finish();
}

criterion_group!(benches, bench_chunkers);
criterion_main!(benches);
