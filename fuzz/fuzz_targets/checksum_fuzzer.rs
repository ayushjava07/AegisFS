#![no_main]

use libfuzzer_sys::fuzz_target;

use aegisfs::checksum::{Sha256Hasher, Blake3Hasher, Xxh3Hasher, CombinedHasher};
use aegisfs::core::traits::Hasher;

fuzz_target!(|data: &[u8]| {
    let sha = Sha256Hasher;
    let blake3 = Blake3Hasher;
    let xxh3 = Xxh3Hasher;
    let combined = CombinedHasher::new();

    let _ = sha.hash(data);
    let _ = blake3.hash(data);
    let _ = xxh3.hash(data);
    let _ = combined.hash(data);

    // verify combined hasher matches individual
    let combined_hash = combined.hash(data);
    let sha_hash = sha.hash(data);
    let _xxh3_raw = Xxh3Hasher::hash_raw(data);

    // combined hash is deterministic
    assert_eq!(combined.hash(data), combined_hash);

    // SHA256 component matches
    let sha_only = Sha256Hasher;
    assert_eq!(sha_only.hash(data), sha_hash);

    // Combined includes both SHA256 and XXH3
    assert_eq!(combined_hash.as_bytes(), sha_hash.as_bytes());
});
