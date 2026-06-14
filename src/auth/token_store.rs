use std::sync::Arc;
use std::time::Duration;

use crate::database::{Condition, DatabaseManager, DbValue, Query, Sql};
use crate::foundation::{Error, Result};
use crate::support::Token;

const PASSWORD_RESET_TOKENS_TABLE: &str = "password_reset_tokens";

/// Shared token storage for password resets and email verification.
///
/// Both use the `password_reset_tokens` table, differentiated by the `guard`
/// column value.
pub(crate) struct TokenStore {
    database: Arc<DatabaseManager>,
    expiry: Duration,
    kind: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) enum TokenStorePruneScope {
    PasswordResets,
    EmailVerification,
}

impl TokenStore {
    pub fn new(database: Arc<DatabaseManager>, expiry: Duration, kind: &'static str) -> Self {
        Self {
            database,
            expiry,
            kind,
        }
    }

    pub async fn create_token(&self, email: &str, guard: String) -> Result<String> {
        let plaintext = Token::base64(32)?;
        let hash = crate::support::sha256_hex_str(&plaintext);

        Query::insert_into(PASSWORD_RESET_TOKENS_TABLE)
            .values([
                ("email", DbValue::Text(email.to_string())),
                ("guard", DbValue::Text(guard)),
                ("token_hash", DbValue::Text(hash)),
            ])
            .on_conflict_columns(["email", "guard"])
            .do_update()
            .set_excluded("token_hash")
            .set_expr("created_at", Sql::now())
            .execute(&*self.database)
            .await?;

        Ok(plaintext)
    }

    pub async fn validate_token(&self, email: &str, token: &str, guard: String) -> Result<()> {
        let hash = crate::support::sha256_hex_str(token);
        let expiry_seconds = self.expiry.as_secs() as i64;

        let invalid_msg = format!("invalid or expired {} token", self.kind);
        let mut query = Query::delete_from(PASSWORD_RESET_TOKENS_TABLE)
            .where_eq("email", email.to_string())
            .where_eq("guard", guard)
            .where_eq("token_hash", hash);

        if expiry_seconds > 0 {
            // Enforce the TTL inside the DELETE so an expired token is never
            // consumed; deleting first and checking expiry afterwards made
            // every expired attempt destructive.
            query = query.where_(Condition::raw(
                "created_at >= NOW() - (? * INTERVAL '1 second')",
                vec![DbValue::Float64(expiry_seconds as f64)],
            ));
        }

        let rows = query.returning(["created_at"]).get(&*self.database).await?;

        if rows.is_empty() {
            return Err(Error::message(&invalid_msg));
        }

        Ok(())
    }

    pub async fn prune_expired(&self, scope: TokenStorePruneScope) -> Result<u64> {
        self.prune_expired_limited(scope, i64::MAX as u64).await
    }

    pub async fn prune_expired_limited(
        &self,
        scope: TokenStorePruneScope,
        batch_size: u64,
    ) -> Result<u64> {
        if self.expiry.as_secs() == 0 || batch_size == 0 || !self.database.is_configured() {
            return Ok(0);
        }

        let expiry_seconds = self.expiry.as_secs() as i64;
        let sql = match scope {
            TokenStorePruneScope::PasswordResets => {
                r#"
                DELETE FROM password_reset_tokens
                WHERE (email, guard) IN (
                    SELECT email, guard
                    FROM password_reset_tokens
                    WHERE guard NOT LIKE $3
                      AND created_at < NOW() - ($1::double precision * INTERVAL '1 second')
                    ORDER BY created_at ASC
                    LIMIT $2
                )
                "#
            }
            TokenStorePruneScope::EmailVerification => {
                r#"
                DELETE FROM password_reset_tokens
                WHERE (email, guard) IN (
                    SELECT email, guard
                    FROM password_reset_tokens
                    WHERE guard LIKE $3
                      AND created_at < NOW() - ($1::double precision * INTERVAL '1 second')
                    ORDER BY created_at ASC
                    LIMIT $2
                )
                "#
            }
        };
        self.database
            .raw_execute(
                sql,
                &[
                    DbValue::Int64(expiry_seconds),
                    DbValue::Int64(batch_size.min(i64::MAX as u64) as i64),
                    DbValue::Text("verify:%".to_string()),
                ],
            )
            .await
    }
}
