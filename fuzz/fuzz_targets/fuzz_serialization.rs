#![no_main]
use libfuzzer_sys::fuzz_target;
use aegisfs::serialization::{
    deserialize_chunk, deserialize_manifest, deserialize_snapshot, deserialize_node,
    SerializedBundle, SerializationFormat,
};

fuzz_target!(|data: &[u8]| {
    // Try deserializing as chunk (has checksum verification)
    let _ = deserialize_chunk(data);

    // Try deserializing as manifest
    let _ = deserialize_manifest(data);

    // Try deserializing as snapshot
    let _ = deserialize_snapshot(data);

    // Try deserializing as node
    let _ = deserialize_node(data);

    // Try serialized bundle decoding
    if let Ok(bundle) = SerializedBundle::decode(data) {
        let _ = bundle.encode();
    }

    // Try detecting serialization format
    let _ = SerializationFormat::detect(data);
});
