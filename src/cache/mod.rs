use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bitvec::vec::BitVec;
use lru::LruCache;
use parking_lot::Mutex;
use xxhash_rust::xxh3;

use crate::core::error::AegisResult;
use crate::core::traits::{BoxFuture, CacheBackend};

const LN_2: f64 = std::f64::consts::LN_2;
const LN_2_SQ: f64 = LN_2 * LN_2;

fn optimal_bit_count(expected_items: usize, fp_rate: f64) -> usize {
    let n = expected_items.max(1) as f64;
    (-(n * fp_rate.ln()) / LN_2_SQ).ceil() as usize
}

fn optimal_hash_count(num_bits: usize, expected_items: usize) -> usize {
    let m = num_bits as f64;
    let n = expected_items.max(1) as f64;
    ((m / n) * LN_2).ceil().max(1.0) as usize
}

/// Tracks cache performance metrics.
#[derive(Debug)]
pub struct CacheStats {
    hits: AtomicUsize,
    misses: AtomicUsize,
    evictions: AtomicUsize,
    size: AtomicUsize,
}

impl Default for CacheStats {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for CacheStats {
    fn clone(&self) -> Self {
        Self {
            hits: AtomicUsize::new(self.hits()),
            misses: AtomicUsize::new(self.misses()),
            evictions: AtomicUsize::new(self.evictions()),
            size: AtomicUsize::new(self.size()),
        }
    }
}

impl CacheStats {
    pub fn new() -> Self {
        CacheStats {
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            evictions: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
        }
    }

    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_size(&self, size: usize) {
        self.size.store(size, Ordering::Relaxed);
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> usize {
        self.misses.load(Ordering::Relaxed)
    }

    pub fn evictions(&self) -> usize {
        self.evictions.load(Ordering::Relaxed)
    }

    pub fn size(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    pub fn hit_ratio(&self) -> f64 {
        let h = self.hits();
        let total = h + self.misses();
        if total == 0 {
            0.0
        } else {
            h as f64 / total as f64
        }
    }

    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.size.store(0, Ordering::Relaxed);
    }
}

/// LRU cache for metadata, parameterized by key and value types.
pub struct LruMetadataCache<K, V> {
    inner: Mutex<LruCache<K, V>>,
    stats: CacheStats,
}

impl<K, V> LruMetadataCache<K, V>
where
    K: Hash + Eq + Send + Sync,
    V: Clone + Send + Sync,
{
    pub fn new(capacity: NonZeroUsize) -> Self {
        LruMetadataCache {
            inner: Mutex::new(LruCache::new(capacity)),
            stats: CacheStats::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity).expect("LruMetadataCache capacity must be > 0");
        Self::new(cap)
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.inner.lock();
        match cache.get(key) {
            Some(v) => {
                self.stats.record_hit();
                Some(v.clone())
            }
            None => {
                self.stats.record_miss();
                None
            }
        }
    }

    pub fn insert(&self, key: K, value: V) {
        let mut cache = self.inner.lock();
        let existed = cache.contains(&key);
        let evicted = cache.push(key, value);
        if !existed && evicted.is_some() {
            self.stats.record_eviction();
        }
        self.stats.set_size(cache.len());
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        let mut cache = self.inner.lock();
        let val = cache.pop(key);
        self.stats.set_size(cache.len());
        val
    }

    pub fn clear(&self) {
        self.inner.lock().clear();
        self.stats.set_size(0);
    }

    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> NonZeroUsize {
        self.inner.lock().cap()
    }

    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }
}

impl<K, V> CacheBackend<K, V> for LruMetadataCache<K, V>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn get(&self, key: &K) -> BoxFuture<'_, AegisResult<Option<V>>> {
        let result = LruMetadataCache::get(self, key);
        Box::pin(async move { Ok(result) })
    }

    fn insert(&self, key: K, value: V) -> BoxFuture<'_, AegisResult<()>> {
        LruMetadataCache::insert(self, key, value);
        Box::pin(async move { Ok(()) })
    }

    fn remove(&self, key: &K) -> BoxFuture<'_, AegisResult<()>> {
        LruMetadataCache::remove(self, key);
        Box::pin(async move { Ok(()) })
    }

    fn clear(&self) -> BoxFuture<'_, AegisResult<()>> {
        LruMetadataCache::clear(self);
        Box::pin(async move { Ok(()) })
    }

    fn len(&self) -> BoxFuture<'_, AegisResult<usize>> {
        let len = LruMetadataCache::len(self);
        Box::pin(async move { Ok(len) })
    }
}

/// A bloom-filter based cache for quick existence checks.
pub struct BloomCache {
    bits: Mutex<BitVec>,
    num_hashes: usize,
    num_bits: usize,
    count: AtomicUsize,
    stats: CacheStats,
}

impl BloomCache {
    pub fn new(expected_items: usize, false_positive_rate: f64) -> Self {
        let num_bits = optimal_bit_count(expected_items, false_positive_rate);
        let num_hashes = optimal_hash_count(num_bits, expected_items);
        BloomCache {
            bits: Mutex::new(bitvec::bitvec![0; num_bits]),
            num_hashes,
            num_bits,
            count: AtomicUsize::new(0),
            stats: CacheStats::new(),
        }
    }

    fn hash_indices(&self, item: &[u8]) -> Vec<usize> {
        (0..self.num_hashes)
            .map(|i| xxh3::xxh3_64_with_seed(item, i as u64) as usize % self.num_bits)
            .collect()
    }

    pub fn insert(&self, item: &[u8]) {
        let indices = self.hash_indices(item);
        let mut bits = self.bits.lock();
        for &idx in &indices {
            bits.set(idx, true);
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        self.stats.set_size(self.count.load(Ordering::Relaxed));
    }

    pub fn contains(&self, item: &[u8]) -> bool {
        let indices = self.hash_indices(item);
        let bits = self.bits.lock();
        indices.iter().all(|&idx| bits[idx])
    }

    pub fn clear(&self) {
        self.bits.lock().fill(false);
        self.count.store(0, Ordering::Relaxed);
        self.stats.set_size(0);
    }

    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn num_bits(&self) -> usize {
        self.num_bits
    }

    pub fn num_hashes(&self) -> usize {
        self.num_hashes
    }

    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }
}

/// Trait for a backing store that can be used with TwoTierCache.
pub trait BackingStore<K, V>: Send + Sync {
    fn fetch(&self, key: &K) -> AegisResult<Option<V>>;
}

impl<K, V, F> BackingStore<K, V> for F
where
    F: Send + Sync + Fn(&K) -> AegisResult<Option<V>>,
{
    fn fetch(&self, key: &K) -> AegisResult<Option<V>> {
        (self)(key)
    }
}

/// Two-tier cache combining an in-memory LRU with a backing store.
/// On cache miss, fetches from backing store and populates the LRU.
pub struct TwoTierCache<K, V> {
    lru: LruMetadataCache<K, V>,
    store: Arc<dyn BackingStore<K, V>>,
}

impl<K, V> TwoTierCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync,
    V: Clone + Send + Sync,
{
    pub fn new(capacity: NonZeroUsize, store: Arc<dyn BackingStore<K, V>>) -> Self {
        TwoTierCache {
            lru: LruMetadataCache::new(capacity),
            store,
        }
    }

    pub fn with_capacity(capacity: usize, store: Arc<dyn BackingStore<K, V>>) -> Self {
        let cap = NonZeroUsize::new(capacity).expect("TwoTierCache capacity must be > 0");
        Self::new(cap, store)
    }

    pub fn get(&self, key: &K) -> AegisResult<Option<V>> {
        if let Some(value) = self.lru.get(key) {
            return Ok(Some(value));
        }
        match self.store.fetch(key)? {
            Some(value) => {
                self.lru.insert(key.clone(), value.clone());
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    pub fn insert(&self, key: K, value: V) {
        self.lru.insert(key, value);
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        self.lru.remove(key)
    }

    pub fn clear(&self) {
        self.lru.clear();
    }

    pub fn len(&self) -> usize {
        self.lru.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lru.is_empty()
    }

    pub fn stats(&self) -> &CacheStats {
        self.lru.stats()
    }

    pub fn backing_store(&self) -> &Arc<dyn BackingStore<K, V>> {
        &self.store
    }
}

impl<K, V> CacheBackend<K, V> for TwoTierCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn get(&self, key: &K) -> BoxFuture<'_, AegisResult<Option<V>>> {
        let result = TwoTierCache::get(self, key);
        Box::pin(async move { result })
    }

    fn insert(&self, key: K, value: V) -> BoxFuture<'_, AegisResult<()>> {
        TwoTierCache::insert(self, key, value);
        Box::pin(async move { Ok(()) })
    }

    fn remove(&self, key: &K) -> BoxFuture<'_, AegisResult<()>> {
        TwoTierCache::remove(self, key);
        Box::pin(async move { Ok(()) })
    }

    fn clear(&self) -> BoxFuture<'_, AegisResult<()>> {
        TwoTierCache::clear(self);
        Box::pin(async move { Ok(()) })
    }

    fn len(&self) -> BoxFuture<'_, AegisResult<usize>> {
        let len = TwoTierCache::len(self);
        Box::pin(async move { Ok(len) })
    }
}

/// Cache that does nothing — always misses.
pub struct NoopCache<K, V> {
    _phantom: std::marker::PhantomData<(K, V)>,
}

impl<K, V> NoopCache<K, V>
where
    K: Send + Sync,
    V: Clone + Send + Sync,
{
    pub fn new() -> Self {
        NoopCache {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<K, V> Default for NoopCache<K, V>
where
    K: Send + Sync,
    V: Clone + Send + Sync,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> NoopCache<K, V>
where
    K: Send + Sync,
    V: Clone + Send + Sync,
{
    pub fn get(&self, _key: &K) -> Option<V> {
        None
    }

    pub fn insert(&self, _key: K, _value: V) {}

    pub fn remove(&self, _key: &K) -> Option<V> {
        None
    }

    pub fn clear(&self) {}

    pub fn len(&self) -> usize {
        0
    }

    pub fn is_empty(&self) -> bool {
        true
    }
}

impl<K, V> CacheBackend<K, V> for NoopCache<K, V>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn get(&self, _key: &K) -> BoxFuture<'_, AegisResult<Option<V>>> {
        Box::pin(async move { Ok(None) })
    }

    fn insert(&self, _key: K, _value: V) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn remove(&self, _key: &K) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn clear(&self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn len(&self) -> BoxFuture<'_, AegisResult<usize>> {
        Box::pin(async move { Ok(0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI32;

    // -----------------------------------------------------------------------
    // LruMetadataCache tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_lru_basic_operations() {
        let cache = LruMetadataCache::with_capacity(3);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);

        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);
        assert_eq!(cache.len(), 3);
        assert!(!cache.is_empty());

        assert_eq!(cache.get(&"a"), Some(1));
        assert_eq!(cache.get(&"b"), Some(2));
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.get(&"d"), None);
    }

    #[test]
    fn test_lru_eviction_when_full() {
        let cache = LruMetadataCache::with_capacity(3);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);
        cache.insert("d", 4);

        // "a" should have been evicted (LRU)
        assert_eq!(cache.get(&"a"), None);
        assert_eq!(cache.get(&"b"), Some(2));
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.get(&"d"), Some(4));
        assert_eq!(cache.len(), 3);

        // Verify eviction was recorded
        assert!(cache.stats().evictions() >= 1);
    }

    #[test]
    fn test_lru_access_promotes_item() {
        let cache = LruMetadataCache::with_capacity(3);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);

        // Access "a", making it most recently used
        cache.get(&"a");
        cache.insert("d", 4);

        // "b" should be evicted (next LRU after "a" was promoted)
        assert_eq!(cache.get(&"a"), Some(1));
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.get(&"d"), Some(4));
    }

    #[test]
    fn test_lru_remove() {
        let cache = LruMetadataCache::with_capacity(3);
        cache.insert("a", 1);
        cache.insert("b", 2);

        assert_eq!(cache.remove(&"a"), Some(1));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"a"), None);

        assert_eq!(cache.remove(&"nonexistent"), None);
    }

    #[test]
    fn test_lru_clear() {
        let cache = LruMetadataCache::with_capacity(3);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(&"a"), None);
        assert_eq!(cache.get(&"b"), None);
    }

    #[test]
    fn test_lru_update_existing_key() {
        let cache = LruMetadataCache::with_capacity(3);
        cache.insert("a", 1);
        cache.insert("a", 99);
        assert_eq!(cache.get(&"a"), Some(99));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_lru_single_element() {
        let cache = LruMetadataCache::with_capacity(1);
        assert!(cache.is_empty());
        cache.insert("only", 42);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"only"), Some(42));

        // Inserting another should evict the first
        cache.insert("another", 99);
        assert_eq!(cache.get(&"only"), None);
        assert_eq!(cache.get(&"another"), Some(99));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_lru_stats_tracking() {
        let cache = LruMetadataCache::with_capacity(3);

        assert_eq!(cache.stats().hits(), 0);
        assert_eq!(cache.stats().misses(), 0);
        assert_eq!(cache.stats().hit_ratio(), 0.0);

        cache.get(&"miss");
        assert_eq!(cache.stats().misses(), 1);
        assert_eq!(cache.stats().hit_ratio(), 0.0);

        cache.insert("a", 1);
        cache.get(&"a");
        assert_eq!(cache.stats().hits(), 1);
        assert_eq!(cache.stats().misses(), 1);
        assert!((cache.stats().hit_ratio() - 0.5).abs() < 1e-9);

        cache.get(&"a");
        assert_eq!(cache.stats().hits(), 2);
        assert!((cache.stats().hit_ratio() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_lru_full_eviction_stats() {
        let cache = LruMetadataCache::with_capacity(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        assert_eq!(cache.stats().evictions(), 0);

        cache.insert("c", 3);
        assert_eq!(cache.stats().evictions(), 1);

        cache.insert("d", 4);
        assert_eq!(cache.stats().evictions(), 2);

        // "a" and "b" should be gone
        assert_eq!(cache.get(&"a"), None);
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.get(&"d"), Some(4));
    }

    // -----------------------------------------------------------------------
    // BloomCache tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_bloom_basic_operations() {
        let bloom = BloomCache::new(100, 0.01);
        assert!(bloom.is_empty());
        assert_eq!(bloom.len(), 0);

        bloom.insert(b"hello");
        bloom.insert(b"world");

        assert_eq!(bloom.len(), 2);
        assert!(!bloom.is_empty());
        assert!(bloom.contains(b"hello"));
        assert!(bloom.contains(b"world"));
    }

    #[test]
    fn test_bloom_clear() {
        let bloom = BloomCache::new(100, 0.01);
        bloom.insert(b"data");
        assert!(bloom.contains(b"data"));
        assert_eq!(bloom.len(), 1);

        bloom.clear();
        assert!(!bloom.contains(b"data"));
        assert!(bloom.is_empty());
        assert_eq!(bloom.len(), 0);
    }

    #[test]
    fn test_bloom_false_positive_rate() {
        let n = 10_000;
        let fp_rate = 0.01;
        let bloom = BloomCache::new(n, fp_rate);

        // Insert n items
        for i in 0..n {
            bloom.insert(format!("key_{i}").as_bytes());
        }

        // Test n different items not inserted — count false positives
        let trials = n;
        let mut false_positives = 0;
        for i in n..n + trials {
            if bloom.contains(format!("key_{i}").as_bytes()) {
                false_positives += 1;
            }
        }

        let actual_fp = false_positives as f64 / trials as f64;
        // Allow 5x the target rate — bloom filter guarantees are probabilistic
        assert!(
            actual_fp < fp_rate * 5.0,
            "False positive rate {:.4} exceeds {}",
            actual_fp,
            fp_rate * 5.0
        );
    }

    #[test]
    fn test_bloom_no_false_negatives() {
        let bloom = BloomCache::new(1000, 0.01);
        let items: Vec<_> = (0..500).map(|i| format!("item_{i}")).collect();

        for item in &items {
            bloom.insert(item.as_bytes());
        }

        for item in &items {
            assert!(
                bloom.contains(item.as_bytes()),
                "Bloom filter must not have false negatives"
            );
        }
    }

    #[test]
    fn test_bloom_empty_filter() {
        let bloom = BloomCache::new(100, 0.01);
        assert!(!bloom.contains(b"anything"));
        assert!(bloom.is_empty());
    }

    // -----------------------------------------------------------------------
    // TwoTierCache tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_two_tier_fallback_to_backing_store() {
        let store: Arc<dyn BackingStore<String, i32>> = Arc::new(|key: &String| {
            let val: i32 = key.parse().unwrap_or(0);
            Ok(Some(val * 10))
        });

        let cache = TwoTierCache::with_capacity(3, store);

        // Miss in LRU, should fall back to the store
        let result = cache.get(&"5".to_string()).unwrap();
        assert_eq!(result, Some(50));

        // Now should be cached in LRU
        let result = cache.get(&"5".to_string()).unwrap();
        assert_eq!(result, Some(50));
    }

    #[test]
    fn test_two_tier_miss_propagates() {
        let store: Arc<dyn BackingStore<String, i32>> =
            Arc::new(|_: &String| -> AegisResult<Option<i32>> { Ok(None) });

        let cache = TwoTierCache::with_capacity(3, store);
        let result = cache.get(&"missing".to_string()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_two_tier_insert_and_evict() {
        let store: Arc<dyn BackingStore<String, i32>> =
            Arc::new(|key: &String| -> AegisResult<Option<i32>> {
                if key == "a" {
                    Ok(None)
                } else {
                    Ok(Some(0)) // fallback default
                }
            });

        let cache = TwoTierCache::with_capacity(2, store);

        cache.insert("a".to_string(), 1);
        cache.insert("b".to_string(), 2);
        cache.insert("c".to_string(), 3);

        assert_eq!(cache.get(&"a".to_string()).unwrap(), None);
        assert_eq!(cache.get(&"b".to_string()).unwrap(), Some(2));
        assert_eq!(cache.get(&"c".to_string()).unwrap(), Some(3));
    }

    #[test]
    fn test_two_tier_clear() {
        let store: Arc<dyn BackingStore<String, i32>> =
            Arc::new(|_: &String| -> AegisResult<Option<i32>> { Ok(None) });

        let cache = TwoTierCache::with_capacity(5, store);
        cache.insert("x".to_string(), 100);
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    // -----------------------------------------------------------------------
    // NoopCache tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_noop_always_misses() {
        let cache: NoopCache<&str, i32> = NoopCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);

        cache.insert("anything", 42);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(&"anything"), None);

        assert_eq!(cache.remove(&"anything"), None);
        cache.clear();
        assert!(cache.is_empty());
    }

    // -----------------------------------------------------------------------
    // CacheStats tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_cache_stats_basic() {
        let stats = CacheStats::new();
        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 0);
        assert_eq!(stats.evictions(), 0);
        assert_eq!(stats.size(), 0);
        assert_eq!(stats.hit_ratio(), 0.0);

        stats.record_hit();
        stats.record_hit();
        stats.record_miss();
        assert_eq!(stats.hits(), 2);
        assert_eq!(stats.misses(), 1);
        assert!((stats.hit_ratio() - 2.0 / 3.0).abs() < 1e-9);

        stats.record_eviction();
        assert_eq!(stats.evictions(), 1);

        stats.set_size(100);
        assert_eq!(stats.size(), 100);
    }

    #[test]
    fn test_cache_stats_reset() {
        let stats = CacheStats::new();
        stats.record_hit();
        stats.record_miss();
        stats.record_eviction();
        stats.set_size(50);

        stats.reset();
        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 0);
        assert_eq!(stats.evictions(), 0);
        assert_eq!(stats.size(), 0);
    }

    #[test]
    fn test_cache_stats_clone() {
        let stats = CacheStats::new();
        stats.record_hit();
        stats.record_miss();
        stats.set_size(10);

        let cloned = stats.clone();
        assert_eq!(cloned.hits(), 1);
        assert_eq!(cloned.misses(), 1);
        assert_eq!(cloned.size(), 10);
    }

    // -----------------------------------------------------------------------
    // Concurrent access tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_lru_concurrent_access() {
        let cache = Arc::new(LruMetadataCache::with_capacity(100));
        let mut handles = Vec::new();

        for t in 0..8 {
            let cache = Arc::clone(&cache);
            handles.push(std::thread::spawn(move || {
                for i in 0..500 {
                    let key = format!("thread_{t}_key_{i}");
                    cache.insert(key.clone(), i);
                    let _ = cache.get(&key);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Perform a query for a nonexistent key to guarantee at least one miss
        let _ = cache.get(&"nonexistent_key_to_force_miss".to_string());

        // All operations should have completed without deadlock
        assert!(cache.len() <= 100);
        assert!(cache.stats().hits() > 0);
        assert!(cache.stats().misses() > 0);
    }

    #[test]
    fn test_bloom_concurrent_access() {
        let bloom = Arc::new(BloomCache::new(1000, 0.01));
        let mut handles = Vec::new();

        for t in 0..8 {
            let bloom = Arc::clone(&bloom);
            handles.push(std::thread::spawn(move || {
                for i in 0..200 {
                    let key = format!("concurrent_key_{t}_{i}");
                    bloom.insert(key.as_bytes());
                    assert!(bloom.contains(key.as_bytes()));
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_two_tier_concurrent_access() {
        let store: Arc<dyn BackingStore<String, i32>> = Arc::new(|key: &String| {
            Ok(Some(key.len() as i32))
        });

        let cache = Arc::new(TwoTierCache::with_capacity(50, store));
        let mut handles = Vec::new();

        for _ in 0..8 {
            let cache = Arc::clone(&cache);
            handles.push(std::thread::spawn(move || {
                for i in 0..200 {
                    let key = format!("key_{i}");
                    let _ = cache.get(&key);
                    cache.insert(key, i);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    // -----------------------------------------------------------------------
    // CacheBackend trait implementation tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_cache_backend_lru() {
        let cache = LruMetadataCache::with_capacity(3);

        CacheBackend::insert(&cache, "a", 1).await.unwrap();
        CacheBackend::insert(&cache, "b", 2).await.unwrap();

        let result = CacheBackend::get(&cache, &"a").await.unwrap();
        assert_eq!(result, Some(1));

        let len = CacheBackend::len(&cache).await.unwrap();
        assert_eq!(len, 2);

        CacheBackend::remove(&cache, &"a").await.unwrap();
        let result = CacheBackend::get(&cache, &"a").await.unwrap();
        assert_eq!(result, None);

        CacheBackend::clear(&cache).await.unwrap();
        let len = CacheBackend::len(&cache).await.unwrap();
        assert_eq!(len, 0);
    }

    #[tokio::test]
    async fn test_cache_backend_noop() {
        let cache: NoopCache<&str, i32> = NoopCache::new();

        CacheBackend::insert(&cache, "k", 99).await.unwrap();
        let result = CacheBackend::get(&cache, &"k").await.unwrap();
        assert_eq!(result, None);

        let len = CacheBackend::len(&cache).await.unwrap();
        assert_eq!(len, 0);

        CacheBackend::remove(&cache, &"k").await.unwrap();
        CacheBackend::clear(&cache).await.unwrap();
    }

    // -----------------------------------------------------------------------
    // Edge case tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_lru_capacity_one_full_eviction() {
        let cache = LruMetadataCache::with_capacity(1);
        cache.insert("first", 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"first"), Some(1));

        cache.insert("second", 2);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"first"), None);
        assert_eq!(cache.get(&"second"), Some(2));

        cache.insert("third", 3);
        assert_eq!(cache.get(&"second"), None);
        assert_eq!(cache.get(&"third"), Some(3));
    }

    #[test]
    fn test_bloom_tiny_filter() {
        let bloom = BloomCache::new(1, 0.5);
        assert!(bloom.num_bits() > 0);
        assert!(bloom.num_hashes() >= 1);

        bloom.insert(b"x");
        assert!(bloom.contains(b"x"));
    }

    #[test]
    fn test_two_tier_empty_cache() {
        let store: Arc<dyn BackingStore<String, i32>> =
            Arc::new(|_: &String| -> AegisResult<Option<i32>> { Ok(None) });

        let cache = TwoTierCache::with_capacity(5, store);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(&"anything".to_string()).unwrap(), None);
    }
}
