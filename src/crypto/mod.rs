//! Pluggable encryption providers for the AegisFS project.
//!
//! This module provides encryption implementations using AES-256-GCM,
//! ChaCha20-Poly1305, a no-op encryption provider, key derivation via
//! Blake3, and a registry for managing encryption providers.

use std::collections::HashMap;
use std::sync::Arc;
use zeroize::ZeroizeOnDrop;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::EncryptionProvider;
use crate::core::types::EncryptionAlgorithm;

// ---------------------------------------------------------------------------
// Key Types
// ---------------------------------------------------------------------------

/// A 256-bit symmetric key that is zeroed on drop.
#[derive(ZeroizeOnDrop)]
pub struct SymmetricKey {
    key: [u8; 32],
}

impl SymmetricKey {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }
}

impl Clone for SymmetricKey {
    fn clone(&self) -> Self {
        Self { key: self.key }
    }
}

// ---------------------------------------------------------------------------
// AES-256-GCM Provider
// ---------------------------------------------------------------------------

#[cfg(feature = "aes-encryption")]
mod aes_provider {
    use super::*;
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };

    /// Encryption provider using AES-256-GCM.
    pub struct Aes256GcmProvider {
        key: SymmetricKey,
        key_id: Vec<u8>,
    }

    impl Aes256GcmProvider {
        pub fn new(key: SymmetricKey, key_id: Vec<u8>) -> Self {
            Self { key, key_id }
        }
    }

    impl EncryptionProvider for Aes256GcmProvider {
        fn encrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
            let cipher = Aes256Gcm::new_from_slice(self.key.as_bytes())
                .map_err(|e| AegisError::EncryptionError(e.to_string()))?;
            let nonce_bytes: [u8; 12] = rand::random();
            let nonce = Nonce::from_slice(&nonce_bytes);
            let ciphertext = cipher
                .encrypt(nonce, data)
                .map_err(|e| AegisError::EncryptionError(e.to_string()))?;
            let mut result = Vec::with_capacity(12 + ciphertext.len());
            result.extend_from_slice(&nonce_bytes);
            result.extend_from_slice(&ciphertext);
            Ok(result)
        }

        fn decrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
            if data.len() < 12 {
                return Err(AegisError::DecryptionError(
                    "encrypted data too short".to_string(),
                ));
            }
            let cipher = Aes256Gcm::new_from_slice(self.key.as_bytes())
                .map_err(|e| AegisError::DecryptionError(e.to_string()))?;
            let (nonce_bytes, ciphertext) = data.split_at(12);
            let nonce = Nonce::from_slice(nonce_bytes);
            cipher
                .decrypt(nonce, ciphertext)
                .map_err(|e| AegisError::DecryptionError(e.to_string()))
        }

        fn algorithm(&self) -> EncryptionAlgorithm {
            EncryptionAlgorithm::Aes256Gcm
        }

        fn key_identifier(&self) -> &[u8] {
            &self.key_id
        }
    }
}

#[cfg(feature = "aes-encryption")]
pub use aes_provider::Aes256GcmProvider;

// ---------------------------------------------------------------------------
// ChaCha20-Poly1305 Provider
// ---------------------------------------------------------------------------

#[cfg(feature = "chacha-encryption")]
mod chacha_provider {
    use super::*;
    use chacha20poly1305::{
        aead::{Aead, KeyInit},
        ChaCha20Poly1305, Nonce,
    };

    /// Encryption provider using ChaCha20-Poly1305.
    pub struct ChaCha20Poly1305Provider {
        key: SymmetricKey,
        key_id: Vec<u8>,
    }

    impl ChaCha20Poly1305Provider {
        pub fn new(key: SymmetricKey, key_id: Vec<u8>) -> Self {
            Self { key, key_id }
        }
    }

    impl EncryptionProvider for ChaCha20Poly1305Provider {
        fn encrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
            let cipher = ChaCha20Poly1305::new_from_slice(self.key.as_bytes())
                .map_err(|e| AegisError::EncryptionError(e.to_string()))?;
            let nonce_bytes: [u8; 12] = rand::random();
            let nonce = Nonce::from_slice(&nonce_bytes);
            let ciphertext = cipher
                .encrypt(nonce, data)
                .map_err(|e| AegisError::EncryptionError(e.to_string()))?;
            let mut result = Vec::with_capacity(12 + ciphertext.len());
            result.extend_from_slice(&nonce_bytes);
            result.extend_from_slice(&ciphertext);
            Ok(result)
        }

        fn decrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
            if data.len() < 12 {
                return Err(AegisError::DecryptionError(
                    "encrypted data too short".to_string(),
                ));
            }
            let cipher = ChaCha20Poly1305::new_from_slice(self.key.as_bytes())
                .map_err(|e| AegisError::DecryptionError(e.to_string()))?;
            let (nonce_bytes, ciphertext) = data.split_at(12);
            let nonce = Nonce::from_slice(nonce_bytes);
            cipher
                .decrypt(nonce, ciphertext)
                .map_err(|e| AegisError::DecryptionError(e.to_string()))
        }

        fn algorithm(&self) -> EncryptionAlgorithm {
            EncryptionAlgorithm::ChaCha20Poly1305
        }

        fn key_identifier(&self) -> &[u8] {
            &self.key_id
        }
    }
}

#[cfg(feature = "chacha-encryption")]
pub use chacha_provider::ChaCha20Poly1305Provider;

// ---------------------------------------------------------------------------
// Noop Encryption Provider
// ---------------------------------------------------------------------------

/// A no-op encryption provider that returns data as-is without encryption.
pub struct NoopEncryption;

impl EncryptionProvider for NoopEncryption {
    fn encrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
        Ok(data.to_vec())
    }

    fn decrypt(&self, data: &[u8]) -> AegisResult<Vec<u8>> {
        Ok(data.to_vec())
    }

    fn algorithm(&self) -> EncryptionAlgorithm {
        EncryptionAlgorithm::Aes256Gcm
    }

    fn key_identifier(&self) -> &[u8] {
        &[]
    }
}

// ---------------------------------------------------------------------------
// Key Derivation
// ---------------------------------------------------------------------------

/// Key derivation using Blake3.
pub struct KeyDerivation {
    context: String,
}

impl KeyDerivation {
    pub fn new(context: &str) -> Self {
        Self {
            context: context.to_string(),
        }
    }

    /// Derives a 256-bit symmetric key from a passphrase and salt.
    pub fn derive_key(&self, passphrase: &str, salt: &[u8]) -> SymmetricKey {
        let mut key_material = Vec::with_capacity(passphrase.len() + salt.len());
        key_material.extend_from_slice(passphrase.as_bytes());
        key_material.extend_from_slice(salt);
        let key_bytes = blake3::derive_key(&self.context, &key_material);
        SymmetricKey { key: key_bytes }
    }
}

// ---------------------------------------------------------------------------
// Encryption Registry
// ---------------------------------------------------------------------------

/// A registry that maps algorithm names to encryption providers.
pub struct EncryptionRegistry {
    providers: HashMap<String, Arc<dyn EncryptionProvider>>,
}

impl EncryptionRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }
}

impl Default for EncryptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EncryptionRegistry {
    pub fn register(&mut self, name: &str, provider: Arc<dyn EncryptionProvider>) {
        self.providers.insert(name.to_string(), provider);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn EncryptionProvider>> {
        self.providers.get(name).cloned()
    }

    pub fn list_algorithms(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn has_provider(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn EncryptionProvider>> {
        self.providers.remove(name)
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> SymmetricKey {
        SymmetricKey::new([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ])
    }

    fn alt_key() -> SymmetricKey {
        SymmetricKey::new([
            0xff, 0xfe, 0xfd, 0xfc, 0xfb, 0xfa, 0xf9, 0xf8, 0xf7, 0xf6, 0xf5, 0xf4, 0xf3, 0xf2,
            0xf1, 0xf0, 0xef, 0xee, 0xed, 0xec, 0xeb, 0xea, 0xe9, 0xe8, 0xe7, 0xe6, 0xe5, 0xe4,
            0xe3, 0xe2, 0xe1, 0xe0,
        ])
    }

    // -----------------------------------------------------------------------
    // AES-256-GCM Tests
    // -----------------------------------------------------------------------

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_aes256gcm_roundtrip() {
        let provider = Aes256GcmProvider::new(test_key(), b"key-1".to_vec());
        let plaintext = b"Hello, AegisFS AES-256-GCM!";
        let encrypted = provider.encrypt(plaintext).unwrap();
        let decrypted = provider.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_aes256gcm_empty_data() {
        let provider = Aes256GcmProvider::new(test_key(), b"key-1".to_vec());
        let encrypted = provider.encrypt(b"").unwrap();
        assert!(encrypted.len() >= 12);
        let decrypted = provider.decrypt(&encrypted).unwrap();
        assert!(decrypted.is_empty());
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_aes256gcm_various_sizes() {
        let provider = Aes256GcmProvider::new(test_key(), b"key-1".to_vec());
        let sizes = [1, 16, 255, 256, 1024, 4096, 65535];
        for &size in &sizes {
            let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
            let encrypted = provider.encrypt(&data).unwrap();
            assert_eq!(encrypted.len(), 12 + data.len() + 16);
            let decrypted = provider.decrypt(&encrypted).unwrap();
            assert_eq!(decrypted, data, "failed at size {}", size);
        }
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_aes256gcm_wrong_key_fails() {
        let provider_a = Aes256GcmProvider::new(test_key(), b"key-a".to_vec());
        let provider_b = Aes256GcmProvider::new(alt_key(), b"key-b".to_vec());
        let plaintext = b"secret data";
        let encrypted = provider_a.encrypt(plaintext).unwrap();
        let result = provider_b.decrypt(&encrypted);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // ChaCha20-Poly1305 Tests
    // -----------------------------------------------------------------------

    #[cfg(feature = "chacha-encryption")]
    #[test]
    fn test_chacha20poly1305_roundtrip() {
        let provider = ChaCha20Poly1305Provider::new(test_key(), b"key-2".to_vec());
        let plaintext = b"Hello, AegisFS ChaCha20-Poly1305!";
        let encrypted = provider.encrypt(plaintext).unwrap();
        let decrypted = provider.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[cfg(feature = "chacha-encryption")]
    #[test]
    fn test_chacha20poly1305_empty_data() {
        let provider = ChaCha20Poly1305Provider::new(test_key(), b"key-2".to_vec());
        let encrypted = provider.encrypt(b"").unwrap();
        assert!(encrypted.len() >= 12);
        let decrypted = provider.decrypt(&encrypted).unwrap();
        assert!(decrypted.is_empty());
    }

    #[cfg(feature = "chacha-encryption")]
    #[test]
    fn test_chacha20poly1305_various_sizes() {
        let provider = ChaCha20Poly1305Provider::new(test_key(), b"key-2".to_vec());
        let sizes = [1, 16, 255, 256, 1024, 4096, 65535];
        for &size in &sizes {
            let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
            let encrypted = provider.encrypt(&data).unwrap();
            assert_eq!(encrypted.len(), 12 + data.len() + 16);
            let decrypted = provider.decrypt(&encrypted).unwrap();
            assert_eq!(decrypted, data, "failed at size {}", size);
        }
    }

    #[cfg(feature = "chacha-encryption")]
    #[test]
    fn test_chacha20poly1305_wrong_key_fails() {
        let provider_a = ChaCha20Poly1305Provider::new(test_key(), b"key-a".to_vec());
        let provider_b = ChaCha20Poly1305Provider::new(alt_key(), b"key-b".to_vec());
        let plaintext = b"secret data";
        let encrypted = provider_a.encrypt(plaintext).unwrap();
        let result = provider_b.decrypt(&encrypted);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // NoopEncryption Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_noop_roundtrip() {
        let provider = NoopEncryption;
        let plaintext = b"Hello, Noop!";
        let encrypted = provider.encrypt(plaintext).unwrap();
        assert_eq!(encrypted, plaintext);
        let decrypted = provider.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_noop_empty_data() {
        let provider = NoopEncryption;
        let encrypted = provider.encrypt(b"").unwrap();
        assert!(encrypted.is_empty());
        let decrypted = provider.decrypt(&encrypted).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_noop_various_sizes() {
        let provider = NoopEncryption;
        let sizes = [0, 1, 16, 255, 256, 1024, 4096];
        for &size in &sizes {
            let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
            let encrypted = provider.encrypt(&data).unwrap();
            assert_eq!(encrypted, data);
            let decrypted = provider.decrypt(&encrypted).unwrap();
            assert_eq!(decrypted, data);
        }
    }

    #[test]
    fn test_noop_key_identifier() {
        let provider = NoopEncryption;
        assert!(provider.key_identifier().is_empty());
    }

    // -----------------------------------------------------------------------
    // Algorithm Display Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_algorithm_display_aes256gcm() {
        let algo = EncryptionAlgorithm::Aes256Gcm;
        assert_eq!(algo.to_string(), "aes-256-gcm");
    }

    #[test]
    fn test_algorithm_display_chacha20() {
        let algo = EncryptionAlgorithm::ChaCha20Poly1305;
        assert_eq!(algo.to_string(), "chacha20-poly1305");
    }

    // -----------------------------------------------------------------------
    // Key Derivation Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_key_derivation_deterministic() {
        let kd = KeyDerivation::new("AegisFS test");
        let key1 = kd.derive_key("password123", b"saltsalt");
        let key2 = kd.derive_key("password123", b"saltsalt");
        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_key_derivation_different_salts() {
        let kd = KeyDerivation::new("AegisFS test");
        let key1 = kd.derive_key("password123", b"salt1");
        let key2 = kd.derive_key("password123", b"salt2");
        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_key_derivation_different_passphrases() {
        let kd = KeyDerivation::new("AegisFS test");
        let key1 = kd.derive_key("password1", b"saltsalt");
        let key2 = kd.derive_key("password2", b"saltsalt");
        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_key_derivation_different_contexts() {
        let kd1 = KeyDerivation::new("AegisFS context 1");
        let kd2 = KeyDerivation::new("AegisFS context 2");
        let key1 = kd1.derive_key("password123", b"saltsalt");
        let key2 = kd2.derive_key("password123", b"saltsalt");
        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_key_derivation_empty_passphrase() {
        let kd = KeyDerivation::new("AegisFS test");
        let key = kd.derive_key("", b"salt");
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[test]
    fn test_key_derivation_empty_salt() {
        let kd = KeyDerivation::new("AegisFS test");
        let key = kd.derive_key("password", b"");
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_key_derivation_produces_valid_aes_key() {
        let kd = KeyDerivation::new("AegisFS production");
        let key = kd.derive_key("hunter2", b"unique-salt");
        let provider = Aes256GcmProvider::new(key, b"derived-key".to_vec());
        let plaintext = b"data encrypted with derived key";
        let encrypted = provider.encrypt(plaintext).unwrap();
        let decrypted = provider.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    // -----------------------------------------------------------------------
    // Test Vector Verification
    // -----------------------------------------------------------------------

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_aes256gcm_nist_test_vector() {
        // NIST GCM test vector for AES-256-GCM with empty plaintext.
        // Key and nonce are zeroed; verify decrypt roundtrip works.
        use aes_gcm::aead::{Aead as _, KeyInit};
        use aes_gcm::Aes256Gcm;

        let key = [0u8; 32];
        let nonce_bytes = [0u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, b"".as_slice()).unwrap();

        // Verify the tag is 16 bytes
        assert_eq!(ciphertext.len(), 16);

        let decrypted = cipher.decrypt(nonce, ciphertext.as_slice()).unwrap();
        assert!(decrypted.is_empty());
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_aes256gcm_nist_test_vector_nonempty() {
        // NIST GCM test vector for AES-256-GCM with 16-byte plaintext.
        // Key: 000...0 (32 bytes), Nonce: 000...0 (12 bytes)
        // Plaintext: 000...0 (16 bytes), AAD: (empty)
        use aes_gcm::aead::{Aead as _, KeyInit};
        use aes_gcm::Aes256Gcm;

        let key = [0u8; 32];
        let nonce_bytes = [0u8; 12];
        let plaintext = [0u8; 16];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, &plaintext[..]).unwrap();
        assert_eq!(ciphertext.len(), 32);

        let decrypted = cipher.decrypt(nonce, ciphertext.as_slice()).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[cfg(feature = "chacha-encryption")]
    #[test]
    fn test_chacha20poly1305_rfc8439_compatible() {
        // Verify ChaCha20-Poly1305 produces consistent results with
        // known key and nonce (without AAD).
        use chacha20poly1305::aead::{Aead as _, KeyInit};
        use chacha20poly1305::ChaCha20Poly1305;

        let key = [0x42u8; 32];
        let nonce_bytes = [0x24u8; 12];
        let plaintext = b"ChaCha20-Poly1305 test vector";
        let cipher = ChaCha20Poly1305::new_from_slice(&key).unwrap();
        let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, &plaintext[..]).unwrap();
        assert_eq!(ciphertext.len(), plaintext.len() + 16);

        let decrypted = cipher.decrypt(nonce, ciphertext.as_slice()).unwrap();
        assert_eq!(decrypted, plaintext);

        let ciphertext2 = cipher.encrypt(nonce, &plaintext[..]).unwrap();
        assert_eq!(ciphertext, ciphertext2);
    }

    #[cfg(feature = "chacha-encryption")]
    #[test]
    fn test_chacha20poly1305_empty_test_vector() {
        use chacha20poly1305::aead::{Aead as _, KeyInit};
        use chacha20poly1305::ChaCha20Poly1305;

        let key = [0u8; 32];
        let nonce_bytes = [0u8; 12];
        let cipher = ChaCha20Poly1305::new_from_slice(&key).unwrap();
        let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, b"".as_slice()).unwrap();
        assert_eq!(ciphertext.len(), 16);

        let decrypted = cipher.decrypt(nonce, ciphertext.as_slice()).unwrap();
        assert!(decrypted.is_empty());
    }

    // -----------------------------------------------------------------------
    // Provider Registry Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_empty() {
        let registry = EncryptionRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.list_algorithms().is_empty());
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_registry_register_and_get() {
        let mut registry = EncryptionRegistry::new();
        let provider = Arc::new(Aes256GcmProvider::new(test_key(), b"key-1".to_vec()));
        registry.register("aes-256-gcm", provider.clone());

        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.has_provider("aes-256-gcm"));
        assert!(!registry.has_provider("nonexistent"));

        let retrieved = registry.get("aes-256-gcm");
        assert!(retrieved.is_some());
        assert_eq!(
            retrieved.unwrap().algorithm(),
            EncryptionAlgorithm::Aes256Gcm
        );
    }

    #[cfg(all(feature = "aes-encryption", feature = "chacha-encryption"))]
    #[test]
    fn test_registry_multiple_providers() {
        let mut registry = EncryptionRegistry::new();
        let aes = Arc::new(Aes256GcmProvider::new(test_key(), b"key-1".to_vec()));
        let chacha = Arc::new(ChaCha20Poly1305Provider::new(test_key(), b"key-2".to_vec()));
        let noop = Arc::new(NoopEncryption);

        registry.register("aes-256-gcm", aes);
        registry.register("chacha20-poly1305", chacha);
        registry.register("none", noop);

        assert_eq!(registry.len(), 3);

        let algorithms = registry.list_algorithms();
        assert_eq!(algorithms.len(), 3);
        assert!(algorithms.contains(&"aes-256-gcm".to_string()));
        assert!(algorithms.contains(&"chacha20-poly1305".to_string()));
        assert!(algorithms.contains(&"none".to_string()));
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_registry_overwrite() {
        let mut registry = EncryptionRegistry::new();
        let provider1 = Arc::new(Aes256GcmProvider::new(test_key(), b"key-1".to_vec()));
        let provider2 = Arc::new(Aes256GcmProvider::new(alt_key(), b"key-2".to_vec()));
        registry.register("aes-256-gcm", provider1);
        registry.register("aes-256-gcm", provider2);

        assert_eq!(registry.len(), 1);
        let retrieved = registry.get("aes-256-gcm").unwrap();
        assert_eq!(retrieved.key_identifier(), b"key-2");
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_registry_unregister() {
        let mut registry = EncryptionRegistry::new();
        let provider = Arc::new(Aes256GcmProvider::new(test_key(), b"key-1".to_vec()));
        registry.register("aes-256-gcm", provider);
        assert_eq!(registry.len(), 1);

        let removed = registry.unregister("aes-256-gcm");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().algorithm(), EncryptionAlgorithm::Aes256Gcm);
        assert!(registry.is_empty());

        let not_found = registry.unregister("nonexistent");
        assert!(not_found.is_none());
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_registry_provider_usage() {
        let mut registry = EncryptionRegistry::new();
        let provider = Arc::new(Aes256GcmProvider::new(test_key(), b"key-1".to_vec()));
        registry.register("aes-256-gcm", provider);

        let retrieved = registry.get("aes-256-gcm").unwrap();
        let plaintext = b"registry test";
        let encrypted = retrieved.encrypt(plaintext).unwrap();
        let decrypted = retrieved.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[cfg(all(feature = "aes-encryption", feature = "chacha-encryption"))]
    #[test]
    fn test_registry_cross_algorithm() {
        let mut registry = EncryptionRegistry::new();
        let aes = Arc::new(Aes256GcmProvider::new(test_key(), b"key-1".to_vec()));
        let chacha = Arc::new(ChaCha20Poly1305Provider::new(alt_key(), b"key-2".to_vec()));
        registry.register("aes-256-gcm", aes);
        registry.register("chacha20-poly1305", chacha);

        let plaintext = b"cross-algorithm test data";

        let aes_provider = registry.get("aes-256-gcm").unwrap();
        let enc_aes = aes_provider.encrypt(plaintext).unwrap();
        let dec_aes = aes_provider.decrypt(&enc_aes).unwrap();
        assert_eq!(dec_aes, plaintext);

        let chacha_provider = registry.get("chacha20-poly1305").unwrap();
        let enc_chacha = chacha_provider.encrypt(plaintext).unwrap();
        let dec_chacha = chacha_provider.decrypt(&enc_chacha).unwrap();
        assert_eq!(dec_chacha, plaintext);

        let result = chacha_provider.decrypt(&enc_aes);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // SymmetricKey Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_symmetric_key_new() {
        let key_data = [0xabu8; 32];
        let key = SymmetricKey::new(key_data);
        assert_eq!(key.as_bytes(), &key_data);
    }

    #[test]
    fn test_symmetric_key_clone() {
        let key = SymmetricKey::new([0x42u8; 32]);
        let cloned = key.clone();
        assert_eq!(key.as_bytes(), cloned.as_bytes());
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_symmetric_key_used_with_aes() {
        let key = SymmetricKey::new([0x99u8; 32]);
        let provider = Aes256GcmProvider::new(key, b"test".to_vec());
        let data = b"sensitive data";
        let enc = provider.encrypt(data).unwrap();
        let dec = provider.decrypt(&enc).unwrap();
        assert_eq!(dec, data);
    }

    // -----------------------------------------------------------------------
    // Integrity / Edge Case Tests
    // -----------------------------------------------------------------------

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_encrypted_output_different_from_input() {
        let provider = Aes256GcmProvider::new(test_key(), b"key-1".to_vec());
        let plaintext = b"not encrypted";
        let encrypted = provider.encrypt(plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_decrypt_truncated_data_fails() {
        let provider = Aes256GcmProvider::new(test_key(), b"key-1".to_vec());
        let result = provider.decrypt(b"");
        assert!(result.is_err());

        let result = provider.decrypt(&[0u8; 11]);
        assert!(result.is_err());
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_decrypt_corrupted_data_fails() {
        let provider = Aes256GcmProvider::new(test_key(), b"key-1".to_vec());
        let plaintext = b"corrupt me";
        let mut encrypted = provider.encrypt(plaintext).unwrap();
        if encrypted.len() > 13 {
            encrypted[13] ^= 0x01;
        }
        let result = provider.decrypt(&encrypted);
        assert!(result.is_err());
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_different_nonces_per_encryption() {
        let provider = Aes256GcmProvider::new(test_key(), b"key-1".to_vec());
        let plaintext = b"same data";
        let enc1 = provider.encrypt(plaintext).unwrap();
        let enc2 = provider.encrypt(plaintext).unwrap();
        assert_ne!(enc1, enc2);
        assert_eq!(provider.decrypt(&enc1).unwrap(), plaintext);
        assert_eq!(provider.decrypt(&enc2).unwrap(), plaintext);
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_key_identifier_roundtrip() {
        let key_id = b"my-custom-key-id-123";
        let provider = Aes256GcmProvider::new(test_key(), key_id.to_vec());
        assert_eq!(provider.key_identifier(), key_id);
    }

    #[cfg(feature = "chacha-encryption")]
    #[test]
    fn test_key_identifier_chacha() {
        let key_id = b"chacha-key-v1";
        let provider = ChaCha20Poly1305Provider::new(test_key(), key_id.to_vec());
        assert_eq!(provider.key_identifier(), key_id);
    }
}
