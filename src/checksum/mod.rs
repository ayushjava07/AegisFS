pub mod hashers;
pub mod stream;

pub use hashers::*;
pub use stream::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::id::HashValue;
    use crate::core::traits::Hasher;
    use std::io::{Cursor, Read, Write};
    use std::str::FromStr;

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
        let expected_empty = hasher.hash(b"");
        let expected_abc = hasher.hash(b"abc");
        assert_eq!(hasher.hash(b""), expected_empty);
        assert_eq!(hasher.hash(b"abc"), expected_abc);
        let raw_empty = Xxh3Hasher::hash_raw(b"");
        let raw_abc = Xxh3Hasher::hash_raw(b"abc");
        let mut buf = [0u8; 32];
        buf[..8].copy_from_slice(&raw_empty.to_le_bytes());
        assert_eq!(HashValue::from_bytes(buf), expected_empty);
        buf[..8].copy_from_slice(&raw_abc.to_le_bytes());
        assert_eq!(HashValue::from_bytes(buf), expected_abc);
    }

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

    #[test]
    fn test_large_data_hashing() {
        let size = 2_000_000;
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
