use criterion::{black_box, criterion_group, criterion_main, Criterion};

use aegisfs::core::id::*;
use aegisfs::core::traits::Serializer;
use aegisfs::core::types::*;
use aegisfs::serialization::{BinSerializer, JsonSerializer};

fn bench_bin_serialize_chunk(c: &mut Criterion) {
    let serializer = BinSerializer;
    let data = vec![0xABu8; 65536];
    let chunk = Chunk::new(
        ChunkId::from_data(&data),
        bytes::Bytes::copy_from_slice(&data),
    );

    c.bench_function("bin_serialize_64kb_chunk", |b| {
        b.iter(|| serializer.serialize(black_box(&chunk)).unwrap());
    });
}

fn bench_bin_deserialize_chunk(c: &mut Criterion) {
    let serializer = BinSerializer;
    let data = vec![0xABu8; 65536];
    let chunk = Chunk::new(
        ChunkId::from_data(&data),
        bytes::Bytes::copy_from_slice(&data),
    );
    let serialized = serializer.serialize(&chunk).unwrap();

    c.bench_function("bin_deserialize_64kb_chunk", |b| {
        b.iter(|| {
            serializer
                .deserialize::<Chunk>(black_box(&serialized))
                .unwrap()
        });
    });
}

fn bench_json_serialize_manifest(c: &mut Criterion) {
    let serializer = JsonSerializer;
    let manifest = aegisfs::core::types::Manifest {
        id: ManifestId::new(),
        archive_id: ArchiveId::new(),
        parent_manifest: Some(ManifestId::new()),
        root_node: NodeId::new(),
        chunk_list: (0..100)
            .map(|i| ChunkDescriptor::new(ChunkId::nil(), i * 4096, 4096))
            .collect(),
        total_size: 409600,
        chunk_count: 100,
        created_at: chrono::Utc::now(),
        checksum: HashValue::nil(),
        metadata: std::collections::HashMap::new(),
    };

    c.bench_function("json_serialize_100_chunk_manifest", |b| {
        b.iter(|| serializer.serialize(black_box(&manifest)).unwrap());
    });
}

criterion_group!(
    benches,
    bench_bin_serialize_chunk,
    bench_bin_deserialize_chunk,
    bench_json_serialize_manifest
);
criterion_main!(benches);
