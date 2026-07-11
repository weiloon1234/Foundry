use std::sync::Arc;

use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::config::SessionConfig;
use crate::foundation::{AppContext, Error, Result};
use crate::http::cookie::{
    build_cookie_header_value, clear_cookie_header_value, parse_same_site,
    ClearCookieHeaderOptions, CookieHeaderOptions,
};
use crate::redis::RedisManager;
use crate::support::{GuardId, Token};

use super::{Actor, ActorHydratorRegistry, Authenticatable, SessionCredentialAuthenticator};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionData {
    actor_id: String,
    guard: String,
    #[serde(default)]
    remember: bool,
}

/// Manages Redis-backed sessions for web dashboard authentication.
///
/// Stored as a singleton in the container, accessible via `app.sessions()`.
pub struct SessionManager {
    app: AppContext,
    redis: Arc<RedisManager>,
    config: SessionConfig,
    actor_hydrators: Arc<ActorHydratorRegistry>,
}

impl SessionManager {
    pub(crate) fn new(
        app: AppContext,
        redis: Arc<RedisManager>,
        config: SessionConfig,
        actor_hydrators: Arc<ActorHydratorRegistry>,
    ) -> Self {
        Self {
            app,
            redis,
            config,
            actor_hydrators,
        }
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// Create a new session for the given actor. Returns the session ID.
    pub async fn create<M: Authenticatable>(&self, actor_id: &str) -> Result<String> {
        self.create_with_remember::<M>(actor_id, false).await
    }

    /// Create a new session with optional "remember me" extended lifetime.
    ///
    /// When `remember` is `true`, the session uses the extended TTL
    /// (`remember_ttl_days`) instead of the standard `ttl_minutes`.
    pub async fn create_with_remember<M: Authenticatable>(
        &self,
        actor_id: &str,
        remember: bool,
    ) -> Result<String> {
        let session_id = Token::base64(32)?;
        let guard = M::guard();
        let data = SessionData {
            actor_id: actor_id.to_string(),
            guard: guard.to_string(),
            remember,
        };
        let json = serde_json::to_string(&data).map_err(Error::other)?;
        let ttl_secs = self.session_ttl_secs(remember);

        let mut conn = self.redis.connection().await?;
        let session_key = self.redis.key(format!("session:{session_id}"));
        conn.set_ex(&session_key, &json, ttl_secs).await?;

        let index_key = self.redis.key(format!("session_index:{guard}:{actor_id}"));
        conn.sadd(&index_key, &session_id).await?;
        conn.expire(&index_key, self.index_ttl_secs()).await?;

        Ok(session_id)
    }

    /// Validate a session ID and return the Actor if valid.
    /// Extends TTL if sliding expiry is enabled.
    pub async fn validate(&self, session_id: &str) -> Result<Option<Actor>> {
        let actor = self
            .validate_credential(session_id, self.config.sliding_expiry)
            .await?;
        self.hydrate(actor).await
    }

    async fn validate_credential(
        &self,
        session_id: &str,
        extend_sliding_expiry: bool,
    ) -> Result<Option<Actor>> {
        if !is_valid_session_id(session_id) {
            return Ok(None);
        }

        let mut conn = self.redis.connection().await?;
        let session_key = self.redis.key(format!("session:{session_id}"));

        let json: String = match conn.get(&session_key).await {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };

        if json.is_empty() {
            return Ok(None);
        }

        let data: SessionData = serde_json::from_str(&json).map_err(Error::other)?;

        if extend_sliding_expiry {
            let ttl_secs = self.session_ttl_secs(data.remember);
            conn.expire(&session_key, ttl_secs).await?;
            let index_key = self
                .redis
                .key(format!("session_index:{}:{}", data.guard, data.actor_id));
            conn.expire(&index_key, self.index_ttl_secs()).await?;
        }

        Ok(Some(Actor::new(data.actor_id, GuardId::owned(data.guard))))
    }

    async fn hydrate(&self, actor: Option<Actor>) -> Result<Option<Actor>> {
        let Some(actor) = actor else {
            return Ok(None);
        };
        self.actor_hydrators.hydrate(actor, &self.app).await
    }

    /// Destroy a specific session.
    pub async fn destroy(&self, session_id: &str) -> Result<()> {
        let mut conn = self.redis.connection().await?;
        let session_key = self.redis.key(format!("session:{session_id}"));

        let json: String = match conn.get(&session_key).await {
            Ok(value) => value,
            Err(_) => return Ok(()),
        };

        conn.del(&session_key).await?;

        if !json.is_empty() {
            if let Ok(data) = serde_json::from_str::<SessionData>(&json) {
                let index_key = self
                    .redis
                    .key(format!("session_index:{}:{}", data.guard, data.actor_id));
                conn.srem(&index_key, session_id).await?;
            }
        }

        Ok(())
    }

    /// Destroy all sessions for an actor under a specific guard.
    pub async fn destroy_all<M: Authenticatable>(&self, actor_id: &str) -> Result<()> {
        let guard = M::guard();
        let mut conn = self.redis.connection().await?;
        let index_key = self.redis.key(format!("session_index:{guard}:{actor_id}"));

        let session_ids: Vec<String> = conn.smembers(&index_key).await?;

        let session_keys: Vec<_> = session_ids
            .iter()
            .map(|sid| self.redis.key(format!("session:{sid}")))
            .collect();

        let all_keys: Vec<&_> = session_keys
            .iter()
            .chain(std::iter::once(&index_key))
            .collect();
        conn.del_many(&all_keys).await?;
        Ok(())
    }

    /// Extract session ID from request headers by parsing the Cookie header.
    pub(crate) fn extract_session_id(&self, headers: &HeaderMap) -> Option<String> {
        crate::http::cookie::extract_cookie_value(headers, &self.config.cookie_name)
    }

    /// Build a response that sets the session cookie alongside the given body.
    pub fn login_response(&self, session_id: String, body: impl IntoResponse) -> Result<Response> {
        self.login_response_with_options(session_id, None, body)
    }

    /// Build a response that sets the session cookie and, when requested,
    /// persists it for the configured remember-me TTL.
    pub fn login_response_with_remember(
        &self,
        session_id: String,
        remember: bool,
        body: impl IntoResponse,
    ) -> Result<Response> {
        let max_age_secs = remember.then(|| self.session_ttl_secs(true));
        self.login_response_with_options(session_id, max_age_secs, body)
    }

    /// Build a response that clears the session cookie.
    pub fn logout_response(&self, body: impl IntoResponse) -> Result<Response> {
        let same_site = parse_same_site(&self.config.cookie_same_site)?;
        let cookie = clear_cookie_header_value(ClearCookieHeaderOptions {
            name: &self.config.cookie_name,
            http_only: true,
            secure: self.config.cookie_secure,
            path: &self.config.cookie_path,
            same_site,
            domain: Some(&self.config.cookie_domain),
        })?;
        self.with_cookie_header(cookie, body)
    }

    fn login_response_with_options(
        &self,
        session_id: String,
        max_age_secs: Option<u64>,
        body: impl IntoResponse,
    ) -> Result<Response> {
        let same_site = parse_same_site(&self.config.cookie_same_site)?;
        let cookie = build_cookie_header_value(CookieHeaderOptions {
            name: &self.config.cookie_name,
            value: &session_id,
            http_only: true,
            secure: self.config.cookie_secure,
            path: &self.config.cookie_path,
            same_site,
            domain: Some(&self.config.cookie_domain),
            max_age_secs,
        })?;
        self.with_cookie_header(cookie, body)
    }

    fn session_ttl_secs(&self, remember: bool) -> u64 {
        if remember {
            self.config.remember_ttl_days * 24 * 60 * 60
        } else {
            self.config.ttl_minutes * 60
        }
    }

    fn index_ttl_secs(&self) -> u64 {
        self.session_ttl_secs(false)
            .max(self.session_ttl_secs(true))
    }

    fn with_cookie_header(&self, cookie: HeaderValue, body: impl IntoResponse) -> Result<Response> {
        let mut response = body.into_response();
        response.headers_mut().append(header::SET_COOKIE, cookie);
        Ok(response)
    }
}

#[async_trait::async_trait]
impl SessionCredentialAuthenticator for SessionManager {
    fn extract_session_id(&self, headers: &HeaderMap) -> Option<String> {
        SessionManager::extract_session_id(self, headers)
    }

    async fn authenticate_session(
        &self,
        session_id: &str,
        extend_sliding_expiry: bool,
    ) -> Result<Option<Actor>> {
        self.validate_credential(session_id, extend_sliding_expiry)
            .await
    }
}

const MAX_SESSION_ID_LEN: usize = 128;

fn is_valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= MAX_SESSION_ID_LEN
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use serde_json::json;

    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::validation::RuleRegistry;

    fn manager_with_config(config: SessionConfig) -> SessionManager {
        let repository = ConfigRepository::empty();
        let redis = Arc::new(RedisManager::from_config(&repository).unwrap());
        let app = AppContext::new(Container::new(), repository, RuleRegistry::new()).unwrap();
        let hydrators = Arc::new(crate::auth::ActorHydratorRegistryBuilder::freeze_shared(
            crate::auth::ActorHydratorRegistryBuilder::shared(),
        ));
        SessionManager::new(app, redis, config, hydrators)
    }

    #[test]
    fn session_cookie_responses_honor_configured_path() {
        let manager = manager_with_config(SessionConfig {
            cookie_path: "/admin".to_string(),
            cookie_same_site: "strict".to_string(),
            cookie_domain: "example.com".to_string(),
            cookie_secure: false,
            ..Default::default()
        });

        let login = manager
            .login_response("abc_123-XYZ".to_string(), Json(json!({ "ok": true })))
            .unwrap();
        let login_cookie = login
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(login_cookie.starts_with("foundry_session=abc_123-XYZ;"));
        assert!(login_cookie.contains("HttpOnly"));
        assert!(login_cookie.contains("SameSite=Strict"));
        assert!(login_cookie.contains("Path=/admin"));
        assert!(login_cookie.contains("Domain=example.com"));
        assert!(!login_cookie.contains("Secure"));

        let logout = manager
            .logout_response(Json(json!({ "ok": true })))
            .unwrap();
        let logout_cookie = logout
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(logout_cookie.starts_with("foundry_session=;"));
        assert!(logout_cookie.contains("Path=/admin"));
        assert!(logout_cookie.contains("Domain=example.com"));
        assert!(logout_cookie.contains("Max-Age=0"));
    }

    #[test]
    fn remember_login_response_sets_persistent_cookie_max_age() {
        let manager = manager_with_config(SessionConfig {
            remember_ttl_days: 14,
            cookie_secure: false,
            ..Default::default()
        });

        let login = manager
            .login_response_with_remember(
                "abc_123-XYZ".to_string(),
                true,
                Json(json!({ "ok": true })),
            )
            .unwrap();
        let cookie = login
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();

        assert!(cookie.contains("Max-Age=1209600"));
    }

    #[test]
    fn insecure_same_site_none_session_cookie_returns_error() {
        let manager = manager_with_config(SessionConfig {
            cookie_same_site: "none".to_string(),
            cookie_secure: false,
            ..Default::default()
        });

        let error = manager
            .login_response("abc_123-XYZ".to_string(), Json(json!({ "ok": true })))
            .unwrap_err();

        assert!(error.to_string().contains("SameSite=None requires Secure"));
    }

    #[test]
    fn remember_sessions_keep_remember_ttl_for_sliding_expiry() {
        let manager = manager_with_config(SessionConfig {
            ttl_minutes: 5,
            remember_ttl_days: 14,
            ..Default::default()
        });

        assert_eq!(manager.session_ttl_secs(false), 5 * 60);
        assert_eq!(manager.session_ttl_secs(true), 14 * 24 * 60 * 60);
        assert_eq!(manager.index_ttl_secs(), 14 * 24 * 60 * 60);
    }

    #[test]
    fn legacy_session_payload_deserializes_as_non_remembered() {
        let data: SessionData =
            serde_json::from_str(r#"{"actor_id":"42","guard":"admin"}"#).unwrap();

        assert_eq!(data.actor_id, "42");
        assert_eq!(data.guard, "admin");
        assert!(!data.remember);
    }

    #[test]
    fn session_id_validation_accepts_only_generated_token_shape() {
        assert!(is_valid_session_id("abcDEF012-_"));
        assert!(!is_valid_session_id(""));
        assert!(!is_valid_session_id("abc.def"));
        assert!(!is_valid_session_id("abc def"));
        assert!(!is_valid_session_id(&"a".repeat(MAX_SESSION_ID_LEN + 1)));
    }

    #[tokio::test]
    async fn validate_rejects_malformed_session_id_before_redis_lookup() {
        let manager = manager_with_config(SessionConfig::default());

        let result = manager.validate("not a session id").await.unwrap();

        assert!(result.is_none());
    }
}
