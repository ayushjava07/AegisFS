#![no_main]

use libfuzzer_sys::fuzz_target;

use aegisfs::compression::{ZstdCompression, NoopCompression};
use aegisfs::core::traits::CompressionProvider;

fuzz_target!(|data: &[u8]| {
    let zstd = ZstdCompression::with_default_level();
    let noop = NoopCompression::new();

    // Zstd roundtrip
    if let Ok(compressed) = zstd.compress(data) {
        let _ = zstd.decompress(&compressed);
    }

    // Noop roundtrip
    let compressed = noop.compress(data).unwrap();
    let _ = noop.decompress(&compressed);

    // Zstd decompress of arbitrary data
    let _ = zstd.decompress(data);
});
