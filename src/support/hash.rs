use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::config::HashingConfig;
use crate::foundation::{Error, Result};
use crate::support::token::Token;

/// Password hashing manager using Argon2id.
///
/// ```rust
/// use foundry::HashManager;
/// use foundry::config::HashingConfig;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let config = HashingConfig::default();
/// let manager = HashManager::from_config(&config)?;
///
/// let hash = manager.hash("secret")?;
/// assert!(manager.check("secret", &hash)?);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct HashManager {
    memory_cost: u32,
    time_cost: u32,
    parallelism: u32,
}

impl HashManager {
    /// Create a `HashManager` from the given hashing config.
    ///
    /// Returns an error if the driver is not `"argon2"`.
    pub fn from_config(config: &HashingConfig) -> Result<Self> {
        if config.driver != "argon2" {
            return Err(Error::message(format!(
                "unsupported hashing driver: '{}'. Only 'argon2' is supported.",
                config.driver
            )));
        }

        Ok(Self {
            memory_cost: config.memory_cost,
            time_cost: config.time_cost,
            parallelism: config.parallelism,
        })
    }

    /// Hash a password using Argon2id, returning a PHC-format string.
    pub fn hash(&self, password: &str) -> Result<String> {
        let params = Params::new(self.memory_cost, self.time_cost, self.parallelism, None)
            .map_err(|e| Error::message(format!("invalid argon2 parameters: {e}")))?;

        let hasher = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let salt = SaltString::generate(&mut OsRng);

        let hash = hasher
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| Error::message(format!("failed to hash password: {e}")))?;

        Ok(hash.to_string())
    }

    /// Verify a password against a PHC-format hash string.
    ///
    /// Returns `Ok(false)` if the hash format is invalid or the password
    /// does not match. Returns an error only for unexpected failures.
    pub fn check(&self, password: &str, hash: &str) -> Result<bool> {
        let parsed = match PasswordHash::new(hash) {
            Ok(h) => h,
            Err(_) => return Ok(false),
        };

        let verifier = Argon2::default();
        match verifier.verify_password(password.as_bytes(), &parsed) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Determine whether a stored password hash should be regenerated with
    /// the manager's current Argon2id algorithm, version, or work factors.
    ///
    /// Malformed, incomplete, and non-Argon2id hashes return `Ok(true)` so a
    /// successfully authenticated credential can be upgraded safely.
    pub fn needs_rehash(&self, hash: &str) -> Result<bool> {
        let parsed = match PasswordHash::new(hash) {
            Ok(hash) => hash,
            Err(_) => return Ok(true),
        };

        Ok(parsed.algorithm.as_str() != "argon2id"
            || parsed.version != Some(u32::from(Version::V0x13))
            || parsed.params.get_decimal("m") != Some(self.memory_cost)
            || parsed.params.get_decimal("t") != Some(self.time_cost)
            || parsed.params.get_decimal("p") != Some(self.parallelism)
            || parsed.salt.is_none()
            || parsed.hash.is_none())
    }

    /// Generate a random alphanumeric string of the given length.
    ///
    /// Convenience wrapper around [`Token::generate`].
    pub fn random_string(length: usize) -> Result<String> {
        Token::generate(length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_manager() -> HashManager {
        HashManager::from_config(&HashingConfig::default()).unwrap()
    }

    #[test]
    fn hash_and_check_roundtrip() {
        let manager = default_manager();
        let hash = manager.hash("hunter2").unwrap();
        assert!(manager.check("hunter2", &hash).unwrap());
    }

    #[test]
    fn check_wrong_password_returns_false() {
        let manager = default_manager();
        let hash = manager.hash("correct-horse-battery-staple").unwrap();
        assert!(!manager.check("wrong-password", &hash).unwrap());
    }

    #[test]
    fn check_invalid_hash_returns_false() {
        let manager = default_manager();
        assert!(!manager.check("anything", "not-a-valid-hash").unwrap());
        assert!(!manager.check("anything", "").unwrap());
        assert!(!manager.check("anything", "$argon2id$garbage").unwrap());
    }

    #[test]
    fn from_config_rejects_unknown_driver() {
        let config = HashingConfig {
            driver: "bcrypt".to_string(),
            ..HashingConfig::default()
        };
        let result = HashManager::from_config(&config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unsupported hashing driver"));
    }

    #[test]
    fn hash_produces_phc_format() {
        let manager = default_manager();
        let hash = manager.hash("test-password").unwrap();
        assert!(hash.starts_with("$argon2id$"));
    }

    #[test]
    fn random_string_returns_correct_length() {
        let s = HashManager::random_string(32).unwrap();
        assert_eq!(s.len(), 32);
    }

    #[test]
    fn different_passwords_produce_different_hashes() {
        let manager = default_manager();
        let hash_a = manager.hash("password-a").unwrap();
        let hash_b = manager.hash("password-b").unwrap();
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn same_password_produces_different_hashes() {
        let manager = default_manager();
        let hash_a = manager.hash("same-password").unwrap();
        let hash_b = manager.hash("same-password").unwrap();
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn needs_rehash_accepts_current_argon2id_parameters() {
        let config = HashingConfig {
            memory_cost: 32,
            time_cost: 1,
            parallelism: 1,
            ..HashingConfig::default()
        };
        let manager = HashManager::from_config(&config).unwrap();
        let hash = manager.hash("password").unwrap();

        assert!(!manager.needs_rehash(&hash).unwrap());
    }

    #[test]
    fn needs_rehash_detects_work_factor_algorithm_and_version_changes() {
        let original = HashManager::from_config(&HashingConfig {
            memory_cost: 32,
            time_cost: 1,
            parallelism: 1,
            ..HashingConfig::default()
        })
        .unwrap();
        let hash = original.hash("password").unwrap();

        for config in [
            HashingConfig {
                memory_cost: 64,
                time_cost: 1,
                parallelism: 1,
                ..HashingConfig::default()
            },
            HashingConfig {
                memory_cost: 32,
                time_cost: 2,
                parallelism: 1,
                ..HashingConfig::default()
            },
            HashingConfig {
                memory_cost: 32,
                time_cost: 1,
                parallelism: 2,
                ..HashingConfig::default()
            },
        ] {
            let manager = HashManager::from_config(&config).unwrap();
            assert!(manager.needs_rehash(&hash).unwrap());
        }

        assert!(original
            .needs_rehash(&hash.replacen("$argon2id$", "$argon2i$", 1))
            .unwrap());
        assert!(original
            .needs_rehash(&hash.replacen("v=19", "v=16", 1))
            .unwrap());
    }

    #[test]
    fn needs_rehash_treats_malformed_or_incomplete_hashes_as_stale() {
        let manager = HashManager::from_config(&HashingConfig {
            memory_cost: 32,
            time_cost: 1,
            parallelism: 1,
            ..HashingConfig::default()
        })
        .unwrap();

        assert!(manager.needs_rehash("not-a-password-hash").unwrap());
        assert!(manager
            .needs_rehash("$argon2id$v=19$m=32,t=1,p=1$c2FsdA")
            .unwrap());
    }
}
