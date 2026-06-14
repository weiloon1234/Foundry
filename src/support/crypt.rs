use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;

use crate::config::CryptConfig;
use crate::foundation::{Error, Result};

/// AES-256-GCM encryption manager.
///
/// Encrypts data using a 256-bit key with randomly generated 96-bit nonces.
/// The output format is `base64(nonce || ciphertext || tag)`.
pub struct CryptManager {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for CryptManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CryptManager")
            .field("cipher", &"<Aes256Gcm>")
            .finish()
    }
}

impl CryptManager {
    /// Create a `CryptManager` from a `CryptConfig`.
    ///
    /// The key must be a base64-encoded string that decodes to exactly 32 bytes.
    pub fn from_config(config: &CryptConfig) -> Result<Self> {
        if config.key.is_empty() {
            return Err(Error::message(
                "Crypt key is empty. Generate one with: Token::base64(32)",
            ));
        }

        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(&config.key)
            .map_err(|e| {
                Error::message(format!(
                    "Crypt key is not valid base64: {e}. Generate one with: Token::base64(32)"
                ))
            })?;

        if key_bytes.len() != 32 {
            return Err(Error::message(format!(
                "Crypt key must be 32 bytes, got {}. Generate one with: Token::base64(32)",
                key_bytes.len()
            )));
        }

        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        Ok(Self { cipher })
    }

    /// Encrypt bytes and return a base64-encoded string.
    ///
    /// The output is `base64(nonce || ciphertext || tag)`, where the nonce is
    /// 12 bytes randomly generated per encryption.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| Error::message(format!("Encryption failed: {e}")))?;

        let mut output = Vec::with_capacity(nonce.len() + ciphertext.len());
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&ciphertext);

        Ok(base64::engine::general_purpose::STANDARD.encode(&output))
    }

    /// Decrypt a base64-encoded ciphertext produced by [`encrypt`](Self::encrypt).
    pub fn decrypt(&self, encoded: &str) -> Result<Vec<u8>> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Error::message(format!("Failed to decode base64 ciphertext: {e}")))?;

        if bytes.len() < 12 {
            return Err(Error::message(
                "Ciphertext is too short to contain a valid nonce",
            ));
        }

        let (nonce_bytes, ciphertext) = bytes.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::message(format!("Decryption failed: {e}")))
    }

    /// Convenience: encrypt a string and return a base64-encoded ciphertext.
    pub fn encrypt_string(&self, plaintext: &str) -> Result<String> {
        self.encrypt(plaintext.as_bytes())
    }

    /// Convenience: decrypt a base64-encoded ciphertext and return a string.
    pub fn decrypt_string(&self, encoded: &str) -> Result<String> {
        let bytes = self.decrypt(encoded)?;
        String::from_utf8(bytes)
            .map_err(|e| Error::message(format!("Decrypted bytes are not valid UTF-8: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::token::Token;

    fn test_config() -> CryptConfig {
        // CryptManager expects STANDARD base64 encoding.
        let key_bytes = Token::bytes(32).unwrap();
        let key = base64::engine::general_purpose::STANDARD.encode(key_bytes);
        CryptConfig { key }
    }

    #[test]
    fn encrypt_decrypt_roundtrip_bytes() {
        let config = test_config();
        let manager = CryptManager::from_config(&config).unwrap();

        let plaintext = b"hello, foundry encryption!";
        let encrypted = manager.encrypt(plaintext).unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_decrypt_roundtrip_string() {
        let config = test_config();
        let manager = CryptManager::from_config(&config).unwrap();

        let plaintext = "The quick brown fox jumps over the lazy dog";
        let encrypted = manager.encrypt_string(plaintext).unwrap();
        let decrypted = manager.decrypt_string(&encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_ciphertexts_for_same_plaintext() {
        let config = test_config();
        let manager = CryptManager::from_config(&config).unwrap();

        let plaintext = b"same data, different nonce";
        let encrypted_a = manager.encrypt(plaintext).unwrap();
        let encrypted_b = manager.encrypt(plaintext).unwrap();

        assert_ne!(encrypted_a, encrypted_b);

        // Both should still decrypt to the same plaintext
        assert_eq!(manager.decrypt(&encrypted_a).unwrap(), plaintext);
        assert_eq!(manager.decrypt(&encrypted_b).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_tampered_ciphertext_fails() {
        let config = test_config();
        let manager = CryptManager::from_config(&config).unwrap();

        let encrypted = manager.encrypt(b"sensitive data").unwrap();

        // Tamper with one character in the base64 string
        let mut tampered = encrypted;
        let bytes = tampered.as_bytes();
        if bytes.len() > 10 {
            let pos = 10;
            let original = bytes[pos];
            let replacement = if original == b'A' { b'B' } else { b'A' };
            // SAFETY: we're replacing an ASCII character with another ASCII character
            let tampered_bytes = unsafe { tampered.as_bytes_mut() };
            tampered_bytes[pos] = replacement;
        }

        let result = manager.decrypt(&tampered);
        assert!(result.is_err());
    }

    #[test]
    fn from_config_empty_key_returns_error() {
        let config = CryptConfig { key: String::new() };
        let result = CryptManager::from_config(&config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"), "error message should mention empty");
        assert!(
            err.contains("Token::base64(32)"),
            "error message should suggest Token::base64(32)"
        );
    }

    #[test]
    fn from_config_wrong_key_length_returns_error() {
        // 16 bytes encoded as base64
        let key_16 = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        let config = CryptConfig { key: key_16 };
        let result = CryptManager::from_config(&config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("32 bytes"),
            "error message should mention 32 bytes requirement"
        );
        assert!(
            err.contains("got 16"),
            "error message should report actual length"
        );
    }

    #[test]
    fn from_config_invalid_base64_returns_error() {
        let config = CryptConfig {
            key: "!!!not-base64!!!".to_string(),
        };
        let result = CryptManager::from_config(&config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("base64"),
            "error message should mention base64"
        );
    }
}
