#![no_main]
use libfuzzer_sys::fuzz_target;
use aegisfs::chunking::{ContentDefinedChunker, ChunkerConfig, FixedSizeChunker};
use aegisfs::core::traits::Chunker;

fuzz_target!(|data: &[u8]| {
    // Fuzz FixedSizeChunker
    if data.len() >= 8 {
        let size = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let size = (size % 65536).max(512);
        let chunker = FixedSizeChunker::new(size);
        let payload = &data[8..];
        let _ = chunker.chunk_data(payload);
        let _ = chunker.find_chunk_boundaries(payload);
    }

    // Fuzz ContentDefinedChunker
    if data.len() >= 28 {
        let min_size = u64::from_le_bytes(data[0..8].try_into().unwrap()) % 1024;
        let max_size = u64::from_le_bytes(data[8..16].try_into().unwrap()) % 8192;
        let target_size = u64::from_le_bytes(data[16..24].try_into().unwrap()) % 4096;
        let bits = u32::from_le_bytes(data[24..28].try_into().unwrap()) % 16;
        let min_size = min_size.max(16);
        let max_size = max_size.max(min_size + 16);
        let target_size = target_size.max(min_size).min(max_size);
        let bits = bits.max(4);

        let config = ChunkerConfig {
            min_size,
            max_size,
            target_size,
            bits,
            window_size: 16,
        };
        let chunker = ContentDefinedChunker::new(config);
        let payload = &data[28..];
        let _ = chunker.chunk_data(payload);
        let _ = chunker.find_chunk_boundaries(payload);
    }
});
