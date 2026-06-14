use std::fmt::Write;

use sha2::{Digest, Sha256};

/// Compute the SHA-256 hash of raw bytes and return it as a lowercase hex string.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

/// Convenience: SHA-256 hash a string and return lowercase hex.
pub fn sha256_hex_str(s: &str) -> String {
    sha256_hex(s.as_bytes())
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            write!(s, "{b:02x}").unwrap();
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty_string() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hello() {
        assert_eq!(
            sha256_hex_str("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn sha256_deterministic() {
        let a = sha256_hex_str("test-token-123");
        let b = sha256_hex_str("test-token-123");
        assert_eq!(a, b);
    }

    #[test]
    fn sha256_different_inputs_differ() {
        let a = sha256_hex_str("token-a");
        let b = sha256_hex_str("token-b");
        assert_ne!(a, b);
    }
}
