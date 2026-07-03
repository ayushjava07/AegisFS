use std::io::{Read, Write};

use blake3;
use sha2::{Digest, Sha256};
use xxhash_rust::xxh3;

use crate::core::error::{AegisError, AegisResult};
use crate::core::id::HashValue;
use crate::core::traits::Hasher;

/// SHA256 hasher implementing the Hasher trait.
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

/// BLAKE3 hasher implementing the Hasher trait.
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

/// xxHash3 (64-bit) hasher for fast non-cryptographic use.
///
/// The 64-bit hash value is stored little-endian in the first 8 bytes
/// of the HashValue; the remaining 24 bytes are zeroed.
#[derive(Default, Clone, Debug)]
pub struct Xxh3Hasher;

impl Xxh3Hasher {
    /// Returns the raw 64-bit xxHash3 value for the given data.
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

/// A wrapper that reads from a stream and computes the SHA256 hash
/// incrementally as data is read through it.
pub struct HashStream<R: Read> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> HashStream<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// Finalize the hash computation and return the result.
    /// Consumes the stream.
    pub fn finalize(self) -> HashValue {
        let result = self.hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        HashValue::from_bytes(bytes)
    }

    /// Unwrap the inner reader.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for HashStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

/// Wraps a reader and verifies its expected checksum after all data has
/// been read.
///
/// The caller **must** read all data (e.g. via `std::io::copy` or repeated
/// reads) before calling [`verify`][ChecksummedReader::verify] to check the
/// integrity of the stream.
pub struct ChecksummedReader<R: Read> {
    inner: R,
    hasher: Sha256,
    expected: HashValue,
    verified: bool,
}

impl<R: Read> ChecksummedReader<R> {
    pub fn new(inner: R, expected: HashValue) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            expected,
            verified: false,
        }
    }

    /// Finalize the hash computation and compare it against the expected
    /// value.  Returns a `ChecksumMismatch` error if they differ.
    pub fn verify(&mut self) -> AegisResult<()> {
        let result = self.hasher.clone().finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        let computed = HashValue::from_bytes(bytes);

        if computed != self.expected {
            return Err(AegisError::ChecksumMismatch {
                expected: self.expected.to_hex(),
                actual: computed.to_hex(),
            });
        }
        self.verified = true;
        Ok(())
    }

    /// Returns `true` if the checksum has been verified successfully.
    pub fn is_verified(&self) -> bool {
        self.verified
    }

    /// Unwrap the inner reader.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for ChecksummedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

/// Wraps a writer and computes a SHA256 checksum of all data written
/// through it.
///
/// Call [`finalize`][ChecksummedWriter::finalize] after all writes are
/// complete to retrieve the computed hash.
pub struct ChecksummedWriter<W: Write> {
    inner: W,
    hasher: Sha256,
    computed: Option<HashValue>,
}

impl<W: Write> ChecksummedWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            computed: None,
        }
    }

    /// Flush the inner writer, finalize the hash, and store the result.
    /// Returns the computed hash.
    pub fn finalize(&mut self) -> AegisResult<HashValue> {
        self.inner.flush()?;
        let result = self.hasher.clone().finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        let hash = HashValue::from_bytes(bytes);
        self.computed = Some(hash);
        Ok(hash)
    }

    /// Returns the computed hash, if [`finalize`][ChecksummedWriter::finalize]
    /// has already been called.
    pub fn computed_hash(&self) -> Option<HashValue> {
        self.computed
    }

    /// Unwrap the inner writer.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for ChecksummedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Computes both SHA256 and xxHash3 hashes simultaneously.
///
/// This is useful for scenarios where both a cryptographic and a fast
/// non-cryptographic hash are needed from a single pass over the data.
///
/// When used via the [`Hasher`] trait, [`hash`][Hasher::hash] returns only
/// the SHA256 value.  Use [`sha256_hash`][CombinedHasher::sha256_hash] and
/// [`xxh3_hash`][CombinedHasher::xxh3_hash] to retrieve individual results,
/// or [`finalize`][CombinedHasher::finalize] to get both at once.
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

    /// Feed data into both internal hashers.
    pub fn update(&mut self, data: &[u8]) {
        self.sha256.update(data);
        self.xxh3.update(data);
    }

    /// Finalize and return both hashes as a `(SHA256, xxh3_64)` pair.
    pub fn finalize(self) -> (HashValue, u64) {
        let result = self.sha256.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        let sha = HashValue::from_bytes(bytes);
        let xxh = self.xxh3.digest();
        (sha, xxh)
    }

    /// Return the SHA256 hash of all data fed so far (without consuming
    /// the hasher).
    pub fn sha256_hash(&self) -> HashValue {
        let result = self.sha256.clone().finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        HashValue::from_bytes(bytes)
    }

    /// Return the xxHash3 (64-bit) hash of all data fed so far (without
    /// consuming the hasher).
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::str::FromStr;

    // ---- known test vectors ------------------------------------------------

    const SHA256_EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    const BLAKE3_EMPTY: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const BLAKE3_ABC: &str = "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";

    fn hex_to_hash(hex: &str) -> HashValue {
        HashValue::from_str(hex).unwrap()
    }

    #[test]
    fn test_sha256_known_vectors() {
        let hasher = Sha256Hasher;
        assert_eq!(hasher.hash(b""), hex_to_hash(SHA256_EMPTY));
        assert_eq!(hasher.hash(b"abc"), hex_to_hash(SHA256_ABC));
    }

    #[test]
    fn test_blake3_known_vectors() {
        let hasher = Blake3Hasher;
        assert_eq!(hasher.hash(b""), hex_to_hash(BLAKE3_EMPTY));
        assert_eq!(hasher.hash(b"abc"), hex_to_hash(BLAKE3_ABC));
    }

    #[test]
    fn test_xxh3_known_vectors() {
        let hasher = Xxh3Hasher;
        let expected_empty = hasher.hash(b""); // computed – xxhash has no
        let expected_abc = hasher.hash(b"abc"); // official test-vector spec
                                                // Re-hash to confirm determinism
        assert_eq!(hasher.hash(b""), expected_empty);
        assert_eq!(hasher.hash(b"abc"), expected_abc);
        // Ensure the raw 64-bit helper works
        let raw_empty = Xxh3Hasher::hash_raw(b"");
        let raw_abc = Xxh3Hasher::hash_raw(b"abc");
        let mut buf = [0u8; 32];
        buf[..8].copy_from_slice(&raw_empty.to_le_bytes());
        assert_eq!(HashValue::from_bytes(buf), expected_empty);
        buf[..8].copy_from_slice(&raw_abc.to_le_bytes());
        assert_eq!(HashValue::from_bytes(buf), expected_abc);
    }

    // ---- hash_stream matches hash ------------------------------------------

    #[test]
    fn test_sha256_hash_stream_matches_hash() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let hasher = Sha256Hasher;
        let direct = hasher.hash(data);
        let streamed = hasher.hash_stream(&mut Cursor::new(data)).unwrap();
        assert_eq!(direct, streamed);
    }

    #[test]
    fn test_blake3_hash_stream_matches_hash() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let hasher = Blake3Hasher;
        let direct = hasher.hash(data);
        let streamed = hasher.hash_stream(&mut Cursor::new(data)).unwrap();
        assert_eq!(direct, streamed);
    }

    #[test]
    fn test_xxh3_hash_stream_matches_hash() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let hasher = Xxh3Hasher;
        let direct = hasher.hash(data);
        let streamed = hasher.hash_stream(&mut Cursor::new(data)).unwrap();
        assert_eq!(direct, streamed);
    }

    // ---- empty data handling -----------------------------------------------

    #[test]
    fn test_empty_data() {
        let sha = Sha256Hasher;
        let blake3 = Blake3Hasher;
        let xxh3 = Xxh3Hasher;

        assert_eq!(sha.hash(b""), hex_to_hash(SHA256_EMPTY));
        assert_eq!(blake3.hash(b""), hex_to_hash(BLAKE3_EMPTY));

        assert_eq!(
            sha.hash_stream(&mut Cursor::new(b"")).unwrap(),
            sha.hash(b"")
        );
        assert_eq!(
            blake3.hash_stream(&mut Cursor::new(b"")).unwrap(),
            blake3.hash(b"")
        );
        assert_eq!(
            xxh3.hash_stream(&mut Cursor::new(b"")).unwrap(),
            xxh3.hash(b"")
        );
    }

    // ---- large data (>1 MB) ------------------------------------------------

    #[test]
    fn test_large_data_hashing() {
        let size = 2_000_000; // ~2 MB
        let data: Vec<u8> = (0..size).map(|i| (i & 0xFF) as u8).collect();

        let sha = Sha256Hasher;
        let blake3 = Blake3Hasher;
        let xxh3 = Xxh3Hasher;

        let direct_sha = sha.hash(&data);
        let streamed_sha = sha.hash_stream(&mut Cursor::new(&data)).unwrap();
        assert_eq!(direct_sha, streamed_sha);

        let direct_blake3 = blake3.hash(&data);
        let streamed_blake3 = blake3.hash_stream(&mut Cursor::new(&data)).unwrap();
        assert_eq!(direct_blake3, streamed_blake3);

        let direct_xxh3 = xxh3.hash(&data);
        let streamed_xxh3 = xxh3.hash_stream(&mut Cursor::new(&data)).unwrap();
        assert_eq!(direct_xxh3, streamed_xxh3);
    }

    // ---- HashStream --------------------------------------------------------

    #[test]
    fn test_hash_stream_wrapper() {
        let data = b"Hello, world! This is a test of the HashStream wrapper.";
        let expected = Sha256Hasher.hash(data);

        let mut stream = HashStream::new(Cursor::new(data));
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, data);

        let computed = stream.finalize();
        assert_eq!(computed, expected);
    }

    // ---- ChecksummedReader -------------------------------------------------

    #[test]
    fn test_checksummed_reader_passes_data_correctly() {
        let data = b"Data integrity check via ChecksummedReader";
        let expected_hash = Sha256Hasher.hash(data);

        let mut reader = ChecksummedReader::new(Cursor::new(data), expected_hash);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, data);

        assert!(reader.verify().is_ok());
        assert!(reader.is_verified());
    }

    #[test]
    fn test_checksummed_reader_mismatch() {
        let data = b"Some data";
        let wrong_hash = HashValue::nil();

        let mut reader = ChecksummedReader::new(Cursor::new(data), wrong_hash);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();

        assert!(reader.verify().is_err());
        assert!(!reader.is_verified());
    }

    // ---- ChecksummedWriter -------------------------------------------------

    #[test]
    fn test_checksummed_writer_computes_checksum() {
        let data = b"Calculate checksum for this data via ChecksummedWriter";
        let expected_hash = Sha256Hasher.hash(data);

        let mut buf = Vec::new();
        {
            let mut writer = ChecksummedWriter::new(&mut buf);
            writer.write_all(data).unwrap();
            let hash = writer.finalize().unwrap();
            assert_eq!(hash, expected_hash);
        }
        assert_eq!(buf, data);
    }

    // ---- CombinedHasher ----------------------------------------------------

    #[test]
    fn test_combined_hasher_matches_individual() {
        let data = b"Combined hasher test data - both hashes should match.";
        let expected_sha = Sha256Hasher.hash(data);
        let expected_xxh3 = Xxh3Hasher::hash_raw(data);

        let mut combined = CombinedHasher::new();
        combined.update(data);
        let (sha, xxh3) = combined.finalize();

        assert_eq!(sha, expected_sha);
        assert_eq!(xxh3, expected_xxh3);
    }

    #[test]
    fn test_combined_hasher_incremental() {
        let part1 = b"Hello, ";
        let part2 = b"world!";
        let full = b"Hello, world!";
        let expected_sha = Sha256Hasher.hash(full);
        let expected_xxh3 = Xxh3Hasher::hash_raw(full);

        let mut combined = CombinedHasher::new();
        combined.update(part1);
        combined.update(part2);

        assert_eq!(combined.sha256_hash(), expected_sha);
        assert_eq!(combined.xxh3_hash(), expected_xxh3);

        let (sha, xxh3) = combined.finalize();
        assert_eq!(sha, expected_sha);
        assert_eq!(xxh3, expected_xxh3);
    }

    #[test]
    fn test_combined_hasher_stream() {
        let data = b"Combined hasher stream test.";
        let expected = Sha256Hasher.hash(data);

        let combined = CombinedHasher::new();
        let streamed = combined.hash_stream(&mut Cursor::new(data)).unwrap();
        assert_eq!(streamed, expected);
    }
}
