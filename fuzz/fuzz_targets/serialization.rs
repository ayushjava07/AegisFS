#![no_main]

use libfuzzer_sys::fuzz_target;

use aegisfs::serialization::{BinSerializer, JsonSerializer};
use aegisfs::core::types::*;
use aegisfs::core::id::*;

fuzz_target!(|data: &[u8]| {
    let bin = BinSerializer;

    // Try deserializing as various types (will fail gracefully for most inputs)
    let _ = bin.deserialize::<Chunk>(data);
    let _ = bin.deserialize::<Node>(data);
    let _ = bin.deserialize::<Manifest>(data);
    let _ = bin.deserialize::<Snapshot>(data);

    let json = JsonSerializer;
    let _ = json.deserialize::<Chunk>(data);
    let _ = json.deserialize::<Node>(data);
    let _ = json.deserialize::<Manifest>(data);

    // Roundtrip test with empty/invalid data
    let mut buf = Vec::new();
    buf.extend_from_slice(data);
    let _ = bin.deserialize::<Chunk>(&buf);
});
