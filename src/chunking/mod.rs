mod cdc;

use crate::core::error::AegisResult;
use crate::core::traits::Chunker;
use crate::core::types::ChunkDescriptor;
use sha2::Digest;

pub use cdc::ContentDefinedChunker;

#[derive(Debug, Clone)]
pub struct ChunkerConfig {
    pub min_size: u64,
    pub max_size: u64,
    pub target_size: u64,
    pub bits: u32,
    pub window_size: u64,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self {
            min_size: 4096,
            max_size: 65536,
            target_size: 16384,
            bits: 13,
            window_size: 48,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FixedSizeChunker {
    size: u64,
}

impl FixedSizeChunker {
    pub fn new(size: u64) -> Self {
        Self {
            size: size.max(512),
        }
    }
}

impl Chunker for FixedSizeChunker {
    fn chunk_data(&self, data: &[u8]) -> AegisResult<Vec<ChunkDescriptor>> {
        let mut descriptors = Vec::new();
        let mut offset = 0u64;
        let size = self.size as usize;
        while offset < data.len() as u64 {
            let chunk_size = std::cmp::min(size, (data.len() as u64 - offset) as usize);
            let mut hasher = sha2::Sha256::new();
            hasher.update(&data[offset as usize..offset as usize + chunk_size]);
            let hash = crate::core::id::ChunkId::from_bytes(hasher.finalize().into());
            descriptors.push(ChunkDescriptor::new(hash, offset, chunk_size as u64));
            offset += chunk_size as u64;
        }
        Ok(descriptors)
    }

    fn find_chunk_boundaries(&self, data: &[u8]) -> AegisResult<Vec<u64>> {
        let mut boundaries = vec![0u64];
        let mut offset = self.size;
        while offset < data.len() as u64 {
            boundaries.push(offset);
            offset += self.size;
        }
        if *boundaries.last().unwrap() != data.len() as u64 {
            boundaries.push(data.len() as u64);
        }
        Ok(boundaries)
    }

    fn estimate_chunk_count(&self, data_size: u64) -> u64 {
        data_size.div_ceil(self.size)
    }

    fn average_chunk_size(&self) -> u64 {
        self.size
    }

    fn min_chunk_size(&self) -> u64 {
        self.size
    }

    fn max_chunk_size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::Chunker;

    #[test]
    fn test_fixed_chunker_uniform() {
        let chunker = FixedSizeChunker::new(1024);
        let data = vec![0u8; 4096];
        let chunks = chunker.chunk_data(&data).unwrap();
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].size, 1024);
        assert_eq!(chunks[3].size, 1024);
    }

    #[test]
    fn test_fixed_chunker_remainder() {
        let chunker = FixedSizeChunker::new(1024);
        let data = vec![0u8; 4500];
        let chunks = chunker.chunk_data(&data).unwrap();
        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[4].size, 404);
    }

    #[test]
    fn test_fixed_chunker_empty() {
        let chunker = FixedSizeChunker::new(1024);
        let data = vec![];
        let chunks = chunker.chunk_data(&data).unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_fixed_chunker_small() {
        let chunker = FixedSizeChunker::new(1024);
        let data = vec![1u8; 100];
        let chunks = chunker.chunk_data(&data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].size, 100);
    }

    #[test]
    fn test_chunk_boundaries() {
        let chunker = FixedSizeChunker::new(1024);
        let data = vec![0u8; 2500];
        let boundaries = chunker.find_chunk_boundaries(&data).unwrap();
        assert_eq!(boundaries, vec![0, 1024, 2048, 2500]);
    }

    #[test]
    fn test_estimate_chunk_count() {
        let chunker = FixedSizeChunker::new(1024);
        assert_eq!(chunker.estimate_chunk_count(0), 0);
        assert_eq!(chunker.estimate_chunk_count(1), 1);
        assert_eq!(chunker.estimate_chunk_count(1024), 1);
        assert_eq!(chunker.estimate_chunk_count(1025), 2);
    }

    #[test]
    fn test_boundaries_small_data() {
        let chunker = FixedSizeChunker::new(1024);
        let data = vec![0u8; 500];
        let boundaries = chunker.find_chunk_boundaries(&data).unwrap();
        assert_eq!(boundaries, vec![0, 500]);
    }
}
