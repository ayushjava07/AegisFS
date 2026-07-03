use std::collections::HashMap;
use std::sync::Arc;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::CompressionProvider;
use crate::core::types::CompressionAlgorithm;

/// Threshold below which no compression is applied.
const NOOP_THRESHOLD: usize = 64;

/// Threshold below which Lz4 is preferred over Zstd.
const LZ4_THRESHOLD: usize = 4096;

// ---------------------------------------------------------------------------
// ZstdCompression
// ---------------------------------------------------------------------------

pub struct ZstdCompression {
    level: i32,
}

impl ZstdCompression {
    pub fn new(level: i32) -> Self {
        Self {
            level: level.clamp(1, 22),
        }
    }

    pub fn with_default_level() -> Self {
        Self::new(3)
    }
}

impl CompressionProvider for ZstdCompression {
    fn compress(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
        let mut cursor = std::io::Cursor::new(data);
        zstd::stream::encode_all(&mut cursor, self.level)
            .map_err(|e| AegisError::CompressionError(format!("zstd compress: {e}")))
    }

    fn decompress(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
        let mut cursor = std::io::Cursor::new(data);
        zstd::stream::decode_all(&mut cursor)
            .map_err(|e| AegisError::DecompressionError(format!("zstd decompress: {e}")))
    }

    fn algorithm(&self) -> CompressionAlgorithm {
        CompressionAlgorithm::Zstd(self.level)
    }
}

// ---------------------------------------------------------------------------
// Lz4Compression
// ---------------------------------------------------------------------------

pub struct Lz4Compression;

impl Lz4Compression {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Lz4Compression {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressionProvider for Lz4Compression {
    fn compress(&self, _data: &[u8]) -> AegisResult<Vec<u8>> {
        #[cfg(feature = "lz4-compression")]
        {
            Ok(lz4_flex::compress_prepend_size(_data))
        }
        #[cfg(not(feature = "lz4-compression"))]
        {
            Err(AegisError::CompressionError(
                "lz4 not enabled (feature 'lz4-compression')".into(),
            ))
        }
    }

    fn decompress(&self, _data: &[u8]) -> AegisResult<Vec<u8>> {
        #[cfg(feature = "lz4-compression")]
        {
            lz4_flex::decompress_size_prepended(_data)
                .map_err(|e| AegisError::DecompressionError(format!("lz4 decompress: {e}")))
        }
        #[cfg(not(feature = "lz4-compression"))]
        {
            Err(AegisError::DecompressionError(
                "lz4 not enabled (feature 'lz4-compression')".into(),
            ))
        }
    }

    fn algorithm(&self) -> CompressionAlgorithm {
        CompressionAlgorithm::Lz4
    }
}

// ---------------------------------------------------------------------------
// NoopCompression
// ---------------------------------------------------------------------------

pub struct NoopCompression;

impl NoopCompression {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopCompression {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressionProvider for NoopCompression {
    fn compress(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
        Ok(data.to_vec())
    }

    fn decompress(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
        Ok(data.to_vec())
    }

    fn algorithm(&self) -> CompressionAlgorithm {
        CompressionAlgorithm::None
    }
}

// ---------------------------------------------------------------------------
// CompressionRegistry
// ---------------------------------------------------------------------------

pub struct CompressionRegistry {
    providers: HashMap<&'static str, Arc<dyn CompressionProvider>>,
    default: Arc<dyn CompressionProvider>,
}

impl CompressionRegistry {
    pub fn new() -> Self {
        let mut providers: HashMap<&'static str, Arc<dyn CompressionProvider>> = HashMap::new();

        let zstd = Arc::new(ZstdCompression::with_default_level()) as Arc<dyn CompressionProvider>;
        let lz4 = Arc::new(Lz4Compression::new()) as Arc<dyn CompressionProvider>;
        let noop = Arc::new(NoopCompression::new()) as Arc<dyn CompressionProvider>;

        providers.insert("zstd", zstd);
        providers.insert("lz4", lz4);
        providers.insert("none", noop);

        let default = providers.get("zstd").unwrap().clone();

        Self { providers, default }
    }

    pub fn register(
        &mut self,
        name: &'static str,
        provider: Arc<dyn CompressionProvider>,
    ) -> Option<Arc<dyn CompressionProvider>> {
        self.providers.insert(name, provider)
    }

    pub fn get_default(&self) -> &Arc<dyn CompressionProvider> {
        &self.default
    }

    pub fn get(&self, name: &str) -> AegisResult<Arc<dyn CompressionProvider>> {
        self.providers.get(name).cloned().ok_or_else(|| {
            AegisError::InvalidArgument(format!("unknown compression algorithm: {name}"))
        })
    }

    pub fn set_default(&mut self, name: &str) -> AegisResult<()> {
        let provider = self.get(name)?;
        self.default = provider;
        Ok(())
    }

    pub fn default(&self) -> &Arc<dyn CompressionProvider> {
        &self.default
    }

    pub fn algorithms(&self) -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = self.providers.keys().copied().collect();
        keys.sort();
        keys
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

impl Default for CompressionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// auto_select – pick the best compression provider based on data size
// ---------------------------------------------------------------------------

/// Automatically selects a compression provider based on the size of `data`.
///
/// Heuristic:
/// - data.len() <= 64          → NoopCompression (compression overhead is not worth it)
/// - data.len() <= 4096        → Lz4Compression  (fast, low overhead for small data)
/// - else                       → ZstdCompression (better ratio for larger data)
pub fn auto_select(data: &[u8]) -> Box<dyn CompressionProvider> {
    let len = data.len();
    if len <= NOOP_THRESHOLD {
        Box::new(NoopCompression::new())
    } else if len <= LZ4_THRESHOLD {
        #[cfg(feature = "lz4-compression")]
        {
            Box::new(Lz4Compression::new())
        }
        #[cfg(not(feature = "lz4-compression"))]
        {
            Box::new(NoopCompression::new())
        }
    } else {
        Box::new(ZstdCompression::with_default_level())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // helpers
    // -----------------------------------------------------------------------

    fn roundtrip(provider: &dyn CompressionProvider, data: &[u8]) {
        let compressed = provider
            .compress(data)
            .unwrap_or_else(|e| panic!("compress failed: {e}"));
        let decompressed = provider
            .decompress(&compressed)
            .unwrap_or_else(|e| panic!("decompress failed: {e}"));
        assert_eq!(
            data,
            &decompressed[..],
            "roundtrip failed for algorithm {}",
            provider.algorithm()
        );
    }

    fn test_data_sets() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("empty", vec![]),
            ("single_byte", vec![42]),
            ("small", b"hello world".to_vec()),
            // 100-byte pattern
            ("medium", (0..100).map(|i| (i % 256) as u8).collect()),
            // 8 KB pseudo-random
            (
                "8kb",
                (0..8192).map(|i| ((i * 7 + 13) % 256) as u8).collect(),
            ),
            // 64 KB repeated pattern
            (
                "64kb_repeated",
                (0..65536).map(|i| (i % 32) as u8).collect(),
            ),
            // all zeros (highly compressible)
            ("zeros_16kb", vec![0u8; 16384]),
        ]
    }

    // -----------------------------------------------------------------------
    // ZstdCompression
    // -----------------------------------------------------------------------

    #[test]
    fn zstd_level_clamping() {
        let low = ZstdCompression::new(0);
        assert_eq!(low.level, 1);
        let high = ZstdCompression::new(99);
        assert_eq!(high.level, 22);
        let norm = ZstdCompression::new(10);
        assert_eq!(norm.level, 10);
    }

    #[test]
    fn zstd_empty_data() {
        let provider = ZstdCompression::with_default_level();
        let compressed = provider.compress(b"").unwrap();
        let decompressed = provider.decompress(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }

    #[test]
    fn zstd_roundtrip_all() {
        let provider = ZstdCompression::new(3);
        for (name, data) in test_data_sets() {
            roundtrip(&provider, &data);
            let compressed = provider.compress(&data).unwrap();
            assert!(
                !compressed.is_empty(),
                "compressed output should not be empty for {name}"
            );
        }
    }

    #[test]
    fn zstd_decompress_corrupted() {
        let provider = ZstdCompression::new(3);
        let result = provider.decompress(b"not-zstd-data-here");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Lz4Compression
    // -----------------------------------------------------------------------

    #[cfg(feature = "lz4-compression")]
    #[test]
    fn lz4_empty_data() {
        let provider = Lz4Compression::new();
        let compressed = provider.compress(b"").unwrap();
        // lz4_flex prepends size, so even empty input produces a header
        assert!(!compressed.is_empty());
        let decompressed = provider.decompress(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }

    #[cfg(feature = "lz4-compression")]
    #[test]
    fn lz4_roundtrip_all() {
        let provider = Lz4Compression::new();
        for (name, data) in test_data_sets() {
            roundtrip(&provider, &data);
            let compressed = provider.compress(&data).unwrap();
            assert!(
                !compressed.is_empty(),
                "compressed output should not be empty for {name}"
            );
        }
    }

    #[cfg(feature = "lz4-compression")]
    #[test]
    fn lz4_decompress_corrupted() {
        let provider = Lz4Compression::new();
        let result = provider.decompress(b"garbage");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // NoopCompression
    // -----------------------------------------------------------------------

    #[test]
    fn noop_roundtrip_all() {
        let provider = NoopCompression::new();
        for (name, data) in test_data_sets() {
            let compressed = provider.compress(&data).unwrap();
            assert_eq!(
                compressed, data,
                "noop compress should return identical data for {name}"
            );
            let decompressed = provider.decompress(&compressed).unwrap();
            assert_eq!(
                decompressed, data,
                "noop decompress should return identical data for {name}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // CompressionRegistry
    // -----------------------------------------------------------------------

    #[test]
    fn registry_default_providers() {
        let registry = CompressionRegistry::new();
        assert_eq!(registry.len(), 3);
        assert!(registry.algorithms().contains(&"zstd"));
        assert!(registry.algorithms().contains(&"lz4"));
        assert!(registry.algorithms().contains(&"none"));
    }

    #[test]
    fn registry_get_unknown() {
        let registry = CompressionRegistry::new();
        let result = registry.get("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn registry_roundtrip_via_get() {
        let registry = CompressionRegistry::new();
        let data = b"hello compression registry";

        for name in &["zstd", "none"] {
            let provider = registry.get(name).unwrap();
            roundtrip(provider.as_ref(), data);
        }
        #[cfg(feature = "lz4-compression")]
        {
            let provider = registry.get("lz4").unwrap();
            roundtrip(provider.as_ref(), data);
        }
    }

    #[test]
    fn registry_set_default() {
        let mut registry = CompressionRegistry::new();
        assert_eq!(
            registry.default().algorithm(),
            CompressionAlgorithm::Zstd(3)
        );
        registry.set_default("none").unwrap();
        assert_eq!(registry.default().algorithm(), CompressionAlgorithm::None);
    }

    #[test]
    fn registry_register_custom() {
        let mut registry = CompressionRegistry::new();
        let custom = Arc::new(ZstdCompression::new(10)) as Arc<dyn CompressionProvider>;
        registry.register("zstd-fast", custom);
        assert_eq!(registry.len(), 4);
        let provider = registry.get("zstd-fast").unwrap();
        assert_eq!(provider.algorithm(), CompressionAlgorithm::Zstd(10));
    }

    // -----------------------------------------------------------------------
    // auto_select
    // -----------------------------------------------------------------------

    #[test]
    fn auto_select_returns_noop_for_tiny_data() {
        let provider = auto_select(&[0u8; 10]);
        assert_eq!(provider.algorithm(), CompressionAlgorithm::None);
    }

    #[test]
    fn auto_select_returns_noop_for_boundary() {
        let provider = auto_select(&[0u8; NOOP_THRESHOLD]);
        assert_eq!(provider.algorithm(), CompressionAlgorithm::None);
    }

    #[cfg(feature = "lz4-compression")]
    #[test]
    fn auto_select_returns_lz4_for_small_data() {
        let provider = auto_select(&[0u8; 256]);
        assert_eq!(provider.algorithm(), CompressionAlgorithm::Lz4);
    }

    #[cfg(feature = "lz4-compression")]
    #[test]
    fn auto_select_returns_lz4_at_upper_boundary() {
        let provider = auto_select(&[0u8; LZ4_THRESHOLD]);
        assert_eq!(provider.algorithm(), CompressionAlgorithm::Lz4);
    }

    #[test]
    fn auto_select_returns_zstd_for_large_data() {
        let provider = auto_select(&[0u8; 100_000]);
        assert_eq!(provider.algorithm(), CompressionAlgorithm::Zstd(3));
    }

    #[test]
    fn auto_select_roundtrip_all_sets() {
        for (name, data) in test_data_sets() {
            let provider = auto_select(&data);
            roundtrip(provider.as_ref(), &data);
            // also verify the algorithm decision is consistent
            let algo = provider.algorithm();
            match data.len() {
                s if s <= NOOP_THRESHOLD => assert_eq!(algo, CompressionAlgorithm::None, "{name}"),
                _ => {} // algorithm depends on features, just verify roundtrip
            }
        }
    }

    // -----------------------------------------------------------------------
    // algorithm Display (roundtrip via CompressionAlgorithm)
    // -----------------------------------------------------------------------

    #[test]
    fn algorithm_display_zstd() {
        let algo = CompressionAlgorithm::Zstd(3);
        assert_eq!(algo.to_string(), "zstd(3)");
    }

    #[test]
    fn algorithm_display_lz4() {
        let algo = CompressionAlgorithm::Lz4;
        assert_eq!(algo.to_string(), "lz4");
    }

    #[test]
    fn algorithm_display_none() {
        let algo = CompressionAlgorithm::None;
        assert_eq!(algo.to_string(), "none");
    }

    // -----------------------------------------------------------------------
    // Determinism – same input → same compressed output (for same level)
    // -----------------------------------------------------------------------

    #[test]
    fn zstd_deterministic() {
        let provider = ZstdCompression::new(3);
        let data = b"determinism check payload";
        let a = provider.compress(data).unwrap();
        let b = provider.compress(data).unwrap();
        assert_eq!(a, b, "zstd should be deterministic at the same level");
    }

    #[cfg(feature = "lz4-compression")]
    #[test]
    fn lz4_deterministic() {
        let provider = Lz4Compression::new();
        let data = b"determinism check payload";
        let a = provider.compress(data).unwrap();
        let b = provider.compress(data).unwrap();
        assert_eq!(a, b, "lz4 should be deterministic");
    }

    // -----------------------------------------------------------------------
    // Multi-level Zstd roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn zstd_multi_level_roundtrip() {
        let data = b"multi-level roundtrip test data";
        for level in [1, 3, 10, 19, 22] {
            let provider = ZstdCompression::new(level);
            roundtrip(&provider, data);
            assert_eq!(provider.algorithm(), CompressionAlgorithm::Zstd(level));
        }
    }

    // -----------------------------------------------------------------------
    // Large data stress (256 KB)
    // -----------------------------------------------------------------------

    #[test]
    fn zstd_large_data_stress() {
        let data: Vec<u8> = (0..262_144).map(|i| (i % 251) as u8).collect();
        let provider = ZstdCompression::new(3);
        roundtrip(&provider, &data);
    }

    #[cfg(feature = "lz4-compression")]
    #[test]
    fn lz4_large_data_stress() {
        let data: Vec<u8> = (0..262_144).map(|i| (i % 251) as u8).collect();
        let provider = Lz4Compression::new();
        roundtrip(&provider, &data);
    }

    // -----------------------------------------------------------------------
    // Compress(decompress(data)) == data for all providers
    // -----------------------------------------------------------------------

    #[test]
    fn all_providers_roundtrip_identity() {
        #[allow(unused_mut)]
        let mut providers: Vec<Box<dyn CompressionProvider>> = vec![
            Box::new(ZstdCompression::new(3)),
            Box::new(NoopCompression::new()),
        ];
        #[cfg(feature = "lz4-compression")]
        {
            providers.push(Box::new(Lz4Compression::new()));
        }

        let datasets = vec![
            b"" as &[u8],
            b"\x00",
            b"a",
            b"hello",
            &[0u8; 100],
            &[0xFFu8; 1000],
        ];

        for provider in &providers {
            for data in &datasets {
                roundtrip(provider.as_ref(), data);
            }
        }
    }
}
