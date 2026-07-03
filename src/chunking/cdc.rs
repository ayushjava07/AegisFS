use crate::core::error::AegisResult;
use crate::core::id::ChunkId;
use crate::core::traits::Chunker;
use crate::core::types::ChunkDescriptor;

use super::ChunkerConfig;

pub struct ContentDefinedChunker {
    min_size: u64,
    max_size: u64,
    target_size: u64,
    mask: u64,
    window_size: u64,
}

impl ContentDefinedChunker {
    pub fn new(config: ChunkerConfig) -> Self {
        Self {
            min_size: config.min_size,
            max_size: config.max_size,
            target_size: config.target_size,
            mask: (1u64 << config.bits) - 1,
            window_size: config.window_size,
        }
    }

    fn hash_at(&self, data: &[u8], pos: usize) -> u64 {
        if pos + self.window_size as usize > data.len() {
            return 0;
        }
        let window = &data[pos..pos + self.window_size as usize];
        let h = xxhash_rust::xxh3::xxh3_64(window);
        h & self.mask
    }
}

impl Chunker for ContentDefinedChunker {
    fn chunk_data(&self, data: &[u8]) -> AegisResult<Vec<ChunkDescriptor>> {
        let mut descriptors = Vec::new();
        let data_len = data.len();

        if data_len == 0 {
            return Ok(descriptors);
        }

        let mut start = 0usize;
        while start < data_len {
            let end = if data_len - start < self.min_size as usize {
                data_len
            } else {
                let search_start = start + self.min_size as usize;
                let search_end = std::cmp::min(
                    start + self.max_size as usize,
                    data_len,
                );
                let mut split = search_end;
                for pos in (search_start..search_end).step_by(1) {
                    if self.hash_at(data, pos) == self.mask {
                        split = pos;
                        break;
                    }
                }
                split
            };

            let chunk_data = &data[start..end];
            let chunk_id = ChunkId::from_data(chunk_data);
            descriptors.push(ChunkDescriptor::new(chunk_id, start as u64, (end - start) as u64));
            start = end;
        }

        Ok(descriptors)
    }

    fn find_chunk_boundaries(&self, data: &[u8]) -> AegisResult<Vec<u64>> {
        let mut boundaries = vec![0u64];
        let data_len = data.len();
        let mut start = 0usize;

        while start < data_len {
            if data_len - start < self.min_size as usize {
                boundaries.push(data_len as u64);
                break;
            }

            let search_start = start + self.min_size as usize;
            let search_end = std::cmp::min(
                start + self.max_size as usize,
                data_len,
            );
            let mut split = search_end;
            for pos in (search_start..search_end).step_by(1) {
                if self.hash_at(data, pos) == self.mask {
                    split = pos;
                    break;
                }
            }
            boundaries.push(split as u64);
            start = split;
        }

        Ok(boundaries)
    }

    fn estimate_chunk_count(&self, data_size: u64) -> u64 {
        if data_size == 0 {
            return 0;
        }
        let est = data_size / self.target_size;
        std::cmp::max(1, est)
    }

    fn average_chunk_size(&self) -> u64 {
        self.target_size
    }

    fn min_chunk_size(&self) -> u64 {
        self.min_size
    }

    fn max_chunk_size(&self) -> u64 {
        self.max_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_chunker() -> ContentDefinedChunker {
        ContentDefinedChunker::new(ChunkerConfig {
            min_size: 64,
            max_size: 512,
            target_size: 256,
            bits: 10,
            window_size: 16,
        })
    }

    #[test]
    fn test_cdc_small_data() {
        let chunker = create_test_chunker();
        let data = vec![0u8; 50];
        let chunks = chunker.chunk_data(&data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].size, 50);
    }

    #[test]
    fn test_cdc_empty() {
        let chunker = create_test_chunker();
        let data = vec![];
        let chunks = chunker.chunk_data(&data).unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_cdc_chunk_boundaries() {
        let chunker = create_test_chunker();
        let data = vec![0u8; 1000];
        let boundaries = chunker.find_chunk_boundaries(&data).unwrap();
        assert!(boundaries.len() >= 2);
        assert_eq!(boundaries[0], 0);
        assert_eq!(*boundaries.last().unwrap(), 1000);
    }

    #[test]
    fn test_cdc_deterministic() {
        let chunker = create_test_chunker();
        let data = b"Hello, this is a test string with some repeating content for chunking purposes. The quick brown fox jumps over the lazy dog. Hello, this is a test string with some repeating content for chunking purposes. The quick brown fox jumps over the lazy dog.";

        let r1 = chunker.chunk_data(data).unwrap();
        let r2 = chunker.chunk_data(data).unwrap();
        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.offset, b.offset);
            assert_eq!(a.size, b.size);
        }
    }

    #[test]
    fn test_estimate_chunk_count() {
        let chunker = create_test_chunker();
        assert_eq!(chunker.estimate_chunk_count(0), 0);
        assert_eq!(chunker.estimate_chunk_count(256), 1);
        assert_eq!(chunker.estimate_chunk_count(1000), 3);
    }

    #[test]
    fn test_cdc_repetitive_content() {
        let chunker = create_test_chunker();
        let data = vec![0xABu8; 5000];
        let chunks = chunker.chunk_data(&data).unwrap();
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.size >= 64);
            assert!(chunk.size <= 512);
        }
    }

    #[test]
    fn test_cdc_identical_chunks() {
        let chunker = create_test_chunker();
        let data = vec![0x42u8; 2000];
        let data2 = vec![0x42u8; 2000];
        let r1 = chunker.chunk_data(&data).unwrap();
        let r2 = chunker.chunk_data(&data2).unwrap();
        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.id, b.id);
        }
    }
}
