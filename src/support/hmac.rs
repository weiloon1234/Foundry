use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Compute HMAC-SHA256 of a message using the given key.
/// Returns lowercase hex-encoded digest.
pub fn hmac_sha256_hex(key: &[u8], message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(message.as_bytes());
    let result = mac.finalize().into_bytes();
    crate::support::sha256::hex_encode(&result)
}

/// Constant-time byte comparison to prevent timing attacks.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_produces_hex_digest() {
        let key = b"secret-key";
        let message = "hello world";
        let result = hmac_sha256_hex(key, message);
        assert_eq!(result.len(), 64); // 256 bits = 64 hex chars
    }

    #[test]
    fn hmac_sha256_is_deterministic() {
        let key = b"key";
        let a = hmac_sha256_hex(key, "msg");
        let b = hmac_sha256_hex(key, "msg");
        assert_eq!(a, b);
    }

    #[test]
    fn hmac_sha256_different_keys_differ() {
        let a = hmac_sha256_hex(b"key-a", "msg");
        let b = hmac_sha256_hex(b"key-b", "msg");
        assert_ne!(a, b);
    }

    #[test]
    fn hmac_sha256_different_messages_differ() {
        let key = b"key";
        let a = hmac_sha256_hex(key, "msg-a");
        let b = hmac_sha256_hex(key, "msg-b");
        assert_ne!(a, b);
    }
}
