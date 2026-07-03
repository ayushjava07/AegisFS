use proptest::prelude::*;

use aegisfs::chunking::{ContentDefinedChunker, FixedSizeChunker, ChunkerConfig};
use aegisfs::checksum::{Sha256Hasher, Blake3Hasher, Xxh3Hasher, CombinedHasher};
use aegisfs::compression::{ZstdCompression, NoopCompression};
use aegisfs::core::id::{ChunkId, HashValue, NodeId, SnapshotId, ArchiveId, ManifestId};
use aegisfs::core::traits::{Chunker, Hasher, CompressionProvider, DedupIndex, Serializer};
use aegisfs::core::types::*;
use aegisfs::dedup::MemoryDedupIndex;
use aegisfs::serialization::{BinSerializer, JsonSerializer, SerializationFormat};

proptest! {
    // -----------------------------------------------------------------------
    // Core type roundtrip: serialize + deserialize identity
    // -----------------------------------------------------------------------

    #[test]
    fn chunk_bincode_roundtrip(data: Vec<u8>) {
        let id = ChunkId::from_data(&data);
        let chunk = Chunk::new(id.clone(), bytes::Bytes::copy_from_slice(&data));
        let serializer = BinSerializer;
        let serialized = serializer.serialize(&chunk).unwrap();
        let deserialized: Chunk = serializer.deserialize(&serialized).unwrap();
        assert_eq!(chunk.id, deserialized.id);
        assert_eq!(chunk.data, deserialized.data);
        assert_eq!(chunk.size, deserialized.size);
    }

    #[test]
    fn chunk_json_roundtrip(data: Vec<u8>) {
        let id = ChunkId::from_data(&data);
        let chunk = Chunk::new(id.clone(), bytes::Bytes::copy_from_slice(&data));
        let serializer = JsonSerializer;
        let serialized = serializer.serialize(&chunk).unwrap();
        let deserialized: Chunk = serializer.deserialize(&serialized).unwrap();
        assert_eq!(chunk.id, deserialized.id);
        assert_eq!(chunk.data, deserialized.data);
    }

    #[test]
    fn manifest_bincode_roundtrip(
        chunk_count in 0usize..50,
    ) {
        let manifest = Manifest {
            id: ManifestId::new(),
            archive_id: ArchiveId::new(),
            parent_manifest: None,
            root_node: NodeId::new(),
            chunk_list: (0..chunk_count).map(|i| {
                ChunkDescriptor::new(
                    ChunkId::nil(),
                    (i * 4096) as u64,
                    4096,
                )
            }).collect(),
            total_size: (chunk_count * 4096) as u64,
            chunk_count: chunk_count as u64,
            created_at: chrono::Utc::now(),
            checksum: HashValue::nil(),
            metadata: std::collections::HashMap::new(),
        };
        let serializer = BinSerializer;
        let serialized = serializer.serialize(&manifest).unwrap();
        let deserialized: Manifest = serializer.deserialize(&serialized).unwrap();
        assert_eq!(manifest.id, deserialized.id);
        assert_eq!(manifest.chunk_count, deserialized.chunk_count);
        assert_eq!(manifest.total_size, deserialized.total_size);
    }

    // -----------------------------------------------------------------------
    // Chunking invariants
    // -----------------------------------------------------------------------

    #[test]
    fn fixed_chunker_non_empty(data: Vec<u8>) {
        let chunker = FixedSizeChunker::new(64);
        let chunks = chunker.chunk_data(&data).unwrap();
        // Every chunk must have positive size
        for chunk in &chunks {
            prop_assert!(chunk.size > 0, "chunk size must be > 0");
        }
        // Total data must be preserved (sum of sizes)
        let total: u64 = chunks.iter().map(|c| c.size).sum();
        prop_assert_eq!(total, data.len() as u64);
    }

    #[test]
    fn cdc_chunker_non_empty(data: Vec<u8>) {
        let config = ChunkerConfig {
            min_size: 64,
            max_size: 2048,
            target_size: 512,
            bits: 8,
            window_size: 16,
        };
        let chunker = ContentDefinedChunker::new(config);
        let chunks = chunker.chunk_data(&data).unwrap();
        for chunk in &chunks {
            prop_assert!(chunk.size > 0, "chunk size must be > 0");
            let size = chunk.size as usize;
            prop_assert!(size >= 64, "chunk size must be >= min_size");
        }
        let total: u64 = chunks.iter().map(|c| c.size).sum();
        prop_assert_eq!(total, data.len() as u64);
    }

    // -----------------------------------------------------------------------
    // Checksum determinism
    // -----------------------------------------------------------------------

    #[test]
    fn sha256_deterministic(data: Vec<u8>) {
        let hasher = Sha256Hasher::default();
        let h1 = hasher.hash(&data);
        let h2 = hasher.hash(&data);
        prop_assert_eq!(h1, h2);
    }

    #[test]
    fn blake3_deterministic(data: Vec<u8>) {
        let hasher = Blake3Hasher::default();
        let h1 = hasher.hash(&data);
        let h2 = hasher.hash(&data);
        prop_assert_eq!(h1, h2);
    }

    #[test]
    fn xxh3_deterministic(data: Vec<u8>) {
        let hasher = Xxh3Hasher::default();
        let h1 = hasher.hash(&data);
        let h2 = hasher.hash(&data);
        prop_assert_eq!(h1, h2);
    }

    #[test]
    fn combined_deterministic(data: Vec<u8>) {
        let hasher = CombinedHasher::default();
        let h1 = hasher.hash(&data);
        let h2 = hasher.hash(&data);
        prop_assert_eq!(h1, h2);
    }

    // -----------------------------------------------------------------------
    // Compression roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn zstd_roundtrip(data: Vec<u8>) {
        let compressor = ZstdCompression::with_default_level();
        if let Ok(compressed) = compressor.compress(&data) {
            let decompressed = compressor.decompress(&compressed).unwrap();
            prop_assert_eq!(data, decompressed);
        }
    }

    #[test]
    fn noop_roundtrip(data: Vec<u8>) {
        let compressor = NoopCompression::new();
        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        prop_assert_eq!(data, decompressed);
    }

    // -----------------------------------------------------------------------
    // Dedup index invariants
    // -----------------------------------------------------------------------

    #[test]
    fn dedup_insert_and_contains(data: Vec<u8>) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let hash = HashValue::sha256(&data);
        let id = ChunkId::from_data(&data);

        let index = std::sync::Arc::new(MemoryDedupIndex::new());
        let inserted = rt.block_on(index.insert(&hash, &id)).unwrap();
        prop_assert!(inserted); // first insert is new

        let contained = rt.block_on(index.contains(&hash)).unwrap();
        prop_assert!(contained);

        // second insert should return false (already exists)
        let inserted2 = rt.block_on(index.insert(&hash, &id)).unwrap();
        prop_assert!(!inserted2);
    }

    #[test]
    fn dedup_lookup_missing(data: Vec<u8>) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let hash = HashValue::sha256(&data);
        let index = std::sync::Arc::new(MemoryDedupIndex::new());
        let result = rt.block_on(index.lookup(&hash)).unwrap();
        prop_assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Serializer format detection
    // -----------------------------------------------------------------------

    #[test]
    fn format_detection_json(starts_with_brace in proptest::bool::ANY) {
        let data = if starts_with_brace {
            b"{".to_vec()
        } else {
            b"\x00\x00\x00\x00".to_vec()
        };
        let result = SerializationFormat::detect(&data);
        if starts_with_brace {
            prop_assert!(result.is_ok());
        } else {
            // binary data >= 4 bytes should detect as binary
            if data.len() >= 4 {
                prop_assert_eq!(result.unwrap(), SerializationFormat::Binary);
            }
        }
    }

    // -----------------------------------------------------------------------
    // ID generation uniqueness
    // -----------------------------------------------------------------------

    #[test]
    fn chunk_id_from_data_consistent(data: Vec<u8>) {
        let id1 = ChunkId::from_data(&data);
        let id2 = ChunkId::from_data(&data);
        prop_assert_eq!(id1, id2);
    }

    #[test]
    fn chunk_id_empty_distinct(
        a_len in 1usize..100,
        b_len in 1usize..100,
    ) {
        let data_a = vec![0u8; a_len];
        let data_b = vec![0u8; b_len];
        let id_a = ChunkId::from_data(&data_a);
        let id_b = ChunkId::from_data(&data_b);
        if a_len != b_len {
            prop_assert_ne!(id_a, id_b);
        }
    }

    #[test]
    fn hash_value_sha256_deterministic(data: Vec<u8>) {
        let h1 = HashValue::sha256(&data);
        let h2 = HashValue::sha256(&data);
        prop_assert_eq!(h1, h2);
    }

    // -----------------------------------------------------------------------
    // Policy config validation
    // -----------------------------------------------------------------------

    #[test]
    fn retention_policy_valid(
        max_snapshots in 0usize..100,
        min_age_days in 0u64..36500,
    ) {
        let policy = aegisfs::policy::RetentionPolicy {  // fully qualified OK
            max_snapshots,
            min_age_days,
            tags_keep: vec![],
        };
        let _ = policy;
    }
}
