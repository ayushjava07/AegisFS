use bitvec::prelude::*;
use xxhash_rust::xxh3::xxh3_64;

use crate::core::types::HashValue;

pub struct DedupBloomFilter {
    bits: BitVec,
    num_hashes: u32,
    size: usize,
    count: usize,
}

impl DedupBloomFilter {
    pub fn new(expected_items: usize, false_positive_rate: f64) -> Self {
        let size = optimal_bit_size(expected_items, false_positive_rate);
        let num_hashes = optimal_hash_count(size, expected_items);
        Self {
            bits: bitvec![0; size],
            num_hashes: num_hashes.max(1),
            size,
            count: 0,
        }
    }

    pub fn insert(&mut self, hash: &HashValue) {
        let hashes = self.compute_hashes(hash);
        for h in &hashes {
            let idx = (*h as usize) % self.size;
            self.bits.set(idx, true);
        }
        self.count += 1;
    }

    pub fn contains(&self, hash: &HashValue) -> bool {
        let hashes = self.compute_hashes(hash);
        for h in &hashes {
            let idx = (*h as usize) % self.size;
            if !self.bits[idx] {
                return false;
            }
        }
        true
    }

    pub fn clear(&mut self) {
        self.bits.fill(false);
        self.count = 0;
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn estimated_false_positive_rate(&self) -> f64 {
        let k = self.num_hashes as f64;
        let n = self.count as f64;
        let m = self.size as f64;
        (1.0 - (-(k * n) / m).exp()).powf(k)
    }

    fn compute_hashes(&self, hash: &HashValue) -> Vec<u64> {
        let bytes = hash.as_bytes();
        let mut hashes = Vec::with_capacity(self.num_hashes as usize);

        let h0 = xxh3_64(bytes);
        let combined: Vec<u8> = bytes.iter().chain(std::iter::once(&1u8)).copied().collect();
        let h1 = xxh3_64(&combined);

        for i in 0..self.num_hashes {
            let h = h0.wrapping_add(i as u64).wrapping_mul(h1);
            hashes.push(h);
        }

        hashes
    }
}

fn optimal_bit_size(expected_items: usize, false_positive_rate: f64) -> usize {
    if expected_items == 0 {
        return 1;
    }
    let n = expected_items as f64;
    let p = false_positive_rate;
    let size = -(n * p.ln()) / (std::f64::consts::LN_2.powi(2));
    (size.ceil() as usize).max(1)
}

fn optimal_hash_count(size: usize, expected_items: usize) -> u32 {
    if expected_items == 0 || size == 0 {
        return 1;
    }
    let m = size as f64;
    let n = expected_items as f64;
    let k = (m / n) * std::f64::consts::LN_2;
    (k.ceil() as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_filter_insert_and_contains() {
        let mut bloom = DedupBloomFilter::new(100, 0.01);
        let hash = HashValue::sha256(b"test data");
        assert!(!bloom.contains(&hash));
        bloom.insert(&hash);
        assert!(bloom.contains(&hash));
    }

    #[test]
    fn test_bloom_filter_multiple_inserts() {
        let mut bloom = DedupBloomFilter::new(100, 0.01);
        let hashes: Vec<HashValue> = (0..50)
            .map(|i| HashValue::sha256(format!("data-{}", i).as_bytes()))
            .collect();

        for h in &hashes {
            assert!(!bloom.contains(h));
            bloom.insert(&h);
            assert!(bloom.contains(&h));
        }

        assert_eq!(bloom.len(), 50);
    }

    #[test]
    fn test_bloom_filter_clear() {
        let mut bloom = DedupBloomFilter::new(100, 0.01);
        let hash = HashValue::sha256(b"test");
        bloom.insert(&hash);
        assert!(bloom.contains(&hash));
        bloom.clear();
        assert!(!bloom.contains(&hash));
        assert_eq!(bloom.len(), 0);
        assert!(bloom.is_empty());
    }

    #[test]
    fn test_bloom_filter_false_positive_rate() {
        let mut bloom = DedupBloomFilter::new(1000, 0.05);
        let inserted: Vec<HashValue> = (0..500)
            .map(|i| HashValue::sha256(format!("insert-{}", i).as_bytes()))
            .collect();

        for h in &inserted {
            bloom.insert(h);
        }

        let mut false_positives = 0;
        let test_count = 1000;
        for i in 0..test_count {
            let h = HashValue::sha256(format!("test-{}", i).as_bytes());
            if bloom.contains(&h) && !inserted.contains(&h) {
                false_positives += 1;
            }
        }

        let fp_rate = false_positives as f64 / test_count as f64;
        assert!(fp_rate < 0.15, "false positive rate too high: {}", fp_rate);
    }

    #[test]
    fn test_empty_bloom() {
        let bloom = DedupBloomFilter::new(100, 0.01);
        assert!(bloom.is_empty());
        assert_eq!(bloom.len(), 0);
    }

    #[test]
    fn test_optimal_size_calculation() {
        let size = optimal_bit_size(1000, 0.01);
        assert!(size > 1000);
        assert!(size < 20000);
    }

    #[test]
    fn test_optimal_hash_count() {
        let count = optimal_hash_count(10000, 1000);
        assert!(count >= 1);
        assert!(count <= 20);
    }

    #[test]
    fn test_bloom_estimated_fp_rate() {
        let mut bloom = DedupBloomFilter::new(100, 0.01);
        for i in 0..50 {
            let h = HashValue::sha256(format!("data-{}", i).as_bytes());
            bloom.insert(&h);
        }
        let est = bloom.estimated_false_positive_rate();
        assert!(est > 0.0);
        assert!(est < 1.0);
    }

    #[test]
    fn test_bloom_edge_cases() {
        let mut bloom = DedupBloomFilter::new(1, 0.5);
        let hash = HashValue::sha256(b"single");
        assert!(!bloom.contains(&hash));
        bloom.insert(&hash);
        assert!(bloom.contains(&hash));
    }

    #[test]
    fn test_compute_hashes_deterministic() {
        let bloom = DedupBloomFilter::new(100, 0.01);
        let hash = HashValue::sha256(b"deterministic test");
        let h1 = bloom.compute_hashes(&hash);
        let h2 = bloom.compute_hashes(&hash);
        assert_eq!(h1, h2);
    }
}
