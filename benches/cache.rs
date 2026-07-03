use aegisfs::cache::LruMetadataCache;
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_cache_get_hit(c: &mut Criterion) {
    let cache = LruMetadataCache::with_capacity(1000);
    for i in 0..1000 {
        cache.insert(i, format!("value-{}", i));
    }
    c.bench_function("cache_get_hit", |b| {
        b.iter(|| {
            for i in 0..1000 {
                let _ = cache.get(&i);
            }
        })
    });
}

fn bench_cache_insert_evict(c: &mut Criterion) {
    let cache = LruMetadataCache::with_capacity(100);
    c.bench_function("cache_insert_evict", |b| {
        b.iter(|| {
            for i in 0..200 {
                cache.insert(i, format!("value-{}", i));
            }
        })
    });
}

criterion_group!(benches, bench_cache_get_hit, bench_cache_insert_evict);
criterion_main!(benches);
