#![no_main]

use libfuzzer_sys::fuzz_target;

use aegisfs::chunking::{ContentDefinedChunker, ChunkerConfig};
use aegisfs::core::traits::Chunker;

fuzz_target!(|data: &[u8]| {
    let config = ChunkerConfig {
        min_size: 64,
        max_size: 4096,
        target_size: 1024,
        bits: 10,
        window_size: 16,
    };
    let chunker = ContentDefinedChunker::new(config);
    let _ = chunker.chunk_data(data);
    let _ = chunker.find_chunk_boundaries(data);
});
