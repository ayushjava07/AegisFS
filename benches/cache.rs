use std::num::NonZeroUsize;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use aegisfs::cache::LruMetadataCache;

fn bench_lru_insert(c: &mut Criterion) {
    let cache = LruMetadataCache::new(NonZeroUsize::new(1000).unwrap());

    c.bench_function("lru_insert_1000", |b| {
        b.iter(|| {
            for i in 0..1000 {
                cache.insert(black_box(i), i);
            }
        });
    });
}

fn bench_lru_lookup(c: &mut Criterion) {
    let cache = LruMetadataCache::new(NonZeroUsize::new(1000).unwrap());
    for i in 0..1000 {
        cache.insert(i, i);
    }

    c.bench_function("lru_lookup_1000", |b| {
        b.iter(|| {
            for i in 0..1000 {
                black_box(cache.get(&i));
            }
        });
    });
}

fn bench_lru_eviction(c: &mut Criterion) {
    let cache = LruMetadataCache::new(NonZeroUsize::new(100).unwrap());

    c.bench_function("lru_evict_1000_from_100", |b| {
        b.iter(|| {
            for i in 0..1000 {
                cache.insert(black_box(i), i);
            }
        });
    });
}

criterion_group!(
    benches,
    bench_lru_insert,
    bench_lru_lookup,
    bench_lru_eviction
);
criterion_main!(benches);
