pub(crate) mod api_docs;
pub(crate) mod api_docs_metadata;
pub(crate) mod env_publish;
pub(crate) mod publish;
pub(crate) mod published;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use toml::Value;

use crate::foundation::{Error, Result};
use crate::logging::{LogFormat, LogLevel};
use crate::support::{GuardId, QueueId, Timezone};

const MIN_SIGNING_KEY_BYTES: usize = 32;
const MIN_AUTH_TOKEN_LENGTH: usize = 32;

/// Official Cloudflare reverse-proxy CIDR ranges.
///
/// Source: <https://www.cloudflare.com/ips/>
pub const CLOUDFLARE_TRUSTED_CIDRS: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

#[derive(Clone)]
pub struct ConfigRepository {
    root: Arc<Value>,
    diagnostics: Arc<ConfigDiagnostics>,
}

#[derive(Clone, Debug, Default)]
struct ConfigDiagnostics {
    unknown_config_keys: Vec<String>,
    unknown_prefixed_env_overlays: Vec<String>,
    legacy_unprefixed_env_overlays: Vec<String>,
}

impl std::fmt::Debug for ConfigRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigRepository")
            .field("root", &crate::support::redaction::REDACTED)
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Environment {
    #[default]
    Development,
    Production,
    Staging,
    Testing,
    Custom(String),
}

impl Environment {
    pub fn from_label(label: impl Into<String>) -> Self {
        let label = label.into();
        let label = label.trim();
        match label.to_ascii_lowercase().as_str() {
            "development" => Self::Development,
            "production" => Self::Production,
            "staging" => Self::Staging,
            "testing" => Self::Testing,
            _ => Self::Custom(label.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
            Self::Staging => "staging",
            Self::Testing => "testing",
            Self::Custom(label) => label.as_str(),
        }
    }

    pub fn is_production(&self) -> bool {
        matches!(self, Self::Production)
    }

    /// Classify the descriptive label only.
    ///
    /// Security-sensitive code should use [`AppConfig::resolved_security_tier`]
    /// so explicit overrides and custom-label fail-closed behavior are honored.
    pub fn is_production_like(&self) -> bool {
        matches!(self, Self::Production | Self::Staging)
    }

    pub fn is_development(&self) -> bool {
        matches!(self, Self::Development)
    }

    pub fn is_staging(&self) -> bool {
        matches!(self, Self::Staging)
    }

    pub fn is_testing(&self) -> bool {
        matches!(self, Self::Testing)
    }
}

impl<'de> Deserialize<'de> for Environment {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let label = String::deserialize(deserializer)?;
        Ok(Self::from_label(label))
    }
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Security posture applied independently from the descriptive environment label.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityTier {
    Relaxed,
    Strict,
}

impl SecurityTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Relaxed => "relaxed",
            Self::Strict => "strict",
        }
    }

    pub const fn is_strict(self) -> bool {
        matches!(self, Self::Strict)
    }
}

impl std::fmt::Display for SecurityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub name: String,
    pub environment: Environment,
    /// Optional explicit security posture. Built-in environment labels derive
    /// a tier when omitted; custom labels fail closed to strict.
    pub security_tier: Option<SecurityTier>,
    pub timezone: Timezone,
    #[serde(default)]
    pub signing_key: String,
    pub background_shutdown_timeout_ms: u64,
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConfig")
            .field("name", &self.name)
            .field("environment", &self.environment)
            .field("security_tier", &self.security_tier)
            .field("timezone", &self.timezone)
            .field("signing_key", &crate::support::redaction::REDACTED)
            .field(
                "background_shutdown_timeout_ms",
                &self.background_shutdown_timeout_ms,
            )
            .finish()
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: "foundry".to_string(),
            environment: Environment::default(),
            security_tier: None,
            timezone: Timezone::utc(),
            signing_key: String::new(),
            background_shutdown_timeout_ms: 30_000,
        }
    }
}

impl AppConfig {
    /// Resolve the security posture from an explicit override or the built-in
    /// environment mapping. Unknown labels fail closed to [`SecurityTier::Strict`].
    pub fn resolved_security_tier(&self) -> SecurityTier {
        self.security_tier.unwrap_or(match &self.environment {
            Environment::Development | Environment::Testing => SecurityTier::Relaxed,
            Environment::Production | Environment::Staging | Environment::Custom(_) => {
                SecurityTier::Strict
            }
        })
    }

    /// Whether a custom environment label still relies on the fail-closed tier.
    pub fn custom_security_tier_requires_confirmation(&self) -> bool {
        matches!(&self.environment, Environment::Custom(_)) && self.security_tier.is_none()
    }

    /// Decode the base64-encoded signing key into raw bytes.
    ///
    /// Returns an error if the key is not configured, contains invalid base64,
    /// or decodes to fewer than 32 bytes.
    pub fn signing_key_bytes(&self) -> crate::foundation::Result<Vec<u8>> {
        if self.signing_key.is_empty() {
            return Err(crate::foundation::Error::message(
                "app.signing_key is not configured — required for signed routes",
            ));
        }
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes = STANDARD.decode(&self.signing_key).map_err(|e| {
            crate::foundation::Error::message(format!("invalid app.signing_key: {e}"))
        })?;
        if bytes.len() < MIN_SIGNING_KEY_BYTES {
            return Err(crate::foundation::Error::message(format!(
                "app.signing_key must decode to at least {MIN_SIGNING_KEY_BYTES} bytes, got {}; generate one with `key:generate`",
                bytes.len()
            )));
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub worker_threads: usize,
    pub max_blocking_threads: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub max_body_size_bytes: usize,
    pub request_timeout_ms: u64,
    pub security_headers: HttpSecurityHeadersConfig,
    pub trusted_proxy: HttpTrustedProxyConfig,
    pub cors: HttpCorsConfig,
    pub csrf: HttpCsrfConfig,
    pub rate_limit: HttpRateLimitConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HttpSecurityHeadersConfig {
    pub enabled: bool,
    pub hsts: bool,
    pub frame_options: String,
    pub referrer_policy: String,
    pub content_security_policy: String,
}

impl Default for HttpSecurityHeadersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hsts: false,
            frame_options: "DENY".to_string(),
            referrer_policy: "strict-origin-when-cross-origin".to_string(),
            content_security_policy: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HttpTrustedProxyConfig {
    pub enabled: bool,
    pub trusted_cidrs: Vec<String>,
    pub headers: Vec<String>,
}

impl Default for HttpTrustedProxyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trusted_cidrs: CLOUDFLARE_TRUSTED_CIDRS
                .iter()
                .map(|cidr| (*cidr).to_string())
                .collect(),
            headers: vec![
                "cf-connecting-ip".to_string(),
                "x-real-ip".to_string(),
                "x-forwarded-for".to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HttpCorsConfig {
    pub enabled: bool,
    pub allowed_origins: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub allowed_headers: Vec<String>,
    pub allow_credentials: bool,
    pub max_age_seconds: u64,
}

impl Default for HttpCorsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_origins: Vec::new(),
            allowed_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "PATCH".to_string(),
                "DELETE".to_string(),
                "OPTIONS".to_string(),
            ],
            allowed_headers: vec![
                "authorization".to_string(),
                "content-type".to_string(),
                "x-request-id".to_string(),
                "x-csrf-token".to_string(),
            ],
            allow_credentials: false,
            max_age_seconds: 600,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HttpCsrfConfig {
    pub enabled: bool,
    pub cookie_name: String,
    pub header_name: String,
    pub cookie_secure: bool,
    pub cookie_path: String,
    pub cookie_same_site: String,
    pub exclude_paths: Vec<String>,
}

impl Default for HttpCsrfConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cookie_name: "foundry_csrf".to_string(),
            header_name: "x-csrf-token".to_string(),
            cookie_secure: true,
            cookie_path: "/".to_string(),
            cookie_same_site: "lax".to_string(),
            exclude_paths: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HttpRateLimitByConfig {
    Ip,
    Actor,
    #[default]
    ActorOrIp,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HttpRateLimitConfig {
    pub enabled: bool,
    pub max_requests: u32,
    pub window_seconds: u64,
    pub by: HttpRateLimitByConfig,
    pub key_prefix: String,
}

impl Default for HttpRateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_requests: 600,
            window_seconds: 60,
            by: HttpRateLimitByConfig::ActorOrIp,
            key_prefix: "http:".to_string(),
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct RedisConfig {
    pub url: String,
    pub namespace: String,
    pub connect_timeout_ms: u64,
    pub command_timeout_ms: u64,
}

impl std::fmt::Debug for RedisConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisConfig")
            .field(
                "url",
                &crate::support::redaction::redact_url_credentials(&self.url),
            )
            .field("namespace", &self.namespace)
            .field("connect_timeout_ms", &self.connect_timeout_ms)
            .field("command_timeout_ms", &self.command_timeout_ms)
            .finish()
    }
}

impl RedisConfig {
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }

    pub fn command_timeout(&self) -> Duration {
        Duration::from_millis(self.command_timeout_ms)
    }
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            namespace: "foundry".to_string(),
            connect_timeout_ms: 5_000,
            command_timeout_ms: 5_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DatabaseModelConfig {
    pub timestamps_default: bool,
    pub soft_deletes_default: bool,
}

impl Default for DatabaseModelConfig {
    fn default() -> Self {
        Self {
            timestamps_default: true,
            soft_deletes_default: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DatabasePoolConfig {
    pub min_connections: Option<u32>,
    pub max_connections: Option<u32>,
    pub acquire_timeout_ms: Option<u64>,
    pub idle_timeout_seconds: Option<u64>,
    pub max_lifetime_seconds: Option<u64>,
    pub connect_lazy: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDatabasePoolConfig {
    pub min_connections: u32,
    pub max_connections: u32,
    pub acquire_timeout_ms: u64,
    pub idle_timeout_seconds: u64,
    pub max_lifetime_seconds: u64,
    pub connect_lazy: bool,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub url: String,
    pub read_url: Option<String>,
    pub schema: String,
    pub migration_table: String,
    pub migration_lock_timeout_ms: u64,
    pub migrations_path: String,
    pub seeders_path: String,
    pub min_connections: u32,
    pub max_connections: u32,
    pub acquire_timeout_ms: u64,
    pub default_per_page: u64,
    pub log_queries: bool,
    pub log_query_bindings: bool,
    pub redact_sql_literals: bool,
    pub slow_query_threshold_ms: u64,
    pub slow_query_retention: usize,
    pub n_plus_one_detection: bool,
    pub n_plus_one_min_repeats: u64,
    pub n_plus_one_retention: usize,
    pub idle_timeout_seconds: u64,
    pub max_lifetime_seconds: u64,
    pub connect_lazy: bool,
    pub write_pool: DatabasePoolConfig,
    pub read_pool: DatabasePoolConfig,
    pub models: DatabaseModelConfig,
}

impl std::fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::support::redaction::redact_url_credentials;

        f.debug_struct("DatabaseConfig")
            .field("url", &redact_url_credentials(&self.url))
            .field(
                "read_url",
                &self.read_url.as_deref().map(redact_url_credentials),
            )
            .field("schema", &self.schema)
            .field("migration_table", &self.migration_table)
            .field("migration_lock_timeout_ms", &self.migration_lock_timeout_ms)
            .field("migrations_path", &self.migrations_path)
            .field("seeders_path", &self.seeders_path)
            .field("min_connections", &self.min_connections)
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout_ms", &self.acquire_timeout_ms)
            .field("default_per_page", &self.default_per_page)
            .field("log_queries", &self.log_queries)
            .field("log_query_bindings", &self.log_query_bindings)
            .field("redact_sql_literals", &self.redact_sql_literals)
            .field("slow_query_threshold_ms", &self.slow_query_threshold_ms)
            .field("slow_query_retention", &self.slow_query_retention)
            .field("n_plus_one_detection", &self.n_plus_one_detection)
            .field("n_plus_one_min_repeats", &self.n_plus_one_min_repeats)
            .field("n_plus_one_retention", &self.n_plus_one_retention)
            .field("idle_timeout_seconds", &self.idle_timeout_seconds)
            .field("max_lifetime_seconds", &self.max_lifetime_seconds)
            .field("connect_lazy", &self.connect_lazy)
            .field("write_pool", &self.write_pool)
            .field("read_pool", &self.read_pool)
            .field("models", &self.models)
            .finish()
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            read_url: None,
            schema: "public".to_string(),
            migration_table: "foundry_migrations".to_string(),
            migration_lock_timeout_ms: 0,
            migrations_path: "database/migrations".to_string(),
            seeders_path: "database/seeders".to_string(),
            min_connections: 1,
            max_connections: 10,
            acquire_timeout_ms: 5_000,
            default_per_page: 15,
            log_queries: false,
            log_query_bindings: false,
            redact_sql_literals: true,
            slow_query_threshold_ms: 500,
            slow_query_retention: 100,
            n_plus_one_detection: true,
            n_plus_one_min_repeats: 10,
            n_plus_one_retention: 100,
            idle_timeout_seconds: 600,
            max_lifetime_seconds: 1800,
            connect_lazy: false,
            write_pool: DatabasePoolConfig::default(),
            read_pool: DatabasePoolConfig::default(),
            models: DatabaseModelConfig::default(),
        }
    }
}

impl DatabaseConfig {
    pub fn write_pool_config(&self) -> ResolvedDatabasePoolConfig {
        self.resolve_pool_config(&self.write_pool)
    }

    pub fn read_pool_config(&self) -> ResolvedDatabasePoolConfig {
        self.resolve_pool_config(&self.read_pool)
    }

    fn resolve_pool_config(&self, pool: &DatabasePoolConfig) -> ResolvedDatabasePoolConfig {
        ResolvedDatabasePoolConfig {
            min_connections: pool.min_connections.unwrap_or(self.min_connections),
            max_connections: pool.max_connections.unwrap_or(self.max_connections),
            acquire_timeout_ms: pool.acquire_timeout_ms.unwrap_or(self.acquire_timeout_ms),
            idle_timeout_seconds: pool
                .idle_timeout_seconds
                .unwrap_or(self.idle_timeout_seconds),
            max_lifetime_seconds: pool
                .max_lifetime_seconds
                .unwrap_or(self.max_lifetime_seconds),
            connect_lazy: pool.connect_lazy.unwrap_or(self.connect_lazy),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WebSocketConfig {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub heartbeat_interval_seconds: u64,
    pub heartbeat_timeout_seconds: u64,
    /// Maximum time a guarded WebSocket connection may reuse a cached actor
    /// before Foundry revalidates its bearer token or session. Values below one
    /// second are treated as one second.
    pub auth_revalidation_interval_seconds: u64,
    /// Maximum accepted WebSocket message bytes. `0` leaves the transport
    /// default in place.
    pub max_message_size_bytes: usize,
    /// Maximum accepted WebSocket frame bytes. `0` leaves the transport
    /// default in place.
    pub max_frame_size_bytes: usize,
    /// Maximum WebSocket write buffer bytes. `0` leaves the transport default
    /// in place.
    pub max_write_buffer_size_bytes: usize,
    pub max_messages_per_second: u32,
    /// Maximum simultaneous WebSocket connections across the process. `0`
    /// disables this cap.
    pub max_connections_global: u32,
    /// Maximum simultaneous anonymous WebSocket connections per resolved
    /// client IP. The count is released after the connection authenticates.
    /// `0` disables this cap.
    pub max_connections_per_ip: u32,
    pub max_connections_per_user: u32,
    /// Maximum active subscriptions per connection. `0` disables this cap.
    pub max_subscriptions_per_connection: usize,
    /// Maximum client-supplied channel identifier bytes. `0` disables this cap.
    pub max_channel_length: usize,
    /// Maximum client-supplied room identifier bytes. `0` disables this cap.
    pub max_room_length: usize,
    /// Maximum client-supplied event identifier bytes. `0` disables this cap.
    pub max_event_length: usize,
    /// Maximum client-supplied ack identifier bytes. `0` disables this cap.
    pub max_ack_id_length: usize,
    /// Maximum queued outbound frames per connection before Foundry drops the
    /// connection to protect process memory.
    pub outbound_buffer_size: usize,
    /// Allow browser WebSocket clients to pass bearer auth in a query
    /// parameter when they cannot set Authorization headers.
    pub query_token_enabled: bool,
    /// Query parameter name used for bearer auth when query tokens are
    /// enabled.
    pub query_token_name: String,
    /// Maximum decoded query-token bytes. `0` disables this cap.
    pub query_token_max_length: usize,
    /// Optional exact Origin allow-list for browser WebSocket handshakes.
    /// Empty remains permissive outside production-like environments. In
    /// production and staging, an empty list allows same-origin browser
    /// handshakes and rejects cross-origin handshakes.
    pub allowed_origins: Vec<String>,
    /// Maximum number of recent messages retained per channel for replay and
    /// observability history.
    pub history_buffer_size: usize,
    /// Idle TTL for `ws:history:<channel>` Redis lists, in seconds.
    /// Refreshed on every published message — active channels never expire;
    /// only channels that go silent for this long get reaped by Redis.
    /// Set to `0` to disable. Default 7 days.
    pub history_ttl_seconds: u64,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3010,
            path: "/ws".to_string(),
            heartbeat_interval_seconds: 30,
            heartbeat_timeout_seconds: 10,
            auth_revalidation_interval_seconds: 30,
            max_message_size_bytes: 1_048_576,
            max_frame_size_bytes: 1_048_576,
            max_write_buffer_size_bytes: 1_048_576,
            max_messages_per_second: 50,
            max_connections_global: 10_000,
            max_connections_per_ip: 100,
            max_connections_per_user: 5,
            max_subscriptions_per_connection: 100,
            max_channel_length: 128,
            max_room_length: 256,
            max_event_length: 128,
            max_ack_id_length: 128,
            outbound_buffer_size: 1024,
            query_token_enabled: true,
            query_token_name: "token".to_string(),
            query_token_max_length: 4096,
            allowed_origins: Vec::new(),
            history_buffer_size: 50,
            history_ttl_seconds: 604_800,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct JobsConfig {
    pub queue: QueueId,
    /// Maximum number of *total attempts* per job before dead-lettering
    /// (like Laravel's `tries`); `1` means no retry after the first failure.
    pub max_retries: u32,
    pub poll_interval_ms: u64,
    pub lease_ttl_ms: u64,
    pub requeue_batch_size: usize,
    pub max_concurrent_jobs: usize,
    pub timeout_seconds: u64,
    pub shutdown_timeout_ms: u64,
    pub track_history: bool,
    pub history_retention_days: u32,
    pub history_prune_interval_ms: u64,
    pub history_prune_batch_size: usize,
    pub queue_priorities: std::collections::HashMap<String, u32>,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            queue: QueueId::new("default"),
            max_retries: 5,
            poll_interval_ms: 100,
            lease_ttl_ms: 30_000,
            requeue_batch_size: 64,
            max_concurrent_jobs: 16,
            timeout_seconds: 300,
            shutdown_timeout_ms: 30_000,
            track_history: true,
            history_retention_days: 30,
            history_prune_interval_ms: 3_600_000,
            history_prune_batch_size: 1_000,
            queue_priorities: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SchedulerConfig {
    pub tick_interval_ms: u64,
    pub leader_lease_ttl_ms: u64,
    pub shutdown_timeout_ms: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval_ms: 1_000,
            leader_lease_ttl_ms: 5_000,
            shutdown_timeout_ms: 30_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub default_guard: GuardId,
    pub bearer_prefix: String,
    pub tokens: TokenConfig,
    pub sessions: SessionConfig,
    pub password_resets: PasswordResetConfig,
    pub email_verification: EmailVerificationConfig,
    pub lockout: LockoutConfig,
    pub mfa: MfaConfig,
    #[serde(default)]
    pub guards: std::collections::HashMap<String, GuardDriverConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            default_guard: GuardId::new("api"),
            bearer_prefix: "Bearer".to_string(),
            tokens: TokenConfig::default(),
            sessions: SessionConfig::default(),
            password_resets: PasswordResetConfig::default(),
            email_verification: EmailVerificationConfig::default(),
            lockout: LockoutConfig::default(),
            mfa: MfaConfig::default(),
            guards: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AuditConfig {
    pub redact_sensitive_fields: bool,
    pub sensitive_fields: Vec<String>,
    /// Retention window consumed by built-in pruning. Zero keeps rows forever.
    pub retention_days: u32,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            redact_sensitive_fields: true,
            sensitive_fields: vec![
                "password".to_string(),
                "password_hash".to_string(),
                "passwd".to_string(),
                "secret".to_string(),
                "secret_key".to_string(),
                "api_key".to_string(),
                "access_key".to_string(),
                "private_key".to_string(),
                "token".to_string(),
                "token_hash".to_string(),
                "access_token".to_string(),
                "refresh_token".to_string(),
                "authorization".to_string(),
                "credential".to_string(),
                "credentials".to_string(),
                "mfa_secret".to_string(),
                "totp_secret".to_string(),
                "otp_secret".to_string(),
                "recovery_code".to_string(),
                "recovery_codes".to_string(),
            ],
            retention_days: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct LockoutConfig {
    pub enabled: bool,
    pub max_failures: u32,
    pub lockout_minutes: u64,
    pub window_minutes: u64,
}

impl Default for LockoutConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_failures: 5,
            lockout_minutes: 15,
            window_minutes: 15,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct MfaConfig {
    pub enabled: bool,
    pub issuer: String,
    pub pending_token_ttl_minutes: u64,
    pub recovery_codes: usize,
    pub required_roles: std::collections::HashMap<String, Vec<String>>,
}

impl Default for MfaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            issuer: "foundry".to_string(),
            pending_token_ttl_minutes: 10,
            recovery_codes: 8,
            required_roles: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct TokenConfig {
    pub access_token_ttl_minutes: u64,
    pub refresh_token_ttl_days: u64,
    pub token_length: usize,
    pub rotate_refresh_tokens: bool,
    pub prune_retention_days: u64,
    pub prune_interval_ms: u64,
    pub prune_batch_size: u64,
    #[serde(default)]
    pub guards: std::collections::HashMap<String, TokenGuardConfig>,
}

impl Default for TokenConfig {
    fn default() -> Self {
        Self {
            access_token_ttl_minutes: 15,
            refresh_token_ttl_days: 30,
            token_length: 32,
            rotate_refresh_tokens: true,
            prune_retention_days: 30,
            prune_interval_ms: 3_600_000,
            prune_batch_size: 1_000,
            guards: std::collections::HashMap::new(),
        }
    }
}

impl TokenConfig {
    pub fn access_token_ttl_minutes_for_guard(&self, guard: &GuardId) -> u64 {
        self.guards
            .get(guard.as_ref())
            .and_then(|config| config.access_token_ttl_minutes)
            .unwrap_or(self.access_token_ttl_minutes)
    }

    pub fn refresh_token_ttl_days_for_guard(&self, guard: &GuardId) -> u64 {
        self.guards
            .get(guard.as_ref())
            .and_then(|config| config.refresh_token_ttl_days)
            .unwrap_or(self.refresh_token_ttl_days)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TokenGuardConfig {
    pub access_token_ttl_minutes: Option<u64>,
    pub refresh_token_ttl_days: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct PasswordResetConfig {
    pub expiry_minutes: u64,
    pub prune_interval_ms: u64,
    pub prune_batch_size: u64,
}

impl Default for PasswordResetConfig {
    fn default() -> Self {
        Self {
            expiry_minutes: 60,
            prune_interval_ms: 3_600_000,
            prune_batch_size: 1_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EmailVerificationConfig {
    pub expiry_minutes: u64,
    pub prune_interval_ms: u64,
    pub prune_batch_size: u64,
}

impl Default for EmailVerificationConfig {
    fn default() -> Self {
        Self {
            expiry_minutes: 1_440,
            prune_interval_ms: 3_600_000,
            prune_batch_size: 1_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    pub ttl_minutes: u64,
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub cookie_path: String,
    pub cookie_same_site: String,
    pub cookie_domain: String,
    pub sliding_expiry: bool,
    pub remember_ttl_days: u64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_minutes: 120,
            cookie_name: "foundry_session".to_string(),
            cookie_secure: true,
            cookie_path: "/".to_string(),
            cookie_same_site: "lax".to_string(),
            cookie_domain: String::new(),
            sliding_expiry: true,
            remember_ttl_days: 30,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GuardDriverConfig {
    pub driver: GuardDriver,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardDriver {
    Token,
    Session,
    Custom,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    pub log_dir: String,
    pub retention_days: u32,
    pub file_queue_capacity: usize,
    pub file_max_record_bytes: usize,
    pub file_flush_timeout_ms: u64,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::default(),
            log_dir: "logs".to_string(),
            retention_days: 30,
            file_queue_capacity: 8_192,
            file_max_record_bytes: 65_536,
            file_flush_timeout_ms: 5_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct I18nConfig {
    pub default_locale: String,
    pub fallback_locale: String,
    pub resource_path: String,
}

impl Default for I18nConfig {
    fn default() -> Self {
        Self {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            resource_path: "locales".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ObservabilityConfig {
    pub enabled: bool,
    pub capture_enabled: bool,
    pub base_path: String,
    pub http_sample_retention: usize,
    pub websocket_channel_retention: usize,
    pub tracing_enabled: bool,
    pub otlp_endpoint: String,
    pub service_name: String,
    pub websocket: WebSocketObservabilityConfig,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_enabled: true,
            base_path: "/_foundry".to_string(),
            http_sample_retention: 500,
            websocket_channel_retention: 500,
            tracing_enabled: false,
            otlp_endpoint: "http://localhost:4317".to_string(),
            service_name: "foundry".to_string(),
            websocket: WebSocketObservabilityConfig::default(),
        }
    }
}

/// Observability options specific to the WebSocket dashboard endpoints.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct WebSocketObservabilityConfig {
    /// When `true`, `/_foundry/ws/history/:channel` includes full `ServerMessage.payload`
    /// for each buffered message. When `false` (the default), payloads are replaced
    /// with their serialized byte length under `payload_size_bytes`, so dashboard
    /// readers cannot see raw message contents.
    pub include_payloads: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HashingConfig {
    pub driver: String,
    #[serde(default = "default_memory_cost")]
    pub memory_cost: u32,
    #[serde(default = "default_time_cost")]
    pub time_cost: u32,
    #[serde(default = "default_parallelism")]
    pub parallelism: u32,
}

fn default_memory_cost() -> u32 {
    19456
}
fn default_time_cost() -> u32 {
    2
}
fn default_parallelism() -> u32 {
    1
}

impl Default for HashingConfig {
    fn default() -> Self {
        Self {
            driver: "argon2".to_string(),
            memory_cost: default_memory_cost(),
            time_cost: default_time_cost(),
            parallelism: default_parallelism(),
        }
    }
}

#[derive(Clone, Deserialize, Default)]
#[serde(default)]
pub struct CryptConfig {
    pub key: String,
    /// Older encryption keys accepted for decryption only, in newest-first order.
    pub previous_keys: Vec<String>,
}

impl std::fmt::Debug for CryptConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CryptConfig")
            .field("key", &crate::support::redaction::REDACTED)
            .field("previous_keys", &crate::support::redaction::REDACTED)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct TypeScriptConfig {
    pub output_dir: String,
}

impl Default for TypeScriptConfig {
    fn default() -> Self {
        Self {
            output_dir: "frontend/shared/types/generated".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DatatableConfig {
    pub max_per_page: u64,
    pub max_export_rows: u64,
}

impl Default for DatatableConfig {
    fn default() -> Self {
        Self {
            max_per_page: 500,
            max_export_rows: 50_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub driver: CacheDriver,
    pub error_mode: CacheErrorMode,
    pub prefix: String,
    pub ttl_seconds: u64,
    pub max_entries: usize,
    pub key_max_length: usize,
    pub remember_singleflight: bool,
    pub remember_distributed_lock: bool,
    pub remember_lock_ttl_ms: u64,
    pub remember_lock_wait_timeout_ms: u64,
    pub remember_lock_poll_ms: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            driver: CacheDriver::Redis,
            error_mode: CacheErrorMode::Strict,
            prefix: "cache:".to_string(),
            ttl_seconds: 3600,
            max_entries: 10000,
            key_max_length: 512,
            remember_singleflight: true,
            remember_distributed_lock: false,
            remember_lock_ttl_ms: 30_000,
            remember_lock_wait_timeout_ms: 5_000,
            remember_lock_poll_ms: 100,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheDriver {
    Redis,
    Memory,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheErrorMode {
    Strict,
    FailOpen,
}

impl Default for ConfigRepository {
    fn default() -> Self {
        Self::empty()
    }
}

impl ConfigRepository {
    pub fn empty() -> Self {
        Self {
            root: Arc::new(Value::Table(Default::default())),
            diagnostics: Arc::new(ConfigDiagnostics::default()),
        }
    }

    pub fn from_dir(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_dir_with_defaults(path, std::iter::empty())
    }

    pub(crate) fn from_dir_with_defaults<I>(path: impl AsRef<Path>, defaults: I) -> Result<Self>
    where
        I: IntoIterator<Item = Value>,
    {
        let path = path.as_ref();
        let mut root = root_with_defaults(defaults);

        let mut entries = fs::read_dir(path)
            .map_err(Error::other)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("toml"))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let file = entry.path();
            let content = fs::read_to_string(&file).map_err(Error::other)?;
            let value: Value = if content.trim().is_empty() {
                Value::Table(Default::default())
            } else {
                toml::from_str(&content).map_err(Error::other)?
            };
            merge_value(&mut root, value);
        }

        let unknown_config_keys = published::unknown_framework_config_keys(&root);
        let overlay_diagnostics = overlay_env_vars(&mut root)?;

        Ok(Self {
            root: Arc::new(root),
            diagnostics: Arc::new(ConfigDiagnostics {
                unknown_config_keys,
                unknown_prefixed_env_overlays: overlay_diagnostics.unknown_prefixed,
                legacy_unprefixed_env_overlays: overlay_diagnostics.legacy_unprefixed,
            }),
        })
    }

    pub fn with_env_overlay_only() -> Result<Self> {
        Self::with_env_overlay_and_defaults(std::iter::empty())
    }

    pub(crate) fn with_env_overlay_and_defaults<I>(defaults: I) -> Result<Self>
    where
        I: IntoIterator<Item = Value>,
    {
        let mut root = root_with_defaults(defaults);
        let unknown_config_keys = published::unknown_framework_config_keys(&root);
        let overlay_diagnostics = overlay_env_vars(&mut root)?;
        Ok(Self {
            root: Arc::new(root),
            diagnostics: Arc::new(ConfigDiagnostics {
                unknown_config_keys,
                unknown_prefixed_env_overlays: overlay_diagnostics.unknown_prefixed,
                legacy_unprefixed_env_overlays: overlay_diagnostics.legacy_unprefixed,
            }),
        })
    }

    pub fn root(&self) -> Arc<Value> {
        self.root.clone()
    }

    pub(crate) fn unknown_config_keys(&self) -> &[String] {
        &self.diagnostics.unknown_config_keys
    }

    pub(crate) fn unknown_prefixed_env_overlays(&self) -> &[String] {
        &self.diagnostics.unknown_prefixed_env_overlays
    }

    pub(crate) fn legacy_unprefixed_env_overlays(&self) -> &[String] {
        &self.diagnostics.legacy_unprefixed_env_overlays
    }

    pub fn value(&self, path: &str) -> Option<Value> {
        let mut current = &*self.root;
        for segment in path.split('.') {
            current = current.get(segment)?;
        }
        Some(current.clone())
    }

    pub fn string(&self, path: &str) -> Option<String> {
        self.value(path)?.as_str().map(ToOwned::to_owned)
    }

    pub fn section<T>(&self, section: &str) -> Result<T>
    where
        T: DeserializeOwned + Default,
    {
        match self.value(section) {
            Some(value) => value.try_into().map_err(Error::other),
            None => Ok(T::default()),
        }
    }

    pub fn server(&self) -> Result<ServerConfig> {
        self.section("server")
    }

    pub fn http(&self) -> Result<HttpConfig> {
        self.section("http")
    }

    pub fn app(&self) -> Result<AppConfig> {
        self.section("app")
    }

    pub fn redis(&self) -> Result<RedisConfig> {
        self.section("redis")
    }

    pub fn database(&self) -> Result<DatabaseConfig> {
        self.section("database")
    }

    pub fn websocket(&self) -> Result<WebSocketConfig> {
        self.section("websocket")
    }

    pub fn jobs(&self) -> Result<JobsConfig> {
        self.section("jobs")
    }

    pub fn runtime(&self) -> Result<RuntimeConfig> {
        self.section("runtime")
    }

    pub fn auth(&self) -> Result<AuthConfig> {
        let auth: AuthConfig = self.section("auth")?;
        if auth.tokens.token_length < MIN_AUTH_TOKEN_LENGTH {
            return Err(Error::message(format!(
                "auth.tokens.token_length must be at least {MIN_AUTH_TOKEN_LENGTH}"
            )));
        }
        Ok(auth)
    }

    pub fn audit(&self) -> Result<AuditConfig> {
        self.section("audit")
    }

    pub fn scheduler(&self) -> Result<SchedulerConfig> {
        self.section("scheduler")
    }

    pub fn logging(&self) -> Result<LoggingConfig> {
        self.section("logging")
    }

    pub fn i18n(&self) -> Result<I18nConfig> {
        self.section("i18n")
    }

    pub fn typescript(&self) -> Result<TypeScriptConfig> {
        self.section("typescript")
    }

    pub fn datatable(&self) -> Result<DatatableConfig> {
        self.section("datatable")
    }

    pub fn observability(&self) -> Result<ObservabilityConfig> {
        self.section("observability")
    }

    pub fn storage(&self) -> Result<crate::storage::StorageConfig> {
        self.section("storage")
    }

    pub fn email(&self) -> Result<crate::email::config::EmailConfig> {
        self.section("email")
    }

    pub fn hashing(&self) -> Result<HashingConfig> {
        self.section("hashing")
    }

    pub fn cache(&self) -> Result<CacheConfig> {
        self.section("cache")
    }

    pub fn crypt(&self) -> Result<CryptConfig> {
        self.section("crypt")
    }
}

fn root_with_defaults<I>(defaults: I) -> Value
where
    I: IntoIterator<Item = Value>,
{
    let mut root = Value::Table(Default::default());
    for defaults in defaults {
        merge_value(&mut root, defaults);
    }
    root
}

fn merge_value(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Table(target_table), Value::Table(source_table)) => {
            for (key, value) in source_table {
                match target_table.get_mut(&key) {
                    Some(existing) => merge_value(existing, value),
                    None => {
                        target_table.insert(key, value);
                    }
                }
            }
        }
        (target, source) => {
            *target = source;
        }
    }
}

/// Explicit namespace for config env overrides (e.g. `FOUNDRY__SERVER__PORT`).
/// Unprefixed `__`-delimited variables are still honored for compatibility,
/// but any ambient process variable containing `__` can collide with them;
/// the prefix is the collision-proof form and wins when both are set.
const ENV_OVERLAY_PREFIX: &str = "FOUNDRY__";

#[derive(Default)]
struct EnvOverlayDiagnostics {
    unknown_prefixed: Vec<String>,
    legacy_unprefixed: Vec<String>,
}

fn overlay_env_vars(root: &mut Value) -> Result<EnvOverlayDiagnostics> {
    overlay_env_vars_from(root, std::env::vars())
}

fn overlay_env_vars_from<I>(root: &mut Value, variables: I) -> Result<EnvOverlayDiagnostics>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut prefixed = Vec::new();
    let mut diagnostics = EnvOverlayDiagnostics::default();
    for (key, raw_value) in variables {
        if let Some(stripped) = key.strip_prefix(ENV_OVERLAY_PREFIX) {
            let path = env_overlay_path(stripped);
            if !path.is_empty() && !published::is_known_framework_config_path(&path) {
                diagnostics.unknown_prefixed.push(key.clone());
            }
            prefixed.push((stripped.to_string(), raw_value));
            continue;
        }
        if !key.contains("__") {
            continue;
        }
        diagnostics.legacy_unprefixed.push(key.clone());
        apply_env_overlay(root, &key, &raw_value)?;
    }

    // Applied last so the explicit namespace overrides unprefixed variables.
    for (key, raw_value) in prefixed {
        apply_env_overlay(root, &key, &raw_value)?;
    }

    diagnostics.unknown_prefixed.sort();
    diagnostics.unknown_prefixed.dedup();
    diagnostics.legacy_unprefixed.sort();
    diagnostics.legacy_unprefixed.dedup();
    Ok(diagnostics)
}

fn apply_env_overlay(root: &mut Value, key: &str, raw_value: &str) -> Result<()> {
    let segments = env_overlay_path(key);

    if segments.is_empty() {
        return Ok(());
    }

    let value = parse_env_value(raw_value)?;
    set_value(root, &segments, value);
    Ok(())
}

fn env_overlay_path(key: &str) -> Vec<String> {
    key.split("__")
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_lowercase())
        .collect()
}

fn parse_env_value(raw: &str) -> Result<Value> {
    if let Ok(boolean) = raw.parse::<bool>() {
        return Ok(Value::Boolean(boolean));
    }
    if let Ok(integer) = raw.parse::<i64>() {
        return Ok(Value::Integer(integer));
    }
    if let Ok(float) = raw.parse::<f64>() {
        return Ok(Value::Float(float));
    }
    if raw.starts_with('[') || raw.starts_with('{') {
        let wrapped = format!("value = {raw}");
        let parsed: BTreeMap<String, Value> = toml::from_str(&wrapped).map_err(Error::other)?;
        if let Some(value) = parsed.get("value") {
            return Ok(value.clone());
        }
    }

    Ok(Value::String(raw.to_string()))
}

fn set_value(root: &mut Value, path: &[String], value: Value) {
    let mut current = root;
    for segment in &path[..path.len() - 1] {
        match current {
            Value::Table(table) => {
                current = table
                    .entry(segment.clone())
                    .or_insert_with(|| Value::Table(Default::default()));
            }
            _ => return,
        }
    }

    if let Value::Table(table) = current {
        table.insert(path[path.len() - 1].clone(), value);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use tempfile::tempdir;
    use toml::Value;

    use super::{
        AppConfig, AuditConfig, AuthConfig, CacheConfig, CacheDriver, CacheErrorMode,
        ConfigRepository, CryptConfig, DatabaseConfig, DatatableConfig, Environment, HttpConfig,
        HttpRateLimitByConfig, JobsConfig, LoggingConfig, ObservabilityConfig, RedisConfig,
        RuntimeConfig, SchedulerConfig, SecurityTier, TypeScriptConfig, WebSocketConfig,
        CLOUDFLARE_TRUSTED_CIDRS,
    };
    use crate::logging::{LogFormat, LogLevel};
    use crate::support::{GuardId, QueueId};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn repository_debug_never_exposes_raw_config_values() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("secrets.toml"),
            r#"
                [app]
                signing_key = "signing-secret"

                [custom]
                nested_token = "arbitrary-secret"
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let debug = format!("{config:?}");

        assert_eq!(debug, "ConfigRepository { root: \"[redacted]\" }");
        assert!(!debug.contains("signing-secret"));
        assert!(!debug.contains("arbitrary-secret"));
    }

    #[test]
    fn merges_config_files_in_lexical_order() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("SERVER__PORT");
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-base.toml"),
            r#"
                [server]
                host = "127.0.0.1"
                port = 3000
            "#,
        )
        .unwrap();
        fs::write(
            directory.path().join("10-override.toml"),
            r#"
                [server]
                port = 4001
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let server = config.server().unwrap();

        assert_eq!(server.host, "127.0.0.1");
        assert_eq!(server.port, 4001);
    }

    #[test]
    fn later_config_files_override_same_section_values() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("JOBS__MAX_CONCURRENT_JOBS");
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("40-runtime.toml"),
            r#"
                [jobs]
                max_concurrent_jobs = 16
                timeout_seconds = 300
            "#,
        )
        .unwrap();
        fs::write(
            directory.path().join("99-local.toml"),
            r#"
                [jobs]
                max_concurrent_jobs = 4
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let jobs = config.jobs().unwrap();

        assert_eq!(jobs.max_concurrent_jobs, 4);
        assert_eq!(jobs.timeout_seconds, 300);
    }

    #[test]
    fn env_overlay_wins_after_split_config_merge() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("SERVER__PORT", "4123");
        let directory = tempdir().unwrap();
        for (filename, content) in super::published::render_sample_config_files() {
            fs::write(directory.path().join(filename), content).unwrap();
        }

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let server = config.server().unwrap();

        std::env::remove_var("SERVER__PORT");
        assert_eq!(server.port, 4123);
    }

    #[test]
    fn foundry_prefixed_env_overlay_wins_over_unprefixed() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("SERVER__PORT", "4123");
        std::env::set_var("FOUNDRY__SERVER__PORT", "5123");
        let directory = tempdir().unwrap();
        for (filename, content) in super::published::render_sample_config_files() {
            fs::write(directory.path().join(filename), content).unwrap();
        }

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let server = config.server().unwrap();

        std::env::remove_var("SERVER__PORT");
        std::env::remove_var("FOUNDRY__SERVER__PORT");
        assert_eq!(server.port, 5123);
    }

    #[test]
    fn env_overlay_supports_nested_database_pool_config() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("FOUNDRY__DATABASE__READ_URL", "postgres://read.example/app");
        std::env::set_var("FOUNDRY__DATABASE__WRITE_POOL__MAX_CONNECTIONS", "3");
        std::env::set_var("FOUNDRY__DATABASE__READ_POOL__MAX_CONNECTIONS", "9");
        std::env::set_var("FOUNDRY__DATABASE__READ_POOL__CONNECT_LAZY", "true");

        let config = ConfigRepository::with_env_overlay_only().unwrap();
        let database = config.database().unwrap();

        std::env::remove_var("FOUNDRY__DATABASE__READ_URL");
        std::env::remove_var("FOUNDRY__DATABASE__WRITE_POOL__MAX_CONNECTIONS");
        std::env::remove_var("FOUNDRY__DATABASE__READ_POOL__MAX_CONNECTIONS");
        std::env::remove_var("FOUNDRY__DATABASE__READ_POOL__CONNECT_LAZY");

        assert_eq!(
            database.read_url.as_deref(),
            Some("postgres://read.example/app")
        );
        assert_eq!(database.write_pool_config().max_connections, 3);
        assert_eq!(database.read_pool_config().max_connections, 9);
        assert!(database.read_pool_config().connect_lazy);
    }

    #[test]
    fn generated_split_config_matches_single_file_config() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("SERVER__PORT");
        let single_directory = tempdir().unwrap();
        fs::write(
            single_directory.path().join("foundry.toml"),
            super::published::render_sample_config(),
        )
        .unwrap();

        let split_directory = tempdir().unwrap();
        for (filename, content) in super::published::render_sample_config_files() {
            fs::write(split_directory.path().join(filename), content).unwrap();
        }

        let single = ConfigRepository::from_dir(single_directory.path()).unwrap();
        let split = ConfigRepository::from_dir(split_directory.path()).unwrap();

        assert_eq!(single.root(), split.root());
    }

    #[test]
    fn parses_app_timezone_config_section() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                timezone = "Asia/Kuala_Lumpur"
                background_shutdown_timeout_ms = 15000
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let app: AppConfig = config.app().unwrap();

        assert_eq!(app.timezone.to_string(), "Asia/Kuala_Lumpur");
        assert_eq!(app.background_shutdown_timeout_ms, 15_000);
    }

    #[test]
    fn signing_key_bytes_requires_valid_strong_base64_key() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let empty = AppConfig::default();
        assert!(empty
            .signing_key_bytes()
            .unwrap_err()
            .to_string()
            .contains("not configured"));

        let invalid = AppConfig {
            signing_key: "not-base64!!!".to_string(),
            ..Default::default()
        };
        assert!(invalid
            .signing_key_bytes()
            .unwrap_err()
            .to_string()
            .contains("invalid app.signing_key"));

        let weak = AppConfig {
            signing_key: STANDARD.encode([0u8; 16]),
            ..Default::default()
        };
        assert!(weak
            .signing_key_bytes()
            .unwrap_err()
            .to_string()
            .contains("at least 32 bytes"));

        let strong = AppConfig {
            signing_key: STANDARD.encode([7u8; 32]),
            ..Default::default()
        };
        assert_eq!(strong.signing_key_bytes().unwrap(), vec![7u8; 32]);
    }

    #[test]
    fn secret_config_debug_output_is_redacted() {
        let crypt = CryptConfig {
            key: "encryption-key-that-must-not-leak".to_string(),
            previous_keys: vec!["previous-key-that-must-not-leak".to_string()],
        };
        let output = format!("{crypt:?}");

        assert!(output.contains("[redacted]"));
        assert!(!output.contains("encryption-key-that-must-not-leak"));
        assert!(!output.contains("previous-key-that-must-not-leak"));
    }

    #[test]
    fn overlays_app_timezone_from_env() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("APP__TIMEZONE", "Asia/Tokyo");

        let config = ConfigRepository::with_env_overlay_only().unwrap();
        let app = config.app().unwrap();

        std::env::remove_var("APP__TIMEZONE");

        assert_eq!(app.timezone.to_string(), "Asia/Tokyo");
    }

    #[test]
    fn parses_staging_environment_as_production_like() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("APP__ENVIRONMENT");
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                environment = "staging"
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let app = config.app().unwrap();

        assert_eq!(app.environment, Environment::Staging);
        assert_eq!(app.environment.to_string(), "staging");
        assert!(app.environment.is_staging());
        assert!(app.environment.is_production_like());
        assert!(!app.environment.is_production());
        assert_eq!(app.resolved_security_tier(), SecurityTier::Strict);
    }

    #[test]
    fn accepts_custom_environment_labels_from_env() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("APP__ENVIRONMENT", "eu-prod");

        let config = ConfigRepository::with_env_overlay_only().unwrap();
        let app = config.app().unwrap();

        std::env::remove_var("APP__ENVIRONMENT");

        assert_eq!(app.environment, Environment::Custom("eu-prod".to_string()));
        assert_eq!(app.environment.to_string(), "eu-prod");
        assert!(!app.environment.is_production_like());
        assert_eq!(app.resolved_security_tier(), SecurityTier::Strict);
        assert!(app.custom_security_tier_requires_confirmation());
    }

    #[test]
    fn explicit_security_tier_overrides_builtin_and_custom_environment_defaults() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                environment = "development"
                security_tier = "strict"
            "#,
        )
        .unwrap();

        let strict = ConfigRepository::from_dir(directory.path())
            .unwrap()
            .app()
            .unwrap();
        assert_eq!(strict.resolved_security_tier(), SecurityTier::Strict);
        assert!(!strict.custom_security_tier_requires_confirmation());

        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                environment = "eu-prod"
                security_tier = "relaxed"
            "#,
        )
        .unwrap();
        let relaxed = ConfigRepository::from_dir(directory.path())
            .unwrap()
            .app()
            .unwrap();
        assert_eq!(relaxed.resolved_security_tier(), SecurityTier::Relaxed);
        assert!(!relaxed.custom_security_tier_requires_confirmation());
    }

    #[test]
    fn diagnostics_find_framework_table_and_field_typos_but_allow_custom_sections() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                enviroment = "production"

                [databse]
                url = "postgres://localhost/foundry"

                [payments]
                provider = "custom"
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();

        assert_eq!(
            config.unknown_config_keys(),
            &["app.enviroment".to_string(), "databse.url".to_string()]
        );
    }

    #[test]
    fn overlay_diagnostics_find_prefixed_typos_and_legacy_unprefixed_names() {
        let mut root = Value::Table(Default::default());
        let diagnostics = super::overlay_env_vars_from(
            &mut root,
            [
                ("SERVER__PORT".to_string(), "4000".to_string()),
                ("FOUNDRY__SERVRE__PORT".to_string(), "5000".to_string()),
                ("FOUNDRY__SERVER__PORT".to_string(), "6000".to_string()),
            ],
        )
        .unwrap();

        assert_eq!(diagnostics.legacy_unprefixed, ["SERVER__PORT"]);
        assert_eq!(diagnostics.unknown_prefixed, ["FOUNDRY__SERVRE__PORT"]);
        assert_eq!(root["server"]["port"].as_integer(), Some(6000));
    }

    #[test]
    fn rejects_invalid_app_timezone() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                timezone = "Mars/Olympus"
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let error = config.app().unwrap_err();

        assert!(error.to_string().contains("invalid timezone"));
    }

    #[test]
    fn overlays_env_vars_using_double_underscore_paths() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("SERVER__PORT");
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-base.toml"),
            r#"
                [server]
                port = 3000
            "#,
        )
        .unwrap();
        std::env::set_var("SERVER__PORT", "4123");

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let server = config.server().unwrap();

        std::env::remove_var("SERVER__PORT");
        assert_eq!(server.port, 4123);
    }

    #[test]
    fn parses_phase_two_config_sections() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("REDIS__URL");
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-runtime.toml"),
            r#"
                [database]
                url = "postgres://foundry:secret@127.0.0.1:5432/foundry"
                schema = "foundry_test"
                migration_table = "schema_migrations"
                migration_lock_timeout_ms = 2500
                migrations_path = "database/migrations"
                seeders_path = "database/seeders"
                max_connections = 2
                connect_lazy = true
                log_query_bindings = true
                redact_sql_literals = false
                slow_query_retention = 40
                n_plus_one_detection = false
                n_plus_one_min_repeats = 7
                n_plus_one_retention = 25

                [database.write_pool]
                max_connections = 4
                connect_lazy = false

                [database.read_pool]
                min_connections = 0
                max_connections = 8
                acquire_timeout_ms = 750
                idle_timeout_seconds = 30
                max_lifetime_seconds = 120

                [redis]
                url = "redis://127.0.0.1/"
                namespace = "foundry-tests"
                connect_timeout_ms = 1750
                command_timeout_ms = 2250

                [websocket]
                port = 4100
                path = "/realtime"
                max_message_size_bytes = 2048
                max_frame_size_bytes = 1024
                max_write_buffer_size_bytes = 4096
                max_subscriptions_per_connection = 25
                max_channel_length = 64
                max_room_length = 80
                max_event_length = 48
                max_ack_id_length = 32
                outbound_buffer_size = 2048
                query_token_enabled = false
                query_token_name = "ws_token"
                query_token_max_length = 512

                [jobs]
                queue = "critical"
                max_retries = 9
                lease_ttl_ms = 45000
                requeue_batch_size = 12
                shutdown_timeout_ms = 12000
                history_retention_days = 45
                history_prune_interval_ms = 60000
                history_prune_batch_size = 250

                [runtime]
                worker_threads = 4
                max_blocking_threads = 64

                [scheduler]
                tick_interval_ms = 250
                leader_lease_ttl_ms = 7000
                shutdown_timeout_ms = 15000
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let database: DatabaseConfig = config.database().unwrap();
        let redis: RedisConfig = config.redis().unwrap();
        let websocket: WebSocketConfig = config.websocket().unwrap();
        let jobs: JobsConfig = config.jobs().unwrap();
        let runtime: RuntimeConfig = config.runtime().unwrap();
        let scheduler: SchedulerConfig = config.scheduler().unwrap();

        assert_eq!(
            database.url,
            "postgres://foundry:secret@127.0.0.1:5432/foundry"
        );
        assert_eq!(database.schema, "foundry_test");
        assert_eq!(database.migration_table, "schema_migrations");
        assert_eq!(database.migration_lock_timeout_ms, 2_500);
        assert_eq!(database.migrations_path, "database/migrations");
        assert_eq!(database.seeders_path, "database/seeders");
        assert_eq!(database.max_connections, 2);
        assert!(database.connect_lazy);
        assert_eq!(database.write_pool_config().max_connections, 4);
        assert!(!database.write_pool_config().connect_lazy);
        assert_eq!(database.read_pool_config().min_connections, 0);
        assert_eq!(database.read_pool_config().max_connections, 8);
        assert_eq!(database.read_pool_config().acquire_timeout_ms, 750);
        assert_eq!(database.read_pool_config().idle_timeout_seconds, 30);
        assert_eq!(database.read_pool_config().max_lifetime_seconds, 120);
        assert!(database.read_pool_config().connect_lazy);
        assert!(database.log_query_bindings);
        assert!(!database.redact_sql_literals);
        assert_eq!(database.slow_query_retention, 40);
        assert!(!database.n_plus_one_detection);
        assert_eq!(database.n_plus_one_min_repeats, 7);
        assert_eq!(database.n_plus_one_retention, 25);
        assert!(database.models.timestamps_default);
        assert!(!database.models.soft_deletes_default);
        assert_eq!(redis.url, "redis://127.0.0.1/");
        assert_eq!(redis.namespace, "foundry-tests");
        assert_eq!(redis.connect_timeout_ms, 1_750);
        assert_eq!(redis.command_timeout_ms, 2_250);
        assert_eq!(redis.connect_timeout(), Duration::from_millis(1_750));
        assert_eq!(redis.command_timeout(), Duration::from_millis(2_250));
        assert_eq!(websocket.path, "/realtime");
        assert_eq!(websocket.port, 4100);
        assert_eq!(websocket.max_message_size_bytes, 2_048);
        assert_eq!(websocket.max_frame_size_bytes, 1_024);
        assert_eq!(websocket.max_write_buffer_size_bytes, 4_096);
        assert_eq!(websocket.max_subscriptions_per_connection, 25);
        assert_eq!(websocket.max_channel_length, 64);
        assert_eq!(websocket.max_room_length, 80);
        assert_eq!(websocket.max_event_length, 48);
        assert_eq!(websocket.max_ack_id_length, 32);
        assert_eq!(websocket.outbound_buffer_size, 2_048);
        assert!(!websocket.query_token_enabled);
        assert_eq!(websocket.query_token_name, "ws_token");
        assert_eq!(websocket.query_token_max_length, 512);
        assert_eq!(jobs.queue, QueueId::new("critical"));
        assert_eq!(jobs.max_retries, 9);
        assert_eq!(jobs.lease_ttl_ms, 45_000);
        assert_eq!(jobs.requeue_batch_size, 12);
        assert_eq!(jobs.max_concurrent_jobs, 16);
        assert_eq!(jobs.shutdown_timeout_ms, 12_000);
        assert_eq!(jobs.history_retention_days, 45);
        assert_eq!(jobs.history_prune_interval_ms, 60_000);
        assert_eq!(jobs.history_prune_batch_size, 250);
        assert_eq!(runtime.worker_threads, 4);
        assert_eq!(runtime.max_blocking_threads, 64);
        assert_eq!(scheduler.tick_interval_ms, 250);
        assert_eq!(scheduler.leader_lease_ttl_ms, 7_000);
        assert_eq!(scheduler.shutdown_timeout_ms, 15_000);
    }

    #[test]
    fn jobs_config_defaults_shutdown_timeout() {
        let jobs: JobsConfig = ConfigRepository::empty().jobs().unwrap();
        assert_eq!(jobs.max_concurrent_jobs, 16);
        assert_eq!(jobs.shutdown_timeout_ms, 30_000);
        assert_eq!(jobs.history_retention_days, 30);
        assert_eq!(jobs.history_prune_interval_ms, 3_600_000);
        assert_eq!(jobs.history_prune_batch_size, 1_000);
    }

    #[test]
    fn runtime_config_defaults_to_tokio_runtime_defaults() {
        let runtime: RuntimeConfig = ConfigRepository::empty().runtime().unwrap();

        assert_eq!(runtime.worker_threads, 0);
        assert_eq!(runtime.max_blocking_threads, 0);
    }

    #[test]
    fn overlays_runtime_config_from_env() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("RUNTIME__WORKER_THREADS", "8");
        std::env::set_var("RUNTIME__MAX_BLOCKING_THREADS", "128");

        let config = ConfigRepository::with_env_overlay_only().unwrap();
        let runtime = config.runtime().unwrap();

        std::env::remove_var("RUNTIME__WORKER_THREADS");
        std::env::remove_var("RUNTIME__MAX_BLOCKING_THREADS");
        assert_eq!(runtime.worker_threads, 8);
        assert_eq!(runtime.max_blocking_threads, 128);
    }

    #[test]
    fn cache_config_defaults_are_production_safe() {
        let cache: CacheConfig = ConfigRepository::empty().cache().unwrap();

        assert_eq!(cache.driver, CacheDriver::Redis);
        assert_eq!(cache.error_mode, CacheErrorMode::Strict);
        assert_eq!(cache.prefix, "cache:");
        assert_eq!(cache.ttl_seconds, 3_600);
        assert_eq!(cache.max_entries, 10_000);
        assert_eq!(cache.key_max_length, 512);
        assert!(cache.remember_singleflight);
        assert!(!cache.remember_distributed_lock);
        assert_eq!(cache.remember_lock_ttl_ms, 30_000);
        assert_eq!(cache.remember_lock_wait_timeout_ms, 5_000);
        assert_eq!(cache.remember_lock_poll_ms, 100);
    }

    #[test]
    fn redis_config_defaults_bound_connection_and_command_waits() {
        let redis = RedisConfig::default();

        assert_eq!(redis.connect_timeout_ms, 5_000);
        assert_eq!(redis.command_timeout_ms, 5_000);
        assert_eq!(redis.connect_timeout(), Duration::from_secs(5));
        assert_eq!(redis.command_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn email_config_defaults_bound_attachment_payloads() {
        let email = ConfigRepository::empty().email().unwrap();

        assert_eq!(email.default, "smtp");
        assert_eq!(email.queue, "default");
        assert_eq!(email.template_path, "templates/emails");
        assert_eq!(email.max_attachment_bytes, 25 * 1024 * 1024);
        assert_eq!(email.max_total_attachment_bytes, 25 * 1024 * 1024);
    }

    #[test]
    fn parses_email_attachment_limits() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-email.toml"),
            r#"
                [email]
                default = "log"
                queue = "mail"
                template_path = "resources/mail"
                max_attachment_bytes = 1024
                max_total_attachment_bytes = 2048

                [email.mailers.log]
                driver = "log"
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let email = config.email().unwrap();

        assert_eq!(email.default, "log");
        assert_eq!(email.queue, "mail");
        assert_eq!(email.template_path, "resources/mail");
        assert_eq!(email.max_attachment_bytes, 1_024);
        assert_eq!(email.max_total_attachment_bytes, 2_048);
    }

    #[test]
    fn http_config_defaults_are_compatible_and_discoverable() {
        let http: HttpConfig = ConfigRepository::empty().http().unwrap();

        assert_eq!(http.max_body_size_bytes, 0);
        assert_eq!(http.request_timeout_ms, 0);
        assert!(http.security_headers.enabled);
        assert!(!http.security_headers.hsts);
        assert_eq!(http.security_headers.frame_options, "DENY");
        assert!(http.trusted_proxy.enabled);
        assert_eq!(
            http.trusted_proxy
                .trusted_cidrs
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            CLOUDFLARE_TRUSTED_CIDRS
        );
        assert_eq!(
            http.trusted_proxy.headers,
            vec!["cf-connecting-ip", "x-real-ip", "x-forwarded-for"]
        );
        assert!(!http.cors.enabled);
        assert_eq!(
            http.cors.allowed_methods,
            vec!["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
        );
        assert_eq!(
            http.cors.allowed_headers,
            vec![
                "authorization",
                "content-type",
                "x-request-id",
                "x-csrf-token"
            ]
        );
        assert!(!http.csrf.enabled);
        assert_eq!(http.csrf.cookie_name, "foundry_csrf");
        assert_eq!(http.csrf.header_name, "x-csrf-token");
        assert!(http.csrf.cookie_secure);
        assert_eq!(http.csrf.cookie_path, "/");
        assert_eq!(http.csrf.cookie_same_site, "lax");
        assert!(http.csrf.exclude_paths.is_empty());
        assert!(http.rate_limit.enabled);
        assert_eq!(http.rate_limit.max_requests, 600);
        assert_eq!(http.rate_limit.window_seconds, 60);
        assert_eq!(http.rate_limit.by, HttpRateLimitByConfig::ActorOrIp);
        assert_eq!(http.rate_limit.key_prefix, "http:");
    }

    #[test]
    fn websocket_config_defaults_bound_runtime_edges() {
        let websocket: WebSocketConfig = ConfigRepository::empty().websocket().unwrap();

        assert_eq!(websocket.heartbeat_interval_seconds, 30);
        assert_eq!(websocket.heartbeat_timeout_seconds, 10);
        assert_eq!(websocket.auth_revalidation_interval_seconds, 30);
        assert_eq!(websocket.max_message_size_bytes, 1_048_576);
        assert_eq!(websocket.max_frame_size_bytes, 1_048_576);
        assert_eq!(websocket.max_write_buffer_size_bytes, 1_048_576);
        assert_eq!(websocket.max_messages_per_second, 50);
        assert_eq!(websocket.max_connections_global, 10_000);
        assert_eq!(websocket.max_connections_per_ip, 100);
        assert_eq!(websocket.max_connections_per_user, 5);
        assert_eq!(websocket.max_subscriptions_per_connection, 100);
        assert_eq!(websocket.max_channel_length, 128);
        assert_eq!(websocket.max_room_length, 256);
        assert_eq!(websocket.max_event_length, 128);
        assert_eq!(websocket.max_ack_id_length, 128);
        assert_eq!(websocket.outbound_buffer_size, 1_024);
        assert!(websocket.query_token_enabled);
        assert_eq!(websocket.query_token_name, "token");
        assert_eq!(websocket.query_token_max_length, 4_096);
        assert!(websocket.allowed_origins.is_empty());
    }

    #[test]
    fn parses_http_config_section() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-http.toml"),
            r#"
                [http]
                max_body_size_bytes = 1048576
                request_timeout_ms = 2500

                [http.security_headers]
                hsts = true
                frame_options = "SAMEORIGIN"
                referrer_policy = "no-referrer"
                content_security_policy = "default-src 'self'"

                [http.trusted_proxy]
                enabled = true
                trusted_cidrs = ["10.0.0.0/8", "2001:db8::/32"]
                headers = ["x-forwarded-for"]

                [http.cors]
                enabled = true
                allowed_origins = ["https://example.com"]
                allowed_methods = ["GET", "POST"]
                allowed_headers = ["authorization"]
                allow_credentials = true
                max_age_seconds = 1200

                [http.csrf]
                enabled = true
                cookie_name = "app_csrf"
                header_name = "x-app-csrf"
                cookie_secure = false
                cookie_path = "/admin"
                cookie_same_site = "strict"
                exclude_paths = ["/api", "/webhooks"]

                [http.rate_limit]
                enabled = true
                max_requests = 25
                window_seconds = 10
                by = "actor_or_ip"
                key_prefix = "edge:"
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let http: HttpConfig = config.http().unwrap();

        assert_eq!(http.max_body_size_bytes, 1_048_576);
        assert_eq!(http.request_timeout_ms, 2_500);
        assert!(http.security_headers.hsts);
        assert_eq!(http.security_headers.frame_options, "SAMEORIGIN");
        assert_eq!(http.security_headers.referrer_policy, "no-referrer");
        assert_eq!(
            http.security_headers.content_security_policy,
            "default-src 'self'"
        );
        assert!(http.trusted_proxy.enabled);
        assert_eq!(
            http.trusted_proxy.trusted_cidrs,
            vec!["10.0.0.0/8", "2001:db8::/32"]
        );
        assert_eq!(http.trusted_proxy.headers, vec!["x-forwarded-for"]);
        assert!(http.cors.enabled);
        assert_eq!(http.cors.allowed_origins, vec!["https://example.com"]);
        assert_eq!(http.cors.allowed_methods, vec!["GET", "POST"]);
        assert_eq!(http.cors.allowed_headers, vec!["authorization"]);
        assert!(http.cors.allow_credentials);
        assert_eq!(http.cors.max_age_seconds, 1_200);
        assert!(http.csrf.enabled);
        assert_eq!(http.csrf.cookie_name, "app_csrf");
        assert_eq!(http.csrf.header_name, "x-app-csrf");
        assert!(!http.csrf.cookie_secure);
        assert_eq!(http.csrf.cookie_path, "/admin");
        assert_eq!(http.csrf.cookie_same_site, "strict");
        assert_eq!(http.csrf.exclude_paths, vec!["/api", "/webhooks"]);
        assert!(http.rate_limit.enabled);
        assert_eq!(http.rate_limit.max_requests, 25);
        assert_eq!(http.rate_limit.window_seconds, 10);
        assert_eq!(http.rate_limit.by, HttpRateLimitByConfig::ActorOrIp);
        assert_eq!(http.rate_limit.key_prefix, "edge:");
    }

    #[test]
    fn parses_auth_config_section() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("AUTH__DEFAULT_GUARD");
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-auth.toml"),
            r#"
                [auth]
                default_guard = "admin"
                bearer_prefix = "Token"

                [auth.tokens]
                access_token_ttl_minutes = 20
                refresh_token_ttl_days = 40
                prune_retention_days = 45
                prune_interval_ms = 120000
                prune_batch_size = 50

                [auth.tokens.guards.admin]
                access_token_ttl_minutes = 43200

                [auth.tokens.guards.user]
                refresh_token_ttl_days = 3

                [auth.sessions]
                ttl_minutes = 90
                cookie_name = "app_session"
                cookie_secure = false
                cookie_path = "/admin"
                cookie_same_site = "strict"
                cookie_domain = "example.com"
                sliding_expiry = false
                remember_ttl_days = 14

                [auth.password_resets]
                expiry_minutes = 30
                prune_interval_ms = 60000
                prune_batch_size = 25

                [auth.email_verification]
                expiry_minutes = 720
                prune_interval_ms = 90000
                prune_batch_size = 30
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let auth: AuthConfig = config.auth().unwrap();

        assert_eq!(auth.default_guard, GuardId::new("admin"));
        assert_eq!(auth.bearer_prefix, "Token");
        assert_eq!(auth.tokens.access_token_ttl_minutes, 20);
        assert_eq!(auth.tokens.refresh_token_ttl_days, 40);
        assert_eq!(auth.tokens.prune_retention_days, 45);
        assert_eq!(auth.tokens.prune_interval_ms, 120_000);
        assert_eq!(auth.tokens.prune_batch_size, 50);
        assert_eq!(
            auth.tokens
                .access_token_ttl_minutes_for_guard(&GuardId::new("admin")),
            43_200
        );
        assert_eq!(
            auth.tokens
                .refresh_token_ttl_days_for_guard(&GuardId::new("admin")),
            40
        );
        assert_eq!(
            auth.tokens
                .access_token_ttl_minutes_for_guard(&GuardId::new("user")),
            20
        );
        assert_eq!(
            auth.tokens
                .refresh_token_ttl_days_for_guard(&GuardId::new("user")),
            3
        );
        assert_eq!(auth.sessions.ttl_minutes, 90);
        assert_eq!(auth.sessions.cookie_name, "app_session");
        assert!(!auth.sessions.cookie_secure);
        assert_eq!(auth.sessions.cookie_path, "/admin");
        assert_eq!(auth.sessions.cookie_same_site, "strict");
        assert_eq!(auth.sessions.cookie_domain, "example.com");
        assert!(!auth.sessions.sliding_expiry);
        assert_eq!(auth.sessions.remember_ttl_days, 14);
        assert_eq!(auth.password_resets.expiry_minutes, 30);
        assert_eq!(auth.password_resets.prune_interval_ms, 60_000);
        assert_eq!(auth.password_resets.prune_batch_size, 25);
        assert_eq!(auth.email_verification.expiry_minutes, 720);
        assert_eq!(auth.email_verification.prune_interval_ms, 90_000);
        assert_eq!(auth.email_verification.prune_batch_size, 30);
    }

    #[test]
    fn auth_config_rejects_weak_token_length() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-auth.toml"),
            r#"
                [auth.tokens]
                token_length = 0
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let error = config.auth().unwrap_err();

        assert!(error
            .to_string()
            .contains("auth.tokens.token_length must be at least 32"));
    }

    #[test]
    fn audit_config_defaults_redact_common_credentials() {
        let audit: AuditConfig = ConfigRepository::empty().audit().unwrap();

        assert!(audit.redact_sensitive_fields);
        assert!(audit.sensitive_fields.contains(&"password".to_string()));
        assert!(audit.sensitive_fields.contains(&"api_key".to_string()));
        assert!(audit
            .sensitive_fields
            .contains(&"refresh_token".to_string()));
    }

    #[test]
    fn parses_audit_config_section() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-audit.toml"),
            r#"
                [audit]
                redact_sensitive_fields = false
                sensitive_fields = ["pin", "card_token"]
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let audit: AuditConfig = config.audit().unwrap();

        assert!(!audit.redact_sensitive_fields);
        assert_eq!(audit.sensitive_fields, vec!["pin", "card_token"]);
    }

    #[test]
    fn parses_cache_config_section() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-cache.toml"),
            r#"
                [cache]
                driver = "memory"
                error_mode = "fail_open"
                prefix = "app-cache:"
                ttl_seconds = 120
                max_entries = 250
                key_max_length = 128
                remember_singleflight = false
                remember_distributed_lock = true
                remember_lock_ttl_ms = 45000
                remember_lock_wait_timeout_ms = 2500
                remember_lock_poll_ms = 50
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let cache = config.cache().unwrap();

        assert_eq!(cache.driver, CacheDriver::Memory);
        assert_eq!(cache.error_mode, CacheErrorMode::FailOpen);
        assert_eq!(cache.prefix, "app-cache:");
        assert_eq!(cache.ttl_seconds, 120);
        assert_eq!(cache.max_entries, 250);
        assert_eq!(cache.key_max_length, 128);
        assert!(!cache.remember_singleflight);
        assert!(cache.remember_distributed_lock);
        assert_eq!(cache.remember_lock_ttl_ms, 45_000);
        assert_eq!(cache.remember_lock_wait_timeout_ms, 2_500);
        assert_eq!(cache.remember_lock_poll_ms, 50);
    }

    #[test]
    fn typescript_config_defaults_to_generated_output_dir() {
        let config = TypeScriptConfig::default();
        assert_eq!(config.output_dir, "frontend/shared/types/generated");
    }

    #[test]
    fn datatable_config_defaults_bound_expensive_outputs() {
        let config: DatatableConfig = ConfigRepository::empty().datatable().unwrap();
        assert_eq!(config.max_per_page, 500);
        assert_eq!(config.max_export_rows, 50_000);
    }

    #[test]
    fn parses_datatable_config_section() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-datatable.toml"),
            r#"
                [datatable]
                max_per_page = 250
                max_export_rows = 10000
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let datatable: DatatableConfig = config.datatable().unwrap();

        assert_eq!(datatable.max_per_page, 250);
        assert_eq!(datatable.max_export_rows, 10_000);
    }

    #[test]
    fn merges_defaults_before_app_config_and_env_overlay() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("SERVER__PORT");
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [server]
                host = "0.0.0.0"
            "#,
        )
        .unwrap();
        std::env::set_var("SERVER__PORT", "4555");

        let config = ConfigRepository::from_dir_with_defaults(
            directory.path(),
            vec![toml::from_str(
                r#"
                    [server]
                    host = "127.0.0.1"
                    port = 3000
                "#,
            )
            .unwrap()],
        )
        .unwrap();
        let server = config.server().unwrap();

        std::env::remove_var("SERVER__PORT");
        assert_eq!(server.host, "0.0.0.0");
        assert_eq!(server.port, 4555);
    }

    #[test]
    fn parses_database_model_defaults() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-database.toml"),
            r#"
                [database.models]
                timestamps_default = false
                soft_deletes_default = true
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let database: DatabaseConfig = config.database().unwrap();

        assert!(!database.models.timestamps_default);
        assert!(database.models.soft_deletes_default);
    }

    #[test]
    fn database_config_defaults_sql_n_plus_one_observability() {
        let database = DatabaseConfig::default();

        assert!(!database.log_query_bindings);
        assert!(database.redact_sql_literals);
        assert_eq!(database.slow_query_retention, 100);
        assert!(database.n_plus_one_detection);
        assert_eq!(database.n_plus_one_min_repeats, 10);
        assert_eq!(database.n_plus_one_retention, 100);
    }

    #[test]
    fn parses_logging_config_section() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-logging.toml"),
            r#"
                [logging]
                level = "debug"
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let logging: LoggingConfig = config.logging().unwrap();

        assert_eq!(logging.level, LogLevel::Debug);
    }

    #[test]
    fn parses_observability_config_section() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-observability.toml"),
            r#"
                [observability]
                enabled = false
                capture_enabled = false
                base_path = "/_ops"
                http_sample_retention = 25
                websocket_channel_retention = 75
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let observability: ObservabilityConfig = config.observability().unwrap();

        assert!(!observability.enabled);
        assert!(!observability.capture_enabled);
        assert_eq!(observability.base_path, "/_ops");
        assert_eq!(observability.http_sample_retention, 25);
        assert_eq!(observability.websocket_channel_retention, 75);
    }

    #[test]
    fn loads_websocket_observability_overrides() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-observability.toml"),
            r#"
                [observability.websocket]
                include_payloads = true
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let observability: ObservabilityConfig = config.observability().unwrap();

        assert!(observability.websocket.include_payloads);
    }

    #[test]
    fn websocket_observability_defaults_to_redacted() {
        let observability = ObservabilityConfig::default();
        assert!(!observability.websocket.include_payloads);
    }

    #[test]
    fn parses_logging_config_with_format_and_log_dir() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-logging.toml"),
            r#"
                [logging]
                level = "debug"
                format = "json"
                log_dir = "var/log"
                file_queue_capacity = 256
                file_max_record_bytes = 8192
                file_flush_timeout_ms = 750
            "#,
        )
        .unwrap();

        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let logging: LoggingConfig = config.logging().unwrap();

        assert_eq!(logging.level, LogLevel::Debug);
        assert_eq!(logging.format, LogFormat::Json);
        assert_eq!(logging.log_dir, "var/log");
        assert_eq!(logging.file_queue_capacity, 256);
        assert_eq!(logging.file_max_record_bytes, 8_192);
        assert_eq!(logging.file_flush_timeout_ms, 750);
    }

    #[test]
    fn logging_config_defaults_to_json_with_logs_dir() {
        let _guard = env_lock().lock().unwrap();
        let directory = tempdir().unwrap();
        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        let logging: LoggingConfig = config.logging().unwrap();

        assert_eq!(logging.level, LogLevel::Info);
        assert_eq!(logging.format, LogFormat::Json);
        assert_eq!(logging.log_dir, "logs");
        assert_eq!(logging.file_queue_capacity, 8_192);
        assert_eq!(logging.file_max_record_bytes, 65_536);
        assert_eq!(logging.file_flush_timeout_ms, 5_000);
    }
}
