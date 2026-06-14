use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use clap::{Arg, Command};

use crate::cli::{CommandInvocation, CommandRegistrar};
use crate::config::TokenConfig;
use crate::database::{
    ComparisonOp, DatabaseManager, DbRecord, DbValue, Expr, FromDbValue, Query, Sql,
};
use crate::foundation::{AppContext, Error, Result};
use crate::support::{sha256_hex_str, CommandId, DateTime, GuardId, PermissionId, Token};

use super::{Actor, AuthError, AuthErrorCode, Authenticatable, BearerAuthenticator};

const TOKEN_PRUNE_COMMAND: CommandId = CommandId::new("token:prune");
pub const MFA_PENDING_ABILITY: &str = "auth:mfa_pending";
const PERSONAL_ACCESS_TOKENS_TABLE: &str = "personal_access_tokens";

/// A pair of access + refresh tokens returned to the client after login.
#[derive(
    Debug, Clone, Serialize, Deserialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    #[ts(type = "number")]
    pub expires_in: u64,
    pub token_type: String,
}

/// Standard refresh-token request body for token-auth endpoints.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
    foundry_macros::Validate,
)]
pub struct RefreshTokenRequest {
    #[validate(required)]
    pub refresh_token: String,
}

/// Small typed wrapper for token payloads in HTTP or WebSocket responses.
#[derive(
    Debug, Clone, Serialize, Deserialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct TokenResponse {
    pub tokens: TokenPair,
}

impl TokenResponse {
    pub fn new(tokens: TokenPair) -> Self {
        Self { tokens }
    }

    pub fn into_inner(self) -> TokenPair {
        self.tokens
    }
}

impl From<TokenPair> for TokenResponse {
    fn from(tokens: TokenPair) -> Self {
        Self::new(tokens)
    }
}

/// Small typed wrapper for short-lived WebSocket auth token payloads.
///
/// ```ignore
/// context.publish("auth.ws_token", WsTokenResponse::new(ws_token)).await?;
/// ```
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct WsTokenResponse {
    pub token: String,
}

impl WsTokenResponse {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    pub fn into_inner(self) -> String {
        self.token
    }
}

impl From<String> for WsTokenResponse {
    fn from(token: String) -> Self {
        Self::new(token)
    }
}

impl From<&str> for WsTokenResponse {
    fn from(token: &str) -> Self {
        Self::new(token)
    }
}

/// Manages personal access tokens: issuance, validation, refresh, and revocation.
///
/// Stored as a singleton in the container, accessible via `app.tokens()`.
pub struct TokenManager {
    db: Arc<DatabaseManager>,
    config: TokenConfig,
}

impl TokenManager {
    pub(crate) fn new(db: Arc<DatabaseManager>, config: TokenConfig) -> Self {
        Self { db, config }
    }

    /// Issue a new access + refresh token pair for the given actor.
    pub async fn issue<M: Authenticatable>(&self, actor_id: &str) -> Result<TokenPair> {
        self.issue_named::<M>(actor_id, "").await
    }

    /// Issue a new token pair with a human-readable name (e.g., "My iPhone", "CLI").
    pub async fn issue_named<M: Authenticatable>(
        &self,
        actor_id: &str,
        name: &str,
    ) -> Result<TokenPair> {
        self.insert_token_pair_default_ttl(M::guard().as_ref(), actor_id, name, &[])
            .await
    }

    /// Issue a new token pair with scoped abilities.
    ///
    /// Abilities are stored as a JSON array on the token row and automatically
    /// become [`Actor`] permissions when the token is validated, integrating
    /// with the existing permission system.
    ///
    /// ```ignore
    /// let pair = app.tokens()?.issue_with_abilities::<User>(
    ///     &user.id.to_string(),
    ///     "mobile-app",
    ///     vec!["orders:read".into(), "profile:write".into()],
    /// ).await?;
    /// ```
    pub async fn issue_with_abilities<M: Authenticatable>(
        &self,
        actor_id: &str,
        name: &str,
        abilities: Vec<String>,
    ) -> Result<TokenPair> {
        self.insert_token_pair_default_ttl(M::guard().as_ref(), actor_id, name, &abilities)
            .await
    }

    pub async fn issue_actor(&self, actor: &Actor) -> Result<TokenPair> {
        self.issue_actor_named(actor, "").await
    }

    pub async fn issue_actor_named(&self, actor: &Actor, name: &str) -> Result<TokenPair> {
        self.insert_token_pair_default_ttl(actor.guard.as_ref(), &actor.id, name, &[])
            .await
    }

    pub async fn issue_actor_with_abilities(
        &self,
        actor: &Actor,
        name: &str,
        abilities: Vec<String>,
    ) -> Result<TokenPair> {
        self.insert_token_pair_default_ttl(actor.guard.as_ref(), &actor.id, name, &abilities)
            .await
    }

    pub async fn issue_mfa_pending(
        &self,
        actor: &Actor,
        name: &str,
        ttl_minutes: u64,
    ) -> Result<TokenPair> {
        let ttl_secs = ttl_minutes.max(1) * 60;
        self.insert_token_pair_with_ttl(
            actor.guard.as_ref(),
            &actor.id,
            name,
            &[MFA_PENDING_ABILITY.to_string()],
            ttl_secs,
            ttl_secs,
        )
        .await
    }

    /// Validate an access token and return the Actor if valid.
    ///
    /// Read-only — does not write on every request. Use [`Self::touch`] to update
    /// `last_used_at` if needed for auditing.
    pub async fn validate(&self, access_token: &str) -> Result<Option<Actor>> {
        let hash = sha256_hex_str(access_token);

        let rows = Query::table(PERSONAL_ACCESS_TOKENS_TABLE)
            .select(["guard", "actor_id", "abilities"])
            .where_eq("access_token_hash", hash)
            .where_(Expr::column("revoked_at").is_null())
            .where_(Expr::column("expires_at").compare(ComparisonOp::Gt, Sql::now()))
            .get(&*self.db)
            .await?;

        let Some(row) = rows.first() else {
            return Ok(None);
        };

        let guard = String::from_db_value(
            row.get("guard")
                .ok_or_else(|| Error::message("missing guard column"))?,
        )?;
        let actor_id = String::from_db_value(
            row.get("actor_id")
                .ok_or_else(|| Error::message("missing actor_id column"))?,
        )?;

        let mut actor = Actor::new(actor_id, GuardId::owned(guard));

        // Parse token-scoped abilities into Actor permissions.
        let abilities = token_abilities_from_row(row);
        if !abilities.is_empty() {
            actor = actor.with_permissions(abilities.into_iter().map(PermissionId::owned));
        }

        Ok(Some(actor))
    }

    /// Update `last_used_at` for a token. Call this explicitly when you need
    /// usage tracking — it is not called automatically on every request.
    pub async fn touch(&self, access_token: &str) -> Result<()> {
        let hash = sha256_hex_str(access_token);
        Query::update_table(PERSONAL_ACCESS_TOKENS_TABLE)
            .set_expr("last_used_at", Sql::now())
            .where_eq("access_token_hash", hash)
            .where_(Expr::column("revoked_at").is_null())
            .execute(&*self.db)
            .await?;
        Ok(())
    }

    /// Refresh a token pair using a valid refresh token.
    ///
    /// Atomically revokes the old token (if rotation enabled) and returns the
    /// actor info needed to issue a new pair. A stolen refresh token can only
    /// be used once — concurrent use of the same token will fail for the loser.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenPair> {
        let hash = sha256_hex_str(refresh_token);

        // Atomic: revoke + return in one query to prevent concurrent reuse
        let rows = if self.config.rotate_refresh_tokens {
            Query::update_table(PERSONAL_ACCESS_TOKENS_TABLE)
                .set_expr("revoked_at", Sql::now())
                .where_eq("refresh_token_hash", hash)
                .where_(Expr::column("revoked_at").is_null())
                .where_(Expr::column("refresh_expires_at").compare(ComparisonOp::Gt, Sql::now()))
                .returning(["guard", "actor_id", "name", "abilities"])
                .get(&*self.db)
                .await?
        } else {
            Query::table(PERSONAL_ACCESS_TOKENS_TABLE)
                .select(["guard", "actor_id", "name", "abilities"])
                .where_eq("refresh_token_hash", hash)
                .where_(Expr::column("revoked_at").is_null())
                .where_(Expr::column("refresh_expires_at").compare(ComparisonOp::Gt, Sql::now()))
                .get(&*self.db)
                .await?
        };

        let row = rows.first().ok_or_else(invalid_refresh_token_error)?;
        let refresh_record = TokenRowMetadata::from_row(row)?;

        self.insert_token_pair(
            &refresh_record.guard,
            &refresh_record.actor_id,
            &refresh_record.name,
            &refresh_record.abilities,
        )
        .await
    }

    /// Revoke a specific access token.
    pub async fn revoke(&self, access_token: &str) -> Result<()> {
        let hash = sha256_hex_str(access_token);
        Query::update_table(PERSONAL_ACCESS_TOKENS_TABLE)
            .set_expr("revoked_at", Sql::now())
            .where_eq("access_token_hash", hash)
            .where_(Expr::column("revoked_at").is_null())
            .execute(&*self.db)
            .await?;
        Ok(())
    }

    /// Revoke all tokens for an actor under a specific guard. Returns count revoked.
    pub async fn revoke_all<M: Authenticatable>(&self, actor_id: &str) -> Result<u64> {
        let guard = M::guard();
        revoke_actor_tokens(&*self.db, &guard, actor_id).await
    }

    /// Replace all unrevoked token abilities for an actor under a specific guard.
    ///
    /// This is useful when an actor's permissions change and currently issued
    /// tokens should reflect the new effective ability set without forcing a logout.
    pub async fn sync_abilities<M: Authenticatable>(
        &self,
        actor_id: &str,
        abilities: Vec<String>,
    ) -> Result<u64> {
        let guard = M::guard();
        sync_actor_token_abilities(&*self.db, &guard, actor_id, abilities).await
    }

    /// Delete tokens that are expired or revoked older than the given age.
    ///
    /// Returns the number of tokens deleted.
    pub async fn prune(&self, older_than_days: u64) -> Result<u64> {
        self.prune_limited(older_than_days, i64::MAX as u64).await
    }

    /// Delete tokens in bounded batches.
    ///
    /// `batch_size = 0` is a no-op. This is used by Foundry-owned worker
    /// maintenance so long-running apps do not need to register their own
    /// scheduler just to clean old credential rows.
    pub async fn prune_limited(&self, older_than_days: u64, batch_size: u64) -> Result<u64> {
        if older_than_days == 0 || batch_size == 0 || !self.db.is_configured() {
            return Ok(0);
        }

        self.db
            .raw_execute(
                r#"
                DELETE FROM personal_access_tokens
                WHERE id IN (
                    SELECT id
                    FROM personal_access_tokens
                    WHERE (revoked_at IS NOT NULL AND revoked_at < NOW() - ($1::double precision * INTERVAL '1 day'))
                       OR (expires_at < NOW() - ($1::double precision * INTERVAL '1 day'))
                    ORDER BY created_at ASC
                    LIMIT $2
                )
                "#,
                &[
                    DbValue::Int64(older_than_days as i64),
                    DbValue::Int64(batch_size.min(i64::MAX as u64) as i64),
                ],
            )
            .await
    }

    /// Core token pair creation — shared by issue and refresh.
    async fn insert_token_pair_default_ttl(
        &self,
        guard: &str,
        actor_id: &str,
        name: &str,
        abilities: &[String],
    ) -> Result<TokenPair> {
        let guard_id = GuardId::owned(guard.to_string());
        let expires_in_secs = self.config.access_token_ttl_minutes_for_guard(&guard_id) * 60;
        let refresh_expires_in_secs =
            self.config.refresh_token_ttl_days_for_guard(&guard_id) * 24 * 60 * 60;
        self.insert_token_pair_with_ttl(
            guard,
            actor_id,
            name,
            abilities,
            expires_in_secs,
            refresh_expires_in_secs,
        )
        .await
    }

    async fn insert_token_pair(
        &self,
        guard: &str,
        actor_id: &str,
        name: &str,
        abilities: &[String],
    ) -> Result<TokenPair> {
        self.insert_token_pair_default_ttl(guard, actor_id, name, abilities)
            .await
    }

    async fn insert_token_pair_with_ttl(
        &self,
        guard: &str,
        actor_id: &str,
        name: &str,
        abilities: &[String],
        expires_in_secs: u64,
        refresh_expires_in_secs: u64,
    ) -> Result<TokenPair> {
        let access_plain = Token::base64(self.config.token_length)?;
        let refresh_plain = Token::base64(self.config.token_length)?;

        let access_hash = sha256_hex_str(&access_plain);
        let refresh_hash = sha256_hex_str(&refresh_plain);

        let abilities_json = serde_json::Value::Array(
            abilities
                .iter()
                .map(|a| serde_json::Value::String(a.clone()))
                .collect(),
        );

        Query::insert_into(PERSONAL_ACCESS_TOKENS_TABLE)
            .values([
                ("guard", DbValue::Text(guard.to_string())),
                ("actor_id", DbValue::Text(actor_id.to_string())),
                ("name", DbValue::Text(name.to_string())),
                ("access_token_hash", DbValue::Text(access_hash)),
                ("refresh_token_hash", DbValue::Text(refresh_hash)),
                ("abilities", DbValue::Json(abilities_json)),
                ("expires_at", expires_at_after(expires_in_secs).into()),
                (
                    "refresh_expires_at",
                    expires_at_after(refresh_expires_in_secs).into(),
                ),
            ])
            .execute(&*self.db)
            .await?;

        Ok(TokenPair {
            access_token: access_plain,
            refresh_token: refresh_plain,
            expires_in: expires_in_secs,
            token_type: "Bearer".to_string(),
        })
    }
}

pub fn actor_has_mfa_pending(actor: &Actor) -> bool {
    actor
        .permissions
        .iter()
        .any(|permission| permission.as_ref() == MFA_PENDING_ABILITY)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenRowMetadata {
    guard: String,
    actor_id: String,
    name: String,
    abilities: Vec<String>,
}

impl TokenRowMetadata {
    fn from_row(row: &DbRecord) -> Result<Self> {
        Ok(Self {
            guard: String::from_db_value(
                row.get("guard")
                    .ok_or_else(|| Error::message("missing guard column"))?,
            )?,
            actor_id: String::from_db_value(
                row.get("actor_id")
                    .ok_or_else(|| Error::message("missing actor_id column"))?,
            )?,
            name: row.optional_text("name").unwrap_or_default(),
            abilities: token_abilities_from_row(row),
        })
    }
}

fn token_abilities_from_row(row: &DbRecord) -> Vec<String> {
    row.get("abilities")
        .and_then(|abilities_value| serde_json::Value::from_db_value(abilities_value).ok())
        .and_then(|abilities_json| serde_json::from_value::<Vec<String>>(abilities_json).ok())
        .unwrap_or_default()
}

fn invalid_refresh_token_error() -> Error {
    Error::from(AuthError::unauthorized_code(
        AuthErrorCode::InvalidRefreshToken,
    ))
}

/// A [`BearerAuthenticator`] that validates access tokens from the `personal_access_tokens` table.
///
/// Auto-created during bootstrap for guards with `driver = "token"` in config.
pub struct TokenAuthenticator {
    manager: Arc<TokenManager>,
}

impl TokenAuthenticator {
    pub fn new(manager: Arc<TokenManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl BearerAuthenticator for TokenAuthenticator {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>> {
        self.manager.validate(token).await
    }
}

pub(crate) fn builtin_cli_registrar() -> CommandRegistrar {
    Arc::new(|registry| {
        registry.command(
            TOKEN_PRUNE_COMMAND,
            Command::new(TOKEN_PRUNE_COMMAND.as_str().to_string())
                .about("Delete expired and revoked personal access tokens")
                .arg(
                    Arg::new("days")
                        .long("days")
                        .value_name("DAYS")
                        .default_value("30")
                        .help("Delete tokens expired/revoked more than this many days ago"),
                ),
            |invocation| async move { token_prune_command(invocation).await },
        )?;
        Ok(())
    })
}

async fn token_prune_command(invocation: CommandInvocation) -> Result<()> {
    let days_str = invocation
        .matches()
        .get_one::<String>("days")
        .map(|s| s.as_str())
        .unwrap_or("30");
    let days: u64 = days_str
        .parse()
        .map_err(|_| Error::message("--days must be a positive integer"))?;
    if days == 0 {
        return Err(Error::message("--days must be a positive integer"));
    }

    let tokens = invocation.app().tokens()?;
    let deleted = tokens.prune(days).await?;
    println!("pruned {deleted} token(s) older than {days} day(s)");
    Ok(())
}

// ---------------------------------------------------------------------------
// HasToken trait — Laravel-style HasApiTokens for Authenticatable models
// ---------------------------------------------------------------------------

/// Trait for models that can issue and manage personal access tokens.
///
/// Provides convenient instance methods for token CRUD, similar to
/// Laravel's `HasApiTokens` trait.
///
/// ```ignore
/// impl HasToken for User {}  // uses Authenticatable::guard() automatically
///
/// let pair = user.create_token(&app).await?;
/// let pair = user.create_token_named(&app, "My iPhone").await?;
/// let pair = user.create_token_with_abilities(&app, "ci", vec!["deploy:read".into()]).await?;
/// user.revoke_all_tokens(&app).await?;
/// ```
#[async_trait::async_trait]
pub trait HasToken: super::Authenticatable {
    /// Issue a new access + refresh token pair.
    async fn create_token(&self, app: &AppContext) -> Result<TokenPair> {
        let tokens = app.tokens()?;
        let id = self.token_actor_id();
        tokens.issue::<Self>(&id).await
    }

    /// Issue a named token pair (e.g., "My iPhone", "CLI").
    async fn create_token_named(&self, app: &AppContext, name: &str) -> Result<TokenPair> {
        let tokens = app.tokens()?;
        let id = self.token_actor_id();
        tokens.issue_named::<Self>(&id, name).await
    }

    /// Issue a token pair with scoped abilities.
    async fn create_token_with_abilities(
        &self,
        app: &AppContext,
        name: &str,
        abilities: Vec<String>,
    ) -> Result<TokenPair> {
        let tokens = app.tokens()?;
        let id = self.token_actor_id();
        tokens
            .issue_with_abilities::<Self>(&id, name, abilities)
            .await
    }

    /// Revoke all tokens for this model instance.
    async fn revoke_all_tokens(&self, app: &AppContext) -> Result<u64> {
        let tokens = app.tokens()?;
        let id = self.token_actor_id();
        tokens.revoke_all::<Self>(&id).await
    }

    /// Revoke all tokens for this model instance using the provided executor.
    ///
    /// Use this when token changes should participate in an existing transaction.
    async fn revoke_all_tokens_with<E>(&self, executor: &E) -> Result<u64>
    where
        E: crate::database::QueryExecutor,
    {
        let id = self.token_actor_id();
        revoke_actor_tokens(executor, &Self::guard(), &id).await
    }

    /// Replace all unrevoked token abilities for this model instance.
    async fn sync_token_abilities(&self, app: &AppContext, abilities: Vec<String>) -> Result<u64> {
        let tokens = app.tokens()?;
        let id = self.token_actor_id();
        tokens.sync_abilities::<Self>(&id, abilities).await
    }

    /// Replace all unrevoked token abilities for this model instance using the provided executor.
    ///
    /// Use this when permission changes and token updates should commit together.
    async fn sync_token_abilities_with<E>(
        &self,
        executor: &E,
        abilities: Vec<String>,
    ) -> Result<u64>
    where
        E: crate::database::QueryExecutor,
    {
        let id = self.token_actor_id();
        sync_actor_token_abilities(executor, &Self::guard(), &id, abilities).await
    }

    /// The actor ID used for token operations. Override if your model's
    /// primary key field is not named `id` or needs special formatting.
    fn token_actor_id(&self) -> String;
}

async fn revoke_actor_tokens<E>(executor: &E, guard: &GuardId, actor_id: &str) -> Result<u64>
where
    E: crate::database::QueryExecutor,
{
    Query::update_table(PERSONAL_ACCESS_TOKENS_TABLE)
        .set_expr("revoked_at", Sql::now())
        .where_eq("guard", guard.to_string())
        .where_eq("actor_id", actor_id.to_string())
        .where_(Expr::column("revoked_at").is_null())
        .execute(executor)
        .await
}

async fn sync_actor_token_abilities<E>(
    executor: &E,
    guard: &GuardId,
    actor_id: &str,
    abilities: Vec<String>,
) -> Result<u64>
where
    E: crate::database::QueryExecutor,
{
    let abilities_json = serde_json::Value::Array(
        abilities
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    );

    Query::update_table(PERSONAL_ACCESS_TOKENS_TABLE)
        .value("abilities", DbValue::Json(abilities_json))
        .where_eq("guard", guard.to_string())
        .where_eq("actor_id", actor_id.to_string())
        .where_(Expr::column("revoked_at").is_null())
        .execute(executor)
        .await
}

fn expires_at_after(seconds: u64) -> DateTime {
    DateTime::now().add_seconds(seconds.min(i64::MAX as u64) as i64)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{
        invalid_refresh_token_error, token_abilities_from_row, HasToken, TokenRowMetadata,
        WsTokenResponse,
    };
    use crate::auth::Authenticatable;
    use crate::database::{DbRecord, DbValue, QueryExecutionOptions, QueryExecutor};
    use crate::foundation::Result;
    use crate::support::GuardId;

    #[test]
    fn token_row_metadata_preserves_name_and_abilities() {
        let mut row = DbRecord::new();
        row.insert("guard", DbValue::Text("api".to_string()));
        row.insert("actor_id", DbValue::Text("user-1".to_string()));
        row.insert("name", DbValue::Text("mobile-app".to_string()));
        row.insert(
            "abilities",
            DbValue::Json(serde_json::json!(["reports:view", "ws:chat"])),
        );

        let metadata = TokenRowMetadata::from_row(&row).unwrap();

        assert_eq!(metadata.guard, "api");
        assert_eq!(metadata.actor_id, "user-1");
        assert_eq!(metadata.name, "mobile-app");
        assert_eq!(metadata.abilities, vec!["reports:view", "ws:chat"]);
    }

    #[test]
    fn token_abilities_defaults_to_empty_when_missing_or_invalid() {
        let empty_row = DbRecord::new();
        assert!(token_abilities_from_row(&empty_row).is_empty());

        let mut invalid_row = DbRecord::new();
        invalid_row.insert(
            "abilities",
            DbValue::Json(serde_json::json!({"unexpected": true})),
        );
        assert!(token_abilities_from_row(&invalid_row).is_empty());
    }

    #[test]
    fn invalid_refresh_token_error_uses_standardized_auth_code() {
        let payload = invalid_refresh_token_error().payload();
        assert_eq!(payload["status"], 401);
        assert_eq!(
            payload["message"],
            "The refresh token is invalid or expired."
        );
        assert_eq!(payload["error_code"], "invalid_refresh_token");
        assert_eq!(payload["message_key"], "auth.invalid_refresh_token");
    }

    #[test]
    fn ws_token_response_wraps_token_values() {
        let response = WsTokenResponse::new("ws_123");
        assert_eq!(response.token, "ws_123");
        assert_eq!(response.clone().into_inner(), "ws_123");
        assert_eq!(WsTokenResponse::from("ws_456").token, "ws_456");
        assert_eq!(
            WsTokenResponse::from(String::from("ws_789")).token,
            "ws_789"
        );
    }

    #[tokio::test]
    async fn has_token_syncs_abilities_with_executor() {
        let actor = TestTokenActor {
            id: crate::support::ModelId::from_uuid(uuid::Uuid::nil()),
        };
        let executor = RecordingTokenExecutor::default();

        actor
            .sync_token_abilities_with(&executor, vec!["posts:read".into(), "posts:write".into()])
            .await
            .unwrap();

        let calls = executor.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.contains("SET \"abilities\" = $1::jsonb"));
        assert_eq!(
            calls[0].1,
            vec![
                DbValue::Json(serde_json::json!(["posts:read", "posts:write"])),
                DbValue::Text("admin".to_string()),
                DbValue::Text("actor-1".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn has_token_revokes_all_with_executor() {
        let actor = TestTokenActor {
            id: crate::support::ModelId::from_uuid(uuid::Uuid::nil()),
        };
        let executor = RecordingTokenExecutor::default();

        actor.revoke_all_tokens_with(&executor).await.unwrap();

        let calls = executor.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.contains("SET \"revoked_at\" = NOW()"));
        assert_eq!(
            calls[0].1,
            vec![
                DbValue::Text("admin".to_string()),
                DbValue::Text("actor-1".to_string()),
            ]
        );
    }

    #[derive(serde::Serialize, crate::Model)]
    #[foundry(table = "test_token_actors")]
    struct TestTokenActor {
        id: crate::support::ModelId<Self>,
    }

    impl Authenticatable for TestTokenActor {
        fn guard() -> GuardId {
            GuardId::new("admin")
        }
    }

    impl HasToken for TestTokenActor {
        fn token_actor_id(&self) -> String {
            "actor-1".to_string()
        }
    }

    #[derive(Default)]
    struct RecordingTokenExecutor {
        calls: Mutex<Vec<(String, Vec<DbValue>)>>,
    }

    #[async_trait]
    impl QueryExecutor for RecordingTokenExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            Ok(Vec::new())
        }

        async fn raw_execute_with(
            &self,
            sql: &str,
            bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<u64> {
            self.calls
                .lock()
                .unwrap()
                .push((sql.to_string(), bindings.to_vec()));
            Ok(1)
        }
    }
}
