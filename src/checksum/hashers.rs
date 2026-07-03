use std::io::Read;

use blake3;
use sha2::{Digest, Sha256};
use xxhash_rust::xxh3;

use crate::core::error::AegisResult;
use crate::core::id::HashValue;
use crate::core::traits::Hasher;

#[derive(Default, Clone, Debug)]
pub struct Sha256Hasher;

impl Hasher for Sha256Hasher {
    fn hash(&self, data: &[u8]) -> HashValue {
        HashValue::sha256(data)
    }

    fn hash_stream(&self, reader: &mut dyn Read) -> AegisResult<HashValue> {
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        Ok(HashValue::from_bytes(bytes))
    }
}

#[derive(Default, Clone, Debug)]
pub struct Blake3Hasher;

impl Hasher for Blake3Hasher {
    fn hash(&self, data: &[u8]) -> HashValue {
        let hash = blake3::hash(data);
        HashValue::from_bytes(*hash.as_bytes())
    }

    fn hash_stream(&self, reader: &mut dyn Read) -> AegisResult<HashValue> {
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0u8; 8192];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let hash = hasher.finalize();
        Ok(HashValue::from_bytes(*hash.as_bytes()))
    }
}

#[derive(Default, Clone, Debug)]
pub struct Xxh3Hasher;

impl Xxh3Hasher {
    pub fn hash_raw(data: &[u8]) -> u64 {
        xxh3::xxh3_64(data)
    }
}

impl Hasher for Xxh3Hasher {
    fn hash(&self, data: &[u8]) -> HashValue {
        let h = xxh3::xxh3_64(data);
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&h.to_le_bytes());
        HashValue::from_bytes(bytes)
    }

    fn hash_stream(&self, reader: &mut dyn Read) -> AegisResult<HashValue> {
        let mut state = xxh3::Xxh3::new();
        let mut buffer = [0u8; 8192];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            state.update(&buffer[..n]);
        }
        let h = state.digest();
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&h.to_le_bytes());
        Ok(HashValue::from_bytes(bytes))
    }
}

#[derive(Clone)]
pub struct CombinedHasher {
    sha256: Sha256,
    xxh3: xxh3::Xxh3,
}

impl CombinedHasher {
    pub fn new() -> Self {
        Self {
            sha256: Sha256::new(),
            xxh3: xxh3::Xxh3::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.sha256.update(data);
        self.xxh3.update(data);
    }

    pub fn finalize(self) -> (HashValue, u64) {
        let result = self.sha256.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        let sha = HashValue::from_bytes(bytes);
        let xxh = self.xxh3.digest();
        (sha, xxh)
    }

    pub fn sha256_hash(&self) -> HashValue {
        let result = self.sha256.clone().finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        HashValue::from_bytes(bytes)
    }

    pub fn xxh3_hash(&self) -> u64 {
        self.xxh3.clone().digest()
    }
}

impl Default for CombinedHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher for CombinedHasher {
    fn hash(&self, data: &[u8]) -> HashValue {
        HashValue::sha256(data)
    }

    fn hash_stream(&self, reader: &mut dyn Read) -> AegisResult<HashValue> {
        let mut combined = CombinedHasher::new();
        let mut buffer = [0u8; 8192];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            combined.update(&buffer[..n]);
        }
        let (sha, _xxh) = combined.finalize();
        Ok(sha)
    }
}
