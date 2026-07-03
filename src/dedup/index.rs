use std::collections::HashMap;
use std::sync::RwLock;

use crate::core::error::AegisResult;
use crate::core::id::ChunkId;
use crate::core::traits::{BoxFuture, DedupIndex};
use crate::core::types::HashValue;

pub struct MemoryDedupIndex {
    inner: RwLock<HashMap<HashValue, ChunkId>>,
}

impl MemoryDedupIndex {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: RwLock::new(HashMap::with_capacity(capacity)),
        }
    }
}

impl Default for MemoryDedupIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupIndex for MemoryDedupIndex {
    fn insert(&self, hash: &HashValue, chunk_id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
        let hash = *hash;
        let chunk_id = *chunk_id;
        Box::pin(async move {
            let mut inner = self.inner.write().map_err(|e| {
                crate::core::error::AegisError::Internal(format!("lock error: {}", e))
            })?;
            match inner.entry(hash) {
                std::collections::hash_map::Entry::Occupied(_) => Ok(false),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(chunk_id);
                    Ok(true)
                }
            }
        })
    }

    fn lookup(&self, hash: &HashValue) -> BoxFuture<'_, AegisResult<Option<ChunkId>>> {
        let hash = *hash;
        Box::pin(async move {
            let inner = self.inner.read().map_err(|e| {
                crate::core::error::AegisError::Internal(format!("lock error: {}", e))
            })?;
            Ok(inner.get(&hash).copied())
        })
    }

    fn contains(&self, hash: &HashValue) -> BoxFuture<'_, AegisResult<bool>> {
        let hash = *hash;
        Box::pin(async move {
            let inner = self.inner.read().map_err(|e| {
                crate::core::error::AegisError::Internal(format!("lock error: {}", e))
            })?;
            Ok(inner.contains_key(&hash))
        })
    }

    fn remove(&self, hash: &HashValue) -> BoxFuture<'_, AegisResult<()>> {
        let hash = *hash;
        Box::pin(async move {
            let mut inner = self.inner.write().map_err(|e| {
                crate::core::error::AegisError::Internal(format!("lock error: {}", e))
            })?;
            inner.remove(&hash);
            Ok(())
        })
    }

    fn len(&self) -> BoxFuture<'_, AegisResult<u64>> {
        Box::pin(async move {
            let inner = self.inner.read().map_err(|e| {
                crate::core::error::AegisError::Internal(format!("lock error: {}", e))
            })?;
            Ok(inner.len() as u64)
        })
    }

    fn clear(&self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move {
            let mut inner = self.inner.write().map_err(|e| {
                crate::core::error::AegisError::Internal(format!("lock error: {}", e))
            })?;
            inner.clear();
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::DedupIndex;

    #[tokio::test]
    async fn test_insert_and_lookup() {
        let index = MemoryDedupIndex::new();
        let hash = HashValue::sha256(b"test data");
        let chunk_id = ChunkId::from_data(b"test data");

        assert!(index.insert(&hash, &chunk_id).await.unwrap());
        assert!(!index.insert(&hash, &chunk_id).await.unwrap());

        let found = index.lookup(&hash).await.unwrap();
        assert_eq!(found, Some(chunk_id));
    }

    #[tokio::test]
    async fn test_contains() {
        let index = MemoryDedupIndex::new();
        let hash = HashValue::sha256(b"hello");

        assert!(!index.contains(&hash).await.unwrap());
        let cid = ChunkId::from_data(b"hello");
        index.insert(&hash, &cid).await.unwrap();
        assert!(index.contains(&hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_remove() {
        let index = MemoryDedupIndex::new();
        let hash = HashValue::sha256(b"remove me");
        let cid = ChunkId::from_data(b"remove me");

        index.insert(&hash, &cid).await.unwrap();
        assert!(index.contains(&hash).await.unwrap());

        index.remove(&hash).await.unwrap();
        assert!(!index.contains(&hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_len_and_clear() {
        let index = MemoryDedupIndex::new();
        assert_eq!(index.len().await.unwrap(), 0);

        for i in 0..10 {
            let data = format!("data{}", i);
            let hash = HashValue::sha256(data.as_bytes());
            let cid = ChunkId::from_data(data.as_bytes());
            index.insert(&hash, &cid).await.unwrap();
        }

        assert_eq!(index.len().await.unwrap(), 10);
        index.clear().await.unwrap();
        assert_eq!(index.len().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_with_capacity() {
        let index = MemoryDedupIndex::with_capacity(1000);
        assert_eq!(index.len().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_lookup_missing() {
        let index = MemoryDedupIndex::new();
        let hash = HashValue::sha256(b"missing");
        let result = index.lookup(&hash).await.unwrap();
        assert_eq!(result, None);
    }
}
