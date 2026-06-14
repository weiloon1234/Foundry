use std::sync::Arc;
use std::time::Duration;

use crate::database::DatabaseManager;
use crate::foundation::Result;

use super::token_store::{TokenStore, TokenStorePruneScope};
use super::Authenticatable;

/// Manages email verification token generation and validation.
///
/// Reuses the `password_reset_tokens` table with a `verify:` prefix on the guard
/// column to distinguish from password reset tokens.
///
/// Access via `app.email_verification()`.
pub struct EmailVerificationManager {
    store: TokenStore,
}

impl EmailVerificationManager {
    pub(crate) fn new(database: Arc<DatabaseManager>, expiry_minutes: u64) -> Self {
        Self {
            store: TokenStore::new(
                database,
                Duration::from_secs(expiry_minutes * 60),
                "verification",
            ),
        }
    }

    fn guard_key<M: Authenticatable>() -> String {
        format!("verify:{}", M::guard())
    }

    /// Generate an email verification token.
    ///
    /// Returns the plaintext token (to be included in the verification URL).
    pub async fn create_token<M: Authenticatable>(&self, email: &str) -> Result<String> {
        self.store.create_token(email, Self::guard_key::<M>()).await
    }

    /// Validate an email verification token.
    ///
    /// Returns `Ok(())` if the token is valid and not expired.
    /// Deletes the token after successful validation (single use).
    pub async fn validate_token<M: Authenticatable>(&self, email: &str, token: &str) -> Result<()> {
        self.store
            .validate_token(email, token, Self::guard_key::<M>())
            .await
    }

    /// Remove all expired verification tokens.
    pub async fn prune_expired(&self) -> Result<u64> {
        self.store
            .prune_expired(TokenStorePruneScope::EmailVerification)
            .await
    }

    /// Remove expired verification tokens in a bounded batch.
    pub async fn prune_expired_limited(&self, batch_size: u64) -> Result<u64> {
        self.store
            .prune_expired_limited(TokenStorePruneScope::EmailVerification, batch_size)
            .await
    }
}
