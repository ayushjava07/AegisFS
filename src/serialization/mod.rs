use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::Serializer;
use crate::core::types::*;

mod binary;
mod json;

pub use binary::BinSerializer;
pub use json::JsonSerializer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: ManifestId,
    pub archive_id: ArchiveId,
    pub root_node: NodeId,
    pub chunks: Vec<ChunkDescriptor>,
    pub total_size: u64,
    pub chunk_count: u64,
    pub created_at: DateTime<Utc>,
    pub metadata: ManifestMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestMetadata {
    pub compression: Option<CompressionAlgorithm>,
    pub encryption: Option<EncryptionAlgorithm>,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SerializationFormat {
    Binary,
    Json,
}

impl SerializationFormat {
    pub fn from_tag(tag: u8) -> AegisResult<Self> {
        match tag {
            0 => Ok(Self::Binary),
            1 => Ok(Self::Json),
            _ => Err(AegisError::DeserializationError(format!(
                "unknown serialization format tag: {}",
                tag
            ))),
        }
    }

    pub fn to_tag(self) -> u8 {
        match self {
            Self::Binary => 0,
            Self::Json => 1,
        }
    }

    pub fn detect(data: &[u8]) -> AegisResult<Self> {
        if data.is_empty() {
            return Err(AegisError::DeserializationError(
                "empty data: cannot detect format".into(),
            ));
        }
        if data.len() < 4 {
            return Err(AegisError::DeserializationError(
                "data too short: cannot detect format".into(),
            ));
        }
        if data[0] == b'{' || data[0] == b'[' {
            return Ok(Self::Json);
        }
        Ok(Self::Binary)
    }
}

pub enum SerializerChoice {
    Bin(BinSerializer),
    Json(JsonSerializer),
}

impl Serializer for SerializerChoice {
    fn serialize<T: Serialize + ?Sized>(&self, value: &T) -> AegisResult<Vec<u8>> {
        match self {
            Self::Bin(s) => s.serialize(value),
            Self::Json(s) => s.serialize(value),
        }
    }

    fn deserialize<T: serde::de::DeserializeOwned>(&self, data: &[u8]) -> AegisResult<T> {
        match self {
            Self::Bin(s) => s.deserialize(data),
            Self::Json(s) => s.deserialize(data),
        }
    }
}

pub struct SerializerFactory;

impl SerializerFactory {
    pub fn for_format(format: SerializationFormat) -> SerializerChoice {
        match format {
            SerializationFormat::Binary => SerializerChoice::Bin(BinSerializer::new()),
            SerializationFormat::Json => SerializerChoice::Json(JsonSerializer::new()),
        }
    }

    pub fn detect_and_create(data: &[u8]) -> AegisResult<SerializerChoice> {
        let format = SerializationFormat::detect(data)?;
        Ok(Self::for_format(format))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedBundle {
    pub format: SerializationFormat,
    pub data: Vec<u8>,
}

impl SerializedBundle {
    pub fn new(format: SerializationFormat, data: Vec<u8>) -> Self {
        Self { format, data }
    }

    pub fn encode(&self) -> AegisResult<Vec<u8>> {
        let mut buf = Vec::with_capacity(1 + self.data.len());
        buf.push(self.format.to_tag());
        buf.extend_from_slice(&self.data);
        Ok(buf)
    }

    pub fn decode(data: &[u8]) -> AegisResult<Self> {
        if data.is_empty() {
            return Err(AegisError::DeserializationError("empty bundle data".into()));
        }
        let format = SerializationFormat::from_tag(data[0])?;
        let payload = data[1..].to_vec();
        Ok(Self {
            format,
            data: payload,
        })
    }
}

pub fn serialize_chunk(chunk: &Chunk) -> AegisResult<Vec<u8>> {
    BinSerializer::new().serialize(chunk)
}

pub fn deserialize_chunk(data: &[u8]) -> AegisResult<Chunk> {
    let chunk: Chunk = BinSerializer::new().deserialize(data)?;
    if !chunk.verify_integrity() {
        return Err(AegisError::ChecksumMismatch {
            expected: chunk.checksum.to_string(),
            actual: HashValue::sha256(&chunk.data).to_string(),
        });
    }
    Ok(chunk)
}

pub fn serialize_manifest(manifest: &self::Manifest) -> AegisResult<Vec<u8>> {
    BinSerializer::new().serialize(manifest)
}

pub fn deserialize_manifest(data: &[u8]) -> AegisResult<self::Manifest> {
    BinSerializer::new().deserialize(data)
}

pub fn serialize_snapshot(snapshot: &Snapshot) -> AegisResult<Vec<u8>> {
    BinSerializer::new().serialize(snapshot)
}

pub fn deserialize_snapshot(data: &[u8]) -> AegisResult<Snapshot> {
    BinSerializer::new().deserialize(data)
}

pub fn serialize_node(node: &Node) -> AegisResult<Vec<u8>> {
    BinSerializer::new().serialize(node)
}

pub fn deserialize_node(data: &[u8]) -> AegisResult<Node> {
    BinSerializer::new().deserialize(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn test_chunk() -> Chunk {
        Chunk::new(
            ChunkId::from_data(b"test-data"),
            Bytes::from(b"hello-world-chunk-data".as_ref()),
        )
    }

    fn test_manifest() -> self::Manifest {
        self::Manifest {
            id: ManifestId::new(),
            archive_id: ArchiveId::new(),
            root_node: NodeId::new(),
            chunks: vec![
                ChunkDescriptor::new(ChunkId::from_data(b"chunk1"), 0, 16),
                ChunkDescriptor::new(ChunkId::from_data(b"chunk2"), 16, 32),
            ],
            total_size: 48,
            chunk_count: 2,
            created_at: Utc::now(),
            metadata: ManifestMetadata {
                compression: Some(CompressionAlgorithm::Zstd(3)),
                encryption: None,
                labels: HashMap::new(),
            },
        }
    }

    fn test_snapshot() -> Snapshot {
        Snapshot {
            id: SnapshotId::new(),
            parent: None,
            archive_id: ArchiveId::new(),
            manifest: ManifestRef {
                id: ManifestId::new(),
                root_node: NodeId::new(),
                chunk_count: 2,
                total_size: 64,
                created_at: Utc::now(),
            },
            timestamp: Utc::now(),
            labels: {
                let mut m = HashMap::new();
                m.insert("name".into(), "daily-backup".into());
                m
            },
            incremental: false,
        }
    }

    fn test_node() -> Node {
        Node {
            id: NodeId::new(),
            name: "test-file.txt".into(),
            kind: NodeKind::File,
            size: 1024,
            mode: NodePermissions::default_for("admin"),
            created_at: Utc::now(),
            modified_at: Utc::now(),
            content_hash: Some(HashValue::sha256(b"content")),
            metadata: NodeMetadata::default(),
        }
    }

    fn test_large_chunk(size: usize) -> Chunk {
        let data = vec![0xAB; size];
        let mut chunk = Chunk::new(ChunkId::from_data(&data), Bytes::from(data));
        chunk.compressed_size = Some((size / 2) as u64);
        chunk.compression_algorithm = Some(CompressionAlgorithm::Lz4);
        chunk
    }

    fn roundtrip<T, S, F>(value: &T, serialize: S, deserialize: F) -> AegisResult<T>
    where
        T: Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
        S: Fn(&T) -> AegisResult<Vec<u8>>,
        F: Fn(&[u8]) -> AegisResult<T>,
    {
        let data = serialize(value)?;
        let restored = deserialize(&data)?;
        Ok(restored)
    }

    #[test]
    fn test_chunk_roundtrip_binary() {
        let chunk = test_chunk();
        let restored = roundtrip(&chunk, serialize_chunk, deserialize_chunk).unwrap();
        assert_eq!(chunk.id, restored.id);
        assert_eq!(chunk.data, restored.data);
        assert_eq!(chunk.size, restored.size);
        assert_eq!(chunk.flags, restored.flags);
    }

    #[test]
    fn test_chunk_roundtrip_json() {
        let chunk = test_chunk();
        let serializer = JsonSerializer::new();
        let data = serializer.serialize(&chunk).unwrap();
        let restored: Chunk = serializer.deserialize(&data).unwrap();
        assert_eq!(chunk.id, restored.id);
        assert_eq!(chunk.data, restored.data);
        assert_eq!(chunk.size, restored.size);
    }

    #[test]
    fn test_manifest_roundtrip_binary() {
        let manifest = test_manifest();
        let restored = roundtrip(&manifest, serialize_manifest, deserialize_manifest).unwrap();
        assert_eq!(manifest.id, restored.id);
        assert_eq!(manifest.chunks.len(), restored.chunks.len());
        assert_eq!(manifest.total_size, restored.total_size);
    }

    #[test]
    fn test_manifest_roundtrip_json() {
        let manifest = test_manifest();
        let serializer = JsonSerializer::new();
        let data = serializer.serialize(&manifest).unwrap();
        let restored: self::Manifest = serializer.deserialize(&data).unwrap();
        assert_eq!(manifest.id, restored.id);
        assert_eq!(manifest.chunks.len(), restored.chunks.len());
    }

    #[test]
    fn test_snapshot_roundtrip_binary() {
        let snapshot = test_snapshot();
        let restored = roundtrip(&snapshot, serialize_snapshot, deserialize_snapshot).unwrap();
        assert_eq!(snapshot.id, restored.id);
        assert_eq!(snapshot.labels, restored.labels);
        assert_eq!(snapshot.incremental, restored.incremental);
    }

    #[test]
    fn test_snapshot_roundtrip_json() {
        let snapshot = test_snapshot();
        let serializer = JsonSerializer::new();
        let data = serializer.serialize(&snapshot).unwrap();
        let restored: Snapshot = serializer.deserialize(&data).unwrap();
        assert_eq!(snapshot.id, restored.id);
    }

    #[test]
    fn test_node_roundtrip_binary() {
        let node = test_node();
        let restored = roundtrip(&node, serialize_node, deserialize_node).unwrap();
        assert_eq!(node.id, restored.id);
        assert_eq!(node.name, restored.name);
        assert_eq!(node.kind, restored.kind);
        assert_eq!(node.mode.mode, restored.mode.mode);
    }

    #[test]
    fn test_node_roundtrip_json() {
        let node = test_node();
        let serializer = JsonSerializer::new();
        let data = serializer.serialize(&node).unwrap();
        let restored: Node = serializer.deserialize(&data).unwrap();
        assert_eq!(node.id, restored.id);
        assert_eq!(node.name, restored.name);
    }

    #[test]
    fn test_data_integrity_after_roundtrip() {
        let chunk = test_chunk();
        assert!(chunk.verify_integrity());
        let data = serialize_chunk(&chunk).unwrap();
        let restored: Chunk = deserialize_chunk(&data).unwrap();
        assert!(restored.verify_integrity());
        assert_eq!(chunk.checksum, restored.checksum);
    }

    #[test]
    fn test_error_on_corrupted_data() {
        let chunk = test_chunk();
        let mut data = serialize_chunk(&chunk).unwrap();
        let mid = data.len() / 2;
        data[mid] ^= 0xFF;
        let result: AegisResult<Chunk> = deserialize_chunk(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_error_on_truncated_data() {
        let chunk = test_chunk();
        let data = serialize_chunk(&chunk).unwrap();
        let truncated = &data[..data.len() / 2];
        let result: AegisResult<Chunk> = deserialize_chunk(truncated);
        assert!(result.is_err());
    }

    #[test]
    fn test_error_on_empty_data() {
        let result: AegisResult<Chunk> = deserialize_chunk(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_detection_json() {
        let node = test_node();
        let serializer = JsonSerializer::new();
        let data = serializer.serialize(&node).unwrap();
        let format = SerializationFormat::detect(&data).unwrap();
        assert_eq!(format, SerializationFormat::Json);
    }

    #[test]
    fn test_format_detection_binary() {
        let node = test_node();
        let serializer = BinSerializer::new();
        let data = serializer.serialize(&node).unwrap();
        let format = SerializationFormat::detect(&data).unwrap();
        assert_eq!(format, SerializationFormat::Binary);
    }

    #[test]
    fn test_format_detection_empty_error() {
        let result = SerializationFormat::detect(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_serializer_choice_dispatch() {
        let node = test_node();
        let choice = SerializerChoice::Bin(BinSerializer::new());
        let data = choice.serialize(&node).unwrap();
        let restored: Node = choice.deserialize(&data).unwrap();
        assert_eq!(node.id, restored.id);

        let choice = SerializerChoice::Json(JsonSerializer::new());
        let data = choice.serialize(&node).unwrap();
        let restored: Node = choice.deserialize(&data).unwrap();
        assert_eq!(node.id, restored.id);
    }

    #[test]
    fn test_serializer_factory_creates_binary() {
        let serializer = SerializerFactory::for_format(SerializationFormat::Binary);
        let node = test_node();
        let data = serializer.serialize(&node).unwrap();
        let restored: Node = serializer.deserialize(&data).unwrap();
        assert_eq!(node.id, restored.id);
    }

    #[test]
    fn test_serializer_factory_creates_json() {
        let serializer = SerializerFactory::for_format(SerializationFormat::Json);
        let node = test_node();
        let data = serializer.serialize(&node).unwrap();
        let restored: Node = serializer.deserialize(&data).unwrap();
        assert_eq!(node.id, restored.id);
    }

    #[test]
    fn test_serializer_factory_detect_and_create() {
        let node = test_node();
        let json_ser = JsonSerializer::new();
        let json_data = json_ser.serialize(&node).unwrap();

        let detected = SerializerFactory::detect_and_create(&json_data).unwrap();
        let restored: Node = detected.deserialize(&json_data).unwrap();
        assert_eq!(node.id, restored.id);
    }

    #[test]
    fn test_serialized_bundle_encode_decode() {
        let node = test_node();
        let serializer = BinSerializer::new();
        let data = serializer.serialize(&node).unwrap();

        let bundle = SerializedBundle::new(SerializationFormat::Binary, data.clone());
        let encoded = bundle.encode().unwrap();
        let decoded = SerializedBundle::decode(&encoded).unwrap();

        assert_eq!(bundle.format, decoded.format);
        assert_eq!(bundle.data, decoded.data);

        let bin = BinSerializer::new();
        let restored: Node = bin.deserialize(&decoded.data).unwrap();
        assert_eq!(node.id, restored.id);
    }

    #[test]
    fn test_serialized_bundle_format_tag() {
        let bundle = SerializedBundle::new(SerializationFormat::Json, vec![1, 2, 3]);
        let encoded = bundle.encode().unwrap();
        assert_eq!(encoded[0], 1);

        let bundle2 = SerializedBundle::new(SerializationFormat::Binary, vec![4, 5, 6]);
        let encoded2 = bundle2.encode().unwrap();
        assert_eq!(encoded2[0], 0);
    }

    #[test]
    fn test_serialized_bundle_decode_empty_error() {
        let result = SerializedBundle::decode(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_serialized_bundle_decode_invalid_tag() {
        let result = SerializedBundle::decode(&[255, 1, 2, 3]);
        assert!(result.is_err());
    }

    #[test]
    fn test_large_data_serialization() {
        let chunk = test_large_chunk(1024 * 1024);
        let data = serialize_chunk(&chunk).unwrap();
        let restored: Chunk = deserialize_chunk(&data).unwrap();
        assert_eq!(chunk.id, restored.id);
        assert_eq!(chunk.data, restored.data);
        assert_eq!(chunk.size, restored.size);
        assert_eq!(chunk.compressed_size, restored.compressed_size);
        assert_eq!(chunk.compression_algorithm, restored.compression_algorithm);
    }

    #[test]
    fn test_large_data_json() {
        let chunk = test_large_chunk(64 * 1024);
        let serializer = JsonSerializer::new();
        let data = serializer.serialize(&chunk).unwrap();
        let restored: Chunk = serializer.deserialize(&data).unwrap();
        assert_eq!(chunk.id, restored.id);
        assert_eq!(chunk.data, restored.data);
    }

    #[test]
    fn test_empty_manifest() {
        let manifest = self::Manifest {
            id: ManifestId::new(),
            archive_id: ArchiveId::new(),
            root_node: NodeId::nil(),
            chunks: vec![],
            total_size: 0,
            chunk_count: 0,
            created_at: Utc::now(),
            metadata: ManifestMetadata::default(),
        };
        let restored = roundtrip(&manifest, serialize_manifest, deserialize_manifest).unwrap();
        assert!(restored.chunks.is_empty());
        assert_eq!(restored.total_size, 0);
    }

    #[test]
    fn test_nil_ids() {
        let chunk = Chunk::new(ChunkId::nil(), Bytes::from(vec![]));
        let restored = roundtrip(&chunk, serialize_chunk, deserialize_chunk).unwrap();
        assert!(restored.id.is_nil());

        let node = Node {
            id: NodeId::nil(),
            name: String::new(),
            kind: NodeKind::File,
            size: 0,
            mode: NodePermissions::default_for(""),
            created_at: Utc::now(),
            modified_at: Utc::now(),
            content_hash: None,
            metadata: NodeMetadata::default(),
        };
        let restored = roundtrip(&node, serialize_node, deserialize_node).unwrap();
        assert_eq!(restored.id, NodeId::nil());
    }

    #[test]
    fn test_snapshot_with_parent() {
        let snapshot = Snapshot {
            id: SnapshotId::new(),
            parent: Some(SnapshotId::new()),
            archive_id: ArchiveId::new(),
            manifest: ManifestRef {
                id: ManifestId::new(),
                root_node: NodeId::new(),
                chunk_count: 10,
                total_size: 4096,
                created_at: Utc::now(),
            },
            timestamp: Utc::now(),
            labels: HashMap::new(),
            incremental: true,
        };
        let restored = roundtrip(&snapshot, serialize_snapshot, deserialize_snapshot).unwrap();
        assert!(restored.parent.is_some());
        assert!(restored.incremental);
    }

    #[test]
    fn test_node_with_full_metadata() {
        let mut metadata = NodeMetadata::default();
        metadata.labels.insert("env".into(), "production".into());
        metadata
            .attributes
            .insert("custom".into(), vec![1, 2, 3, 4]);

        let node = Node {
            id: NodeId::new(),
            name: "config.yaml".into(),
            kind: NodeKind::Symlink,
            size: 0,
            mode: NodePermissions {
                owner: "root".into(),
                group: "root".into(),
                mode: 0o777,
            },
            created_at: Utc::now(),
            modified_at: Utc::now(),
            content_hash: None,
            metadata,
        };
        let restored = roundtrip(&node, serialize_node, deserialize_node).unwrap();
        assert_eq!(restored.kind, NodeKind::Symlink);
        assert_eq!(restored.mode.mode, 0o777);
        assert_eq!(restored.metadata.labels.get("env").unwrap(), "production");
    }

    #[test]
    fn test_chunk_with_all_flags() {
        let mut chunk = test_chunk();
        chunk.flags = ChunkFlags::DELETED
            | ChunkFlags::INLINE
            | ChunkFlags::COMPACTED
            | ChunkFlags::CHECKPOINT;
        let restored = roundtrip(&chunk, serialize_chunk, deserialize_chunk).unwrap();
        assert_eq!(chunk.flags, restored.flags);
    }

    #[test]
    fn test_chunk_with_encryption() {
        let mut chunk = test_chunk();
        chunk.encryption_algorithm = Some(EncryptionAlgorithm::Aes256Gcm);
        let restored = roundtrip(&chunk, serialize_chunk, deserialize_chunk).unwrap();
        assert_eq!(
            restored.encryption_algorithm,
            Some(EncryptionAlgorithm::Aes256Gcm)
        );
    }

    #[test]
    fn test_wrong_type_deserialization() {
        let node = test_node();
        let data = serialize_node(&node).unwrap();
        let result: AegisResult<Chunk> = deserialize_chunk(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_manifest_with_large_chunk_list() {
        let chunks: Vec<ChunkDescriptor> = (0..1000)
            .map(|i| {
                let mut desc = ChunkDescriptor::new(
                    ChunkId::from_data(format!("chunk-{}", i).as_bytes()),
                    i as u64 * 4096,
                    4096,
                );
                desc.checksum = HashValue::sha256(format!("data-{}", i).as_bytes());
                desc
            })
            .collect();
        let manifest = self::Manifest {
            id: ManifestId::new(),
            archive_id: ArchiveId::new(),
            root_node: NodeId::new(),
            total_size: chunks.len() as u64 * 4096,
            chunk_count: chunks.len() as u64,
            chunks,
            created_at: Utc::now(),
            metadata: ManifestMetadata::default(),
        };
        let restored = roundtrip(&manifest, serialize_manifest, deserialize_manifest).unwrap();
        assert_eq!(restored.chunks.len(), 1000);
    }

    #[test]
    fn test_json_output_readable() {
        let node = test_node();
        let serializer = JsonSerializer::new();
        let data = serializer.serialize(&node).unwrap();
        let json_str = String::from_utf8(data).unwrap();
        assert!(json_str.contains("test-file.txt"));
        assert!(json_str.contains("File"));
    }

    #[test]
    fn test_binary_output_compact() {
        let node = test_node();
        let bin_ser = BinSerializer::new();
        let json_ser = JsonSerializer::new();
        let bin_data = bin_ser.serialize(&node).unwrap();
        let json_data = json_ser.serialize(&node).unwrap();
        assert!(bin_data.len() < json_data.len());
    }

    #[test]
    fn test_format_tag_roundtrip() {
        for format in &[SerializationFormat::Binary, SerializationFormat::Json] {
            let tag = format.to_tag();
            let recovered = SerializationFormat::from_tag(tag).unwrap();
            assert_eq!(*format, recovered);
        }
    }

    #[test]
    fn test_format_from_tag_invalid() {
        let result = SerializationFormat::from_tag(255);
        assert!(result.is_err());
    }

    #[test]
    fn test_serialized_bundle_roundtrip_with_json() {
        let node = test_node();
        let json_ser = JsonSerializer::new();
        let data = json_ser.serialize(&node).unwrap();

        let bundle = SerializedBundle::new(SerializationFormat::Json, data);
        let encoded = bundle.encode().unwrap();

        assert_eq!(encoded[0], 1);

        let decoded = SerializedBundle::decode(&encoded).unwrap();
        assert_eq!(decoded.format, SerializationFormat::Json);

        let restored: Node = json_ser.deserialize(&decoded.data).unwrap();
        assert_eq!(node.name, restored.name);
    }

    #[test]
    fn test_serializer_choice_from_format() {
        let node = test_node();
        let choice = SerializerFactory::for_format(SerializationFormat::Binary);
        let data = choice.serialize(&node).unwrap();
        let restored: Node = choice.deserialize(&data).unwrap();
        assert_eq!(node.id, restored.id);
    }
}
