use aegisfs::checksum::{Blake3Hasher, CombinedHasher, Sha256Hasher, Xxh3Hasher};
use aegisfs::core::traits::Hasher;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn bench_checksums(c: &mut Criterion) {
    let mut group = c.benchmark_group("checksums");

    for size in &[1024, 65536, 1024 * 1024] {
        let data = vec![0u8; *size];
        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_with_input(BenchmarkId::new("sha256", size), size, |b, _| {
            let hasher = Sha256Hasher;
            b.iter(|| hasher.hash(&data))
        });

        group.bench_with_input(BenchmarkId::new("blake3", size), size, |b, _| {
            let hasher = Blake3Hasher;
            b.iter(|| hasher.hash(&data))
        });

        group.bench_with_input(BenchmarkId::new("xxh3", size), size, |b, _| {
            let hasher = Xxh3Hasher;
            b.iter(|| hasher.hash(&data))
        });

        group.bench_with_input(BenchmarkId::new("combined", size), size, |b, _| {
            let hasher = CombinedHasher::new();
            b.iter(|| {
                let mut h = hasher.clone();
                h.update(&data);
                let _ = h.finalize();
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_checksums);
criterion_main!(benches);
