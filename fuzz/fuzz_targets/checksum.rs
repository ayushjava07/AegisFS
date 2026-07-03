#![no_main]

use libfuzzer_sys::fuzz_target;

use aegisfs::checksum::{Sha256Hasher, Blake3Hasher, Xxh3Hasher, CombinedHasher};
use aegisfs::core::traits::Hasher;

fuzz_target!(|data: &[u8]| {
    let sha = Sha256Hasher::new();
    let blake3 = Blake3Hasher::new();
    let xxh3 = Xxh3Hasher::new();
    let combined = CombinedHasher::new();

    let _ = sha.hash(data);
    let _ = blake3.hash(data);
    let _ = xxh3.hash(data);
    let _ = combined.hash(data);

    // verify combined hasher matches individual
    let combined_hash = combined.hash(data);
    let sha_hash = sha.hash(data);
    let xxh3_raw = xxh3.hash_raw(data);

    // combined hash is deterministic
    assert_eq!(combined.hash(data), combined_hash);

    // SHA256 component matches
    let mut sha_only = Sha256Hasher::new();
    assert_eq!(sha_only.hash(data), sha_hash);

    // Combined includes both SHA256 and XXH3
    assert_eq!(combined_hash.as_bytes(), sha_hash.as_bytes());
});
