use aegisfs::core::traits::Serializer;
use aegisfs::core::types::{
    ArchiveId, ChunkDescriptor, ChunkId, ManifestId, Node, NodeId, NodeKind, NodeMetadata,
    NodePermissions,
};
use aegisfs::serialization::{BinSerializer, JsonSerializer, Manifest, ManifestMetadata};
use chrono::Utc;
use criterion::{criterion_group, criterion_main, Criterion};

fn test_node() -> Node {
    Node {
        id: NodeId::new(),
        name: "test-file-bench.txt".into(),
        kind: NodeKind::File,
        size: 1048576,
        mode: NodePermissions::default_for("user"),
        created_at: Utc::now(),
        modified_at: Utc::now(),
        content_hash: None,
        metadata: NodeMetadata::default(),
    }
}

fn test_manifest() -> Manifest {
    Manifest {
        id: ManifestId::new(),
        archive_id: ArchiveId::new(),
        root_node: NodeId::new(),
        chunks: (0..100u64)
            .map(|i| ChunkDescriptor::new(ChunkId::from_data(&i.to_le_bytes()), i * 4096, 4096))
            .collect(),
        total_size: 409600,
        chunk_count: 100,
        created_at: Utc::now(),
        metadata: ManifestMetadata::default(),
    }
}

fn bench_serialization_node(c: &mut Criterion) {
    let node = test_node();
    let bin = BinSerializer::new();
    let json = JsonSerializer::new();

    let mut group = c.benchmark_group("serialization_node");

    group.bench_function("binary_serialize", |b| {
        b.iter(|| bin.serialize(&node).unwrap())
    });
    group.bench_function("json_serialize", |b| {
        b.iter(|| json.serialize(&node).unwrap())
    });

    let bin_data = bin.serialize(&node).unwrap();
    let json_data = json.serialize(&node).unwrap();

    group.bench_function("binary_deserialize", |b| {
        b.iter(|| {
            let _: Node = bin.deserialize(&bin_data).unwrap();
        })
    });
    group.bench_function("json_deserialize", |b| {
        b.iter(|| {
            let _: Node = json.deserialize(&json_data).unwrap();
        })
    });

    group.finish();
}

fn bench_serialization_manifest(c: &mut Criterion) {
    let manifest = test_manifest();
    let bin = BinSerializer::new();
    let json = JsonSerializer::new();

    let mut group = c.benchmark_group("serialization_manifest");

    group.bench_function("binary_serialize", |b| {
        b.iter(|| bin.serialize(&manifest).unwrap())
    });
    group.bench_function("json_serialize", |b| {
        b.iter(|| json.serialize(&manifest).unwrap())
    });

    let bin_data = bin.serialize(&manifest).unwrap();
    let json_data = json.serialize(&manifest).unwrap();

    group.bench_function("binary_deserialize", |b| {
        b.iter(|| {
            let _: Manifest = bin.deserialize(&bin_data).unwrap();
        })
    });
    group.bench_function("json_deserialize", |b| {
        b.iter(|| {
            let _: Manifest = json.deserialize(&json_data).unwrap();
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_serialization_node,
    bench_serialization_manifest
);
criterion_main!(benches);
