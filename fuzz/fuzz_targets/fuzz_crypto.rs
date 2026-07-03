#![no_main]
use libfuzzer_sys::fuzz_target;
use aegisfs::crypto::{
    Aes256GcmProvider, ChaCha20Poly1305Provider, KeyDerivation, SymmetricKey, NoopEncryption,
};
use aegisfs::core::traits::EncryptionProvider;

fuzz_target!(|data: &[u8]| {
    if data.len() < 32 {
        return;
    }

    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&data[0..32]);
    let key = SymmetricKey::new(key_bytes);
    let payload = &data[32..];

    // Fuzz AES-256-GCM if feature is enabled
    #[cfg(feature = "aes-encryption")]
    {
        let provider = Aes256GcmProvider::new(key.clone(), b"fuzz-key-aes".to_vec());
        if let Ok(encrypted) = provider.encrypt(payload) {
            if let Ok(decrypted) = provider.decrypt(&encrypted) {
                assert_eq!(decrypted, payload);
            }
        }
        let _ = provider.decrypt(payload); // Fuzz malformed ciphertexts
    }

    // Fuzz ChaCha20-Poly1305 if feature is enabled
    #[cfg(feature = "chacha-encryption")]
    {
        let provider = ChaCha20Poly1305Provider::new(key.clone(), b"fuzz-key-chacha".to_vec());
        if let Ok(encrypted) = provider.encrypt(payload) {
            if let Ok(decrypted) = provider.decrypt(&encrypted) {
                assert_eq!(decrypted, payload);
            }
        }
        let _ = provider.decrypt(payload); // Fuzz malformed ciphertexts
    }

    // Fuzz NoopEncryption
    {
        let provider = NoopEncryption;
        if let Ok(encrypted) = provider.encrypt(payload) {
            if let Ok(decrypted) = provider.decrypt(&encrypted) {
                assert_eq!(decrypted, payload);
            }
        }
    }

    // Fuzz Key Derivation
    if payload.len() >= 8 {
        let kd = KeyDerivation::new("AegisFS Fuzzing Context");
        let derived = kd.derive_key("fuzz-passphrase", payload);
        assert_eq!(derived.as_bytes().len(), 32);
    }
});
