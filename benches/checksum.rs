use criterion::{black_box, criterion_group, criterion_main, Criterion};

use aegisfs::checksum::{Sha256Hasher, Blake3Hasher, Xxh3Hasher, CombinedHasher};
use aegisfs::core::traits::Hasher;

fn bench_sha256_hash(c: &mut Criterion) {
    let hasher = Sha256Hasher::default();
    let data = vec![0xABu8; 1048576];

    c.bench_function("sha256_hash_1mb", |b| {
        b.iter(|| hasher.hash(black_box(&data)));
    });
}

fn bench_blake3_hash(c: &mut Criterion) {
    let hasher = Blake3Hasher::default();
    let data = vec![0xABu8; 1048576];

    c.bench_function("blake3_hash_1mb", |b| {
        b.iter(|| hasher.hash(black_box(&data)));
    });
}

fn bench_xxh3_hash(c: &mut Criterion) {
    let hasher = Xxh3Hasher::default();
    let data = vec![0xABu8; 1048576];

    c.bench_function("xxh3_hash_1mb", |b| {
        b.iter(|| hasher.hash(black_box(&data)));
    });
}

fn bench_combined_hash(c: &mut Criterion) {
    let hasher = CombinedHasher::default();
    let data = vec![0xABu8; 1048576];

    c.bench_function("combined_hash_1mb", |b| {
        b.iter(|| hasher.hash(black_box(&data)));
    });
}

criterion_group!(benches, bench_sha256_hash, bench_blake3_hash,
                 bench_xxh3_hash, bench_combined_hash);
criterion_main!(benches);
