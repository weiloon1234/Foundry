use std::fmt::Write;

#[derive(Clone, Copy)]
struct PublishedField {
    key: &'static str,
    toml_value: &'static str,
    env_value: &'static str,
    config_required: bool,
    env_required: bool,
    comment: Option<&'static str>,
}

#[derive(Clone, Copy)]
struct PublishedTable {
    path: &'static [&'static str],
    env_heading: Option<&'static str>,
    commented_header: bool,
    fields: &'static [PublishedField],
}

#[derive(Clone, Copy)]
struct PublishedExample {
    toml_heading: Option<&'static str>,
    env_heading: Option<&'static str>,
    toml_lines: &'static [&'static str],
    env_lines: &'static [&'static str],
}

#[derive(Clone, Copy)]
enum PublishedPart {
    Table(PublishedTable),
    Example(PublishedExample),
}

#[derive(Clone, Copy)]
struct PublishedSection {
    config_file: &'static str,
    title: &'static str,
    parts: &'static [PublishedPart],
}

const fn field(
    key: &'static str,
    toml_value: &'static str,
    env_value: &'static str,
    config_required: bool,
    env_required: bool,
    comment: Option<&'static str>,
) -> PublishedField {
    PublishedField {
        key,
        toml_value,
        env_value,
        config_required,
        env_required,
        comment,
    }
}

const fn table(
    path: &'static [&'static str],
    env_heading: Option<&'static str>,
    commented_header: bool,
    fields: &'static [PublishedField],
) -> PublishedPart {
    PublishedPart::Table(PublishedTable {
        path,
        env_heading,
        commented_header,
        fields,
    })
}

const fn example(
    toml_heading: Option<&'static str>,
    env_heading: Option<&'static str>,
    toml_lines: &'static [&'static str],
    env_lines: &'static [&'static str],
) -> PublishedPart {
    PublishedPart::Example(PublishedExample {
        toml_heading,
        env_heading,
        toml_lines,
        env_lines,
    })
}

const fn section(
    config_file: &'static str,
    title: &'static str,
    parts: &'static [PublishedPart],
) -> PublishedSection {
    PublishedSection {
        config_file,
        title,
        parts,
    }
}

const CLOUDFLARE_TRUSTED_CIDRS_TOML: &str = "[\"173.245.48.0/20\", \"103.21.244.0/22\", \"103.22.200.0/22\", \"103.31.4.0/22\", \"141.101.64.0/18\", \"108.162.192.0/18\", \"190.93.240.0/20\", \"188.114.96.0/20\", \"197.234.240.0/22\", \"198.41.128.0/17\", \"162.158.0.0/15\", \"104.16.0.0/13\", \"104.24.0.0/14\", \"172.64.0.0/13\", \"131.0.72.0/22\", \"2400:cb00::/32\", \"2606:4700::/32\", \"2803:f800::/32\", \"2405:b500::/32\", \"2405:8100::/32\", \"2a06:98c0::/29\", \"2c0f:f248::/32\"]";
const CLOUDFLARE_TRUSTED_CIDRS_ENV: &str = "[\"173.245.48.0/20\",\"103.21.244.0/22\",\"103.22.200.0/22\",\"103.31.4.0/22\",\"141.101.64.0/18\",\"108.162.192.0/18\",\"190.93.240.0/20\",\"188.114.96.0/20\",\"197.234.240.0/22\",\"198.41.128.0/17\",\"162.158.0.0/15\",\"104.16.0.0/13\",\"104.24.0.0/14\",\"172.64.0.0/13\",\"131.0.72.0/22\",\"2400:cb00::/32\",\"2606:4700::/32\",\"2803:f800::/32\",\"2405:b500::/32\",\"2405:8100::/32\",\"2a06:98c0::/29\",\"2c0f:f248::/32\"]";

const APP_FIELDS: &[PublishedField] = &[
    field(
        "name",
        "\"my-app\"",
        "my-app",
        true,
        true,
        Some("App name - used in Redis key prefix for multi-project safety"),
    ),
    field(
        "environment",
        "\"development\"",
        "development",
        true,
        true,
        Some("\"development\", \"production\", \"staging\", \"testing\", or custom label"),
    ),
    field("timezone", "\"UTC\"", "UTC", true, false, None),
    field(
        "signing_key",
        "\"\"",
        "",
        false,
        false,
        Some("Base64 key, at least 32 decoded bytes - generate with `key:generate`"),
    ),
    field(
        "background_shutdown_timeout_ms",
        "30000",
        "30000",
        false,
        false,
        Some("0 = abort managed background tasks immediately"),
    ),
];

const SERVER_FIELDS: &[PublishedField] = &[
    field("host", "\"127.0.0.1\"", "127.0.0.1", true, true, None),
    field("port", "3000", "3000", true, true, None),
];

const HTTP_FIELDS: &[PublishedField] = &[
    field(
        "max_body_size_bytes",
        "0",
        "0",
        false,
        false,
        Some("0 = no global body-size cap; route middleware can still cap"),
    ),
    field(
        "request_timeout_ms",
        "0",
        "0",
        false,
        false,
        Some("0 = no global request timeout"),
    ),
];

const HTTP_SECURITY_HEADERS_FIELDS: &[PublishedField] = &[
    field("enabled", "true", "true", false, false, None),
    field(
        "hsts",
        "false",
        "false",
        false,
        false,
        Some("Enable only after HTTPS is guaranteed"),
    ),
    field("frame_options", "\"DENY\"", "DENY", false, false, None),
    field(
        "referrer_policy",
        "\"strict-origin-when-cross-origin\"",
        "strict-origin-when-cross-origin",
        false,
        false,
        None,
    ),
    field(
        "content_security_policy",
        "\"\"",
        "",
        false,
        false,
        Some("Optional CSP header value"),
    ),
];

const HTTP_TRUSTED_PROXY_FIELDS: &[PublishedField] = &[
    field(
        "enabled",
        "true",
        "true",
        false,
        false,
        Some("Enabled by default so Cloudflare client IP headers work automatically"),
    ),
    field(
        "trusted_cidrs",
        CLOUDFLARE_TRUSTED_CIDRS_TOML,
        CLOUDFLARE_TRUSTED_CIDRS_ENV,
        false,
        false,
        Some("Proxy CIDRs allowed to supply client IP headers; defaults to Cloudflare ranges"),
    ),
    field(
        "headers",
        "[\"cf-connecting-ip\", \"x-real-ip\", \"x-forwarded-for\"]",
        "[\"cf-connecting-ip\",\"x-real-ip\",\"x-forwarded-for\"]",
        false,
        false,
        Some("Checked in order when peer IP is trusted"),
    ),
];

const HTTP_CORS_FIELDS: &[PublishedField] = &[
    field("enabled", "false", "false", false, false, None),
    field(
        "allowed_origins",
        "[]",
        "[]",
        false,
        false,
        Some("Exact origins or [\"*\"]; wildcard cannot be used with credentials"),
    ),
    field(
        "allowed_methods",
        "[\"GET\", \"POST\", \"PUT\", \"PATCH\", \"DELETE\", \"OPTIONS\"]",
        "[\"GET\",\"POST\",\"PUT\",\"PATCH\",\"DELETE\",\"OPTIONS\"]",
        false,
        false,
        None,
    ),
    field(
        "allowed_headers",
        "[\"authorization\", \"content-type\", \"x-request-id\", \"x-csrf-token\"]",
        "[\"authorization\",\"content-type\",\"x-request-id\",\"x-csrf-token\"]",
        false,
        false,
        None,
    ),
    field("allow_credentials", "false", "false", false, false, None),
    field("max_age_seconds", "600", "600", false, false, None),
];

const HTTP_CSRF_FIELDS: &[PublishedField] = &[
    field("enabled", "false", "false", false, false, None),
    field(
        "cookie_name",
        "\"foundry_csrf\"",
        "foundry_csrf",
        false,
        false,
        None,
    ),
    field(
        "header_name",
        "\"x-csrf-token\"",
        "x-csrf-token",
        false,
        false,
        None,
    ),
    field("cookie_secure", "true", "true", false, false, None),
    field("cookie_path", "\"/\"", "/", false, false, None),
    field(
        "cookie_same_site",
        "\"lax\"",
        "lax",
        false,
        false,
        Some("\"lax\", \"strict\", or \"none\"; none requires secure cookies"),
    ),
    field(
        "exclude_paths",
        "[]",
        "[]",
        false,
        false,
        Some("Segment-aware prefixes excluded from CSRF checks"),
    ),
];

const HTTP_RATE_LIMIT_FIELDS: &[PublishedField] = &[
    field(
        "enabled",
        "true",
        "true",
        false,
        false,
        Some("Enabled by default with actor-or-IP keys"),
    ),
    field("max_requests", "600", "600", false, false, None),
    field("window_seconds", "60", "60", false, false, None),
    field(
        "by",
        "\"actor_or_ip\"",
        "actor_or_ip",
        false,
        false,
        Some("\"ip\", \"actor\", or \"actor_or_ip\""),
    ),
    field("key_prefix", "\"http:\"", "http:", false, false, None),
];

const REDIS_FIELDS: &[PublishedField] = &[
    field(
        "url",
        "\"redis://127.0.0.1/\"",
        "redis://127.0.0.1/",
        true,
        true,
        None,
    ),
    field(
        "namespace",
        "\"foundry\"",
        "foundry",
        false,
        false,
        Some("Key prefix - auto-derived from app.name:app.environment if not set"),
    ),
];

const DATABASE_FIELDS: &[PublishedField] = &[
    field(
        "url",
        "\"postgres://foundry:secret@127.0.0.1:5432/foundry\"",
        "postgres://foundry:secret@127.0.0.1:5432/foundry",
        true,
        true,
        None,
    ),
    field(
        "read_url",
        "\"\"",
        "",
        false,
        false,
        Some("Read replica URL (auto-routes reads when set)"),
    ),
    field("schema", "\"public\"", "public", false, false, None),
    field(
        "migration_table",
        "\"foundry_migrations\"",
        "foundry_migrations",
        false,
        false,
        None,
    ),
    field(
        "migration_lock_timeout_ms",
        "0",
        "0",
        false,
        false,
        Some("Migration advisory-lock wait timeout (0 = wait forever)"),
    ),
    field(
        "migrations_path",
        "\"database/migrations\"",
        "database/migrations",
        false,
        false,
        None,
    ),
    field(
        "seeders_path",
        "\"database/seeders\"",
        "database/seeders",
        false,
        false,
        None,
    ),
    field("min_connections", "1", "1", false, false, None),
    field("max_connections", "10", "10", false, false, None),
    field("acquire_timeout_ms", "5000", "5000", false, false, None),
    field(
        "default_per_page",
        "15",
        "15",
        false,
        false,
        Some("Default pagination page size"),
    ),
    field(
        "log_queries",
        "false",
        "false",
        false,
        false,
        Some("Log all SQL queries to tracing (dev only)"),
    ),
    field(
        "log_query_bindings",
        "false",
        "false",
        false,
        false,
        Some("Include SQL binding values when log_queries=true (dev only)"),
    ),
    field(
        "redact_sql_literals",
        "true",
        "true",
        false,
        false,
        Some("Redact SQL literals/comments in logs and /_foundry/sql"),
    ),
    field(
        "slow_query_threshold_ms",
        "500",
        "500",
        false,
        false,
        Some("Log queries exceeding this threshold"),
    ),
    field(
        "slow_query_retention",
        "100",
        "100",
        false,
        false,
        Some("Retained slow queries for /_foundry/sql (0 = disable retention)"),
    ),
    field(
        "n_plus_one_detection",
        "true",
        "true",
        false,
        false,
        Some("Detect repeated query fingerprints during HTTP requests"),
    ),
    field(
        "n_plus_one_min_repeats",
        "10",
        "10",
        false,
        false,
        Some("Minimum repeated query count before retaining an N+1 suspect"),
    ),
    field(
        "n_plus_one_retention",
        "100",
        "100",
        false,
        false,
        Some("Maximum retained HTTP N+1 suspect entries"),
    ),
    field(
        "idle_timeout_seconds",
        "600",
        "600",
        false,
        false,
        Some("Close idle connections after 10 min"),
    ),
    field(
        "max_lifetime_seconds",
        "1800",
        "1800",
        false,
        false,
        Some("Recycle connections after 30 min"),
    ),
    field(
        "connect_lazy",
        "false",
        "false",
        false,
        false,
        Some("Create pools without opening a DB socket until first use"),
    ),
];

const DATABASE_POOL_FIELDS: &[PublishedField] = &[
    field(
        "min_connections",
        "0",
        "0",
        false,
        false,
        Some("Override database.min_connections for this pool"),
    ),
    field(
        "max_connections",
        "5",
        "5",
        false,
        false,
        Some("Override database.max_connections for this pool"),
    ),
    field(
        "acquire_timeout_ms",
        "5000",
        "5000",
        false,
        false,
        Some("Override database.acquire_timeout_ms for this pool"),
    ),
    field(
        "idle_timeout_seconds",
        "60",
        "60",
        false,
        false,
        Some("Override database.idle_timeout_seconds for this pool"),
    ),
    field(
        "max_lifetime_seconds",
        "300",
        "300",
        false,
        false,
        Some("Override database.max_lifetime_seconds for this pool"),
    ),
    field(
        "connect_lazy",
        "true",
        "true",
        false,
        false,
        Some("Override database.connect_lazy for this pool"),
    ),
];

const DATABASE_MODEL_FIELDS: &[PublishedField] = &[
    field(
        "timestamps_default",
        "true",
        "true",
        false,
        false,
        Some("Auto-add created_at/updated_at"),
    ),
    field(
        "soft_deletes_default",
        "false",
        "false",
        false,
        false,
        Some("Auto-add deleted_at"),
    ),
];

const AUTH_FIELDS: &[PublishedField] = &[
    field("default_guard", "\"api\"", "api", false, false, None),
    field("bearer_prefix", "\"Bearer\"", "Bearer", false, false, None),
];

const AUTH_TOKEN_FIELDS: &[PublishedField] = &[
    field("access_token_ttl_minutes", "15", "15", false, false, None),
    field("refresh_token_ttl_days", "30", "30", false, false, None),
    field("token_length", "32", "32", false, false, None),
    field("rotate_refresh_tokens", "true", "true", false, false, None),
    field(
        "prune_retention_days",
        "30",
        "30",
        false,
        false,
        Some("Auto-prune expired/revoked tokens older than N days (0 = app-owned/manual)"),
    ),
    field(
        "prune_interval_ms",
        "3600000",
        "3600000",
        false,
        false,
        Some("How often workers attempt token pruning"),
    ),
    field(
        "prune_batch_size",
        "1000",
        "1000",
        false,
        false,
        Some("Max tokens deleted per prune pass"),
    ),
];

const AUTH_SESSION_FIELDS: &[PublishedField] = &[
    field("ttl_minutes", "120", "120", false, false, None),
    field(
        "cookie_name",
        "\"foundry_session\"",
        "foundry_session",
        false,
        false,
        None,
    ),
    field("cookie_secure", "true", "true", false, false, None),
    field("cookie_path", "\"/\"", "/", false, false, None),
    field(
        "cookie_same_site",
        "\"lax\"",
        "lax",
        false,
        false,
        Some("\"lax\", \"strict\", or \"none\"; none requires secure cookies"),
    ),
    field(
        "cookie_domain",
        "\"\"",
        "",
        false,
        false,
        Some("Optional Set-Cookie Domain; empty omits Domain"),
    ),
    field("sliding_expiry", "true", "true", false, false, None),
    field("remember_ttl_days", "30", "30", false, false, None),
];

const AUTH_PASSWORD_RESET_FIELDS: &[PublishedField] = &[
    field(
        "expiry_minutes",
        "60",
        "60",
        false,
        false,
        Some("Password reset token lifetime (0 = no expiry/auto-prune)"),
    ),
    field(
        "prune_interval_ms",
        "3600000",
        "3600000",
        false,
        false,
        Some("How often workers attempt reset-token pruning"),
    ),
    field(
        "prune_batch_size",
        "1000",
        "1000",
        false,
        false,
        Some("Max reset tokens deleted per prune pass"),
    ),
];

const AUTH_EMAIL_VERIFICATION_FIELDS: &[PublishedField] = &[
    field(
        "expiry_minutes",
        "1440",
        "1440",
        false,
        false,
        Some("Email verification token lifetime (0 = no expiry/auto-prune)"),
    ),
    field(
        "prune_interval_ms",
        "3600000",
        "3600000",
        false,
        false,
        Some("How often workers attempt verification-token pruning"),
    ),
    field(
        "prune_batch_size",
        "1000",
        "1000",
        false,
        false,
        Some("Max verification tokens deleted per prune pass"),
    ),
];

const AUTH_LOCKOUT_FIELDS: &[PublishedField] = &[
    field("enabled", "true", "true", false, false, None),
    field("max_failures", "5", "5", false, false, None),
    field("lockout_minutes", "15", "15", false, false, None),
    field("window_minutes", "15", "15", false, false, None),
];

const AUTH_MFA_FIELDS: &[PublishedField] = &[
    field("enabled", "true", "true", false, false, None),
    field("issuer", "\"foundry\"", "foundry", false, false, None),
    field("pending_token_ttl_minutes", "10", "10", false, false, None),
    field("recovery_codes", "8", "8", false, false, None),
];

const AUDIT_FIELDS: &[PublishedField] = &[
    field(
        "redact_sensitive_fields",
        "true",
        "true",
        false,
        false,
        Some("Redact common credential-like model columns in audit JSON"),
    ),
    field(
        "sensitive_fields",
        "[\"password\", \"password_hash\", \"passwd\", \"secret\", \"secret_key\", \"api_key\", \"access_key\", \"private_key\", \"token\", \"token_hash\", \"access_token\", \"refresh_token\", \"authorization\", \"credential\", \"credentials\", \"mfa_secret\", \"totp_secret\", \"otp_secret\", \"recovery_code\", \"recovery_codes\"]",
        "[\"password\",\"password_hash\",\"passwd\",\"secret\",\"secret_key\",\"api_key\",\"access_key\",\"private_key\",\"token\",\"token_hash\",\"access_token\",\"refresh_token\",\"authorization\",\"credential\",\"credentials\",\"mfa_secret\",\"totp_secret\",\"otp_secret\",\"recovery_code\",\"recovery_codes\"]",
        false,
        false,
        Some("Exact/normalized field names; #[foundry(audit_exclude)] still removes fields entirely"),
    ),
];

const RUNTIME_FIELDS: &[PublishedField] = &[
    field(
        "worker_threads",
        "0",
        "0",
        false,
        false,
        Some("Tokio worker thread count for sync runners (0 = Tokio default)"),
    ),
    field(
        "max_blocking_threads",
        "0",
        "0",
        false,
        false,
        Some("Tokio blocking thread cap for sync runners (0 = Tokio default)"),
    ),
];

const JOBS_FIELDS: &[PublishedField] = &[
    field("queue", "\"default\"", "default", false, false, None),
    field("max_retries", "5", "5", false, false, None),
    field("poll_interval_ms", "100", "100", false, false, None),
    field("lease_ttl_ms", "30000", "30000", false, false, None),
    field("requeue_batch_size", "64", "64", false, false, None),
    field(
        "max_concurrent_jobs",
        "16",
        "16",
        false,
        false,
        Some("Worker concurrency cap (0 = explicit unlimited)"),
    ),
    field("timeout_seconds", "300", "300", false, false, None),
    field("shutdown_timeout_ms", "30000", "30000", false, false, None),
    field("track_history", "true", "true", false, false, None),
    field(
        "history_retention_days",
        "30",
        "30",
        false,
        false,
        Some("Auto-prune job_history older than N days (0 = keep forever)"),
    ),
    field(
        "history_prune_interval_ms",
        "3600000",
        "3600000",
        false,
        false,
        Some("How often workers attempt job_history pruning"),
    ),
    field(
        "history_prune_batch_size",
        "1000",
        "1000",
        false,
        false,
        Some("Maximum job_history rows deleted per prune pass"),
    ),
];

const SCHEDULER_FIELDS: &[PublishedField] = &[
    field("tick_interval_ms", "1000", "1000", false, false, None),
    field("leader_lease_ttl_ms", "5000", "5000", false, false, None),
    field(
        "shutdown_timeout_ms",
        "30000",
        "30000",
        false,
        false,
        Some("0 = do not wait"),
    ),
];

const WEBSOCKET_FIELDS: &[PublishedField] = &[
    field("host", "\"127.0.0.1\"", "127.0.0.1", false, false, None),
    field("port", "3010", "3010", false, false, None),
    field("path", "\"/ws\"", "/ws", false, false, None),
    field("heartbeat_interval_seconds", "30", "30", false, false, None),
    field("heartbeat_timeout_seconds", "10", "10", false, false, None),
    field(
        "auth_revalidation_interval_seconds",
        "30",
        "30",
        false,
        false,
        Some("Maximum cached WebSocket credential age; minimum effective value is 1 second"),
    ),
    field(
        "max_message_size_bytes",
        "1048576",
        "1048576",
        false,
        false,
        Some("0 = use transport default"),
    ),
    field(
        "max_frame_size_bytes",
        "1048576",
        "1048576",
        false,
        false,
        Some("0 = use transport default"),
    ),
    field(
        "max_write_buffer_size_bytes",
        "1048576",
        "1048576",
        false,
        false,
        Some("0 = use transport default"),
    ),
    field("max_messages_per_second", "50", "50", false, false, None),
    field(
        "max_connections_global",
        "10000",
        "10000",
        false,
        false,
        Some("0 = unlimited process-wide WebSocket connections"),
    ),
    field(
        "max_connections_per_ip",
        "100",
        "100",
        false,
        false,
        Some("0 = unlimited anonymous connections per resolved client IP"),
    ),
    field("max_connections_per_user", "5", "5", false, false, None),
    field(
        "max_subscriptions_per_connection",
        "100",
        "100",
        false,
        false,
        Some("0 = unlimited active subscriptions per connection"),
    ),
    field("max_channel_length", "128", "128", false, false, None),
    field("max_room_length", "256", "256", false, false, None),
    field("max_event_length", "128", "128", false, false, None),
    field("max_ack_id_length", "128", "128", false, false, None),
    field(
        "outbound_buffer_size",
        "1024",
        "1024",
        false,
        false,
        Some("Queued outbound frames per connection before disconnect"),
    ),
    field(
        "query_token_enabled",
        "true",
        "true",
        false,
        false,
        Some("Allow browser clients to pass short-lived bearer auth as a query token"),
    ),
    field(
        "query_token_name",
        "\"token\"",
        "token",
        false,
        false,
        Some("Query parameter name used when query token auth is enabled"),
    ),
    field(
        "query_token_max_length",
        "4096",
        "4096",
        false,
        false,
        Some("Max decoded query-token bytes (0 = unlimited)"),
    ),
    field(
        "allowed_origins",
        "[]",
        "",
        false,
        false,
        Some("Exact Origin allow-list; empty allows same-origin in production/staging"),
    ),
    field(
        "history_buffer_size",
        "50",
        "50",
        false,
        false,
        Some("Recent messages retained per channel"),
    ),
    field(
        "history_ttl_seconds",
        "604800",
        "604800",
        false,
        false,
        Some("Set to 0 to disable history auto-reap"),
    ),
];

const LOGGING_FIELDS: &[PublishedField] = &[
    field(
        "level",
        "\"info\"",
        "info",
        false,
        false,
        Some("trace, debug, info, warn, error"),
    ),
    field(
        "format",
        "\"json\"",
        "json",
        false,
        false,
        Some("\"json\" or \"text\""),
    ),
    field("log_dir", "\"logs\"", "logs", false, false, None),
    field(
        "retention_days",
        "30",
        "30",
        false,
        false,
        Some("Auto-delete logs older than N days (0 = keep forever)"),
    ),
];

const OBSERVABILITY_FIELDS: &[PublishedField] = &[
    field(
        "enabled",
        "true",
        "true",
        false,
        false,
        Some("Register /_foundry observability routes"),
    ),
    field(
        "capture_enabled",
        "true",
        "true",
        false,
        false,
        Some("Record passive runtime observability data"),
    ),
    field(
        "base_path",
        "\"/_foundry\"",
        "/_foundry",
        false,
        false,
        Some("Dashboard route prefix"),
    ),
    field(
        "http_sample_retention",
        "500",
        "500",
        false,
        false,
        Some("Retained HTTP request samples for rankings (0 = disable samples)"),
    ),
    field(
        "websocket_channel_retention",
        "500",
        "500",
        false,
        false,
        Some("Retained idle WebSocket channel counters (0 = disable per-channel retention)"),
    ),
    field(
        "tracing_enabled",
        "false",
        "false",
        false,
        false,
        Some("Enable OpenTelemetry distributed tracing"),
    ),
    field(
        "otlp_endpoint",
        "\"http://localhost:4317\"",
        "http://localhost:4317",
        false,
        false,
        None,
    ),
    field("service_name", "\"foundry\"", "foundry", false, false, None),
];

const OBSERVABILITY_WEBSOCKET_FIELDS: &[PublishedField] = &[field(
    "include_payloads",
    "false",
    "false",
    false,
    false,
    Some("Include full payloads in /_foundry/ws/history/:channel"),
)];

const CACHE_FIELDS: &[PublishedField] = &[
    field(
        "driver",
        "\"redis\"",
        "redis",
        false,
        false,
        Some("\"redis\" or \"memory\""),
    ),
    field(
        "error_mode",
        "\"strict\"",
        "strict",
        false,
        false,
        Some("\"strict\" or \"fail_open\"; strict surfaces backend failures"),
    ),
    field("prefix", "\"cache:\"", "cache:", false, false, None),
    field("ttl_seconds", "3600", "3600", false, false, None),
    field("max_entries", "10000", "10000", false, false, None),
    field(
        "key_max_length",
        "512",
        "512",
        false,
        false,
        Some("0 = disable cache key length cap"),
    ),
    field(
        "remember_singleflight",
        "true",
        "true",
        false,
        false,
        Some("Coalesce concurrent remember() calls inside one process"),
    ),
    field(
        "remember_distributed_lock",
        "false",
        "false",
        false,
        false,
        Some("Opt-in cross-worker remember() stampede protection"),
    ),
    field("remember_lock_ttl_ms", "30000", "30000", false, false, None),
    field(
        "remember_lock_wait_timeout_ms",
        "5000",
        "5000",
        false,
        false,
        None,
    ),
    field("remember_lock_poll_ms", "100", "100", false, false, None),
];

const HASHING_FIELDS: &[PublishedField] = &[
    field("driver", "\"argon2\"", "argon2", false, false, None),
    field("memory_cost", "19456", "19456", false, false, None),
    field("time_cost", "2", "2", false, false, None),
    field("parallelism", "1", "1", false, false, None),
];

const CRYPT_FIELDS: &[PublishedField] = &[field(
    "key",
    "\"\"",
    "",
    false,
    false,
    Some("Base64 key - generate with `key:generate`"),
)];

const I18N_FIELDS: &[PublishedField] = &[
    field("default_locale", "\"en\"", "en", false, false, None),
    field("fallback_locale", "\"en\"", "en", false, false, None),
    field(
        "resource_path",
        "\"locales\"",
        "locales",
        false,
        false,
        None,
    ),
];

const TYPESCRIPT_FIELDS: &[PublishedField] = &[field(
    "output_dir",
    "\"frontend/shared/types/generated\"",
    "frontend/shared/types/generated",
    false,
    false,
    None,
)];

const DATATABLE_FIELDS: &[PublishedField] = &[
    field(
        "max_per_page",
        "500",
        "500",
        false,
        false,
        Some("Max JSON rows per page (0 = unlimited)"),
    ),
    field(
        "max_export_rows",
        "50000",
        "50000",
        false,
        false,
        Some("Max rows generated into XLSX downloads/jobs (0 = unlimited)"),
    ),
];

const EMAIL_FIELDS: &[PublishedField] = &[
    field(
        "default",
        "\"smtp\"",
        "smtp",
        false,
        false,
        Some("Default mailer name"),
    ),
    field(
        "queue",
        "\"default\"",
        "default",
        false,
        false,
        Some("Queue for async email dispatch"),
    ),
    field(
        "template_path",
        "\"templates/emails\"",
        "templates/emails",
        false,
        false,
        None,
    ),
    field(
        "max_attachment_bytes",
        "26214400",
        "26214400",
        false,
        false,
        Some("Max bytes for one resolved email attachment (0 = unlimited)"),
    ),
    field(
        "max_total_attachment_bytes",
        "26214400",
        "26214400",
        false,
        false,
        Some("Max combined bytes for resolved email attachments (0 = unlimited)"),
    ),
];

const EMAIL_FROM_FIELDS: &[PublishedField] = &[
    field("address", "\"\"", "", false, false, None),
    field("name", "\"\"", "", false, false, None),
];

const STORAGE_FIELDS: &[PublishedField] = &[
    field("default", "\"local\"", "local", false, false, None),
    field(
        "max_upload_size_bytes",
        "104857600",
        "104857600",
        false,
        false,
        Some("Total multipart file bytes per request (0 = no storage-level cap)"),
    ),
    field(
        "max_upload_file_size_bytes",
        "52428800",
        "52428800",
        false,
        false,
        Some("Per-file multipart upload cap (0 = no storage-level cap)"),
    ),
    field(
        "max_upload_files",
        "20",
        "20",
        false,
        false,
        Some("Max uploaded files per multipart request (0 = no storage-level cap)"),
    ),
    field(
        "upload_temp_retention_seconds",
        "3600",
        "3600",
        false,
        false,
        Some("Worker cleanup age for foundry-upload-* temp files (0 = keep forever)"),
    ),
    field(
        "upload_temp_prune_interval_ms",
        "3600000",
        "3600000",
        false,
        false,
        Some("How often workers attempt upload temp cleanup"),
    ),
    field(
        "upload_temp_prune_batch_size",
        "1000",
        "1000",
        false,
        false,
        Some("Max upload temp files deleted per prune pass"),
    ),
    field(
        "image_max_input_bytes",
        "52428800",
        "52428800",
        false,
        false,
        Some("Max image bytes decoded by attachment processing (0 = disabled)"),
    ),
    field(
        "image_max_pixels",
        "50000000",
        "50000000",
        false,
        false,
        Some("Max decoded image pixels for attachment processing (0 = disabled)"),
    ),
    field(
        "image_max_width",
        "12000",
        "12000",
        false,
        false,
        Some("Max decoded image width for attachment processing (0 = disabled)"),
    ),
    field(
        "image_max_height",
        "12000",
        "12000",
        false,
        false,
        Some("Max decoded image height for attachment processing (0 = disabled)"),
    ),
    field(
        "attachment_orphan_audit_enabled",
        "true",
        "true",
        false,
        false,
        Some("Audit old storage objects under attachment_orphan_prefix during worker maintenance"),
    ),
    field(
        "attachment_orphan_delete_enabled",
        "false",
        "false",
        false,
        false,
        Some("Allow Foundry to delete audited attachment orphans (off by default)"),
    ),
    field(
        "attachment_orphan_retention_seconds",
        "604800",
        "604800",
        false,
        false,
        Some("Only audit/delete attachment orphans older than this age"),
    ),
    field(
        "attachment_orphan_prune_interval_ms",
        "3600000",
        "3600000",
        false,
        false,
        Some("How often workers attempt attachment orphan audit/delete"),
    ),
    field(
        "attachment_orphan_prune_batch_size",
        "100",
        "100",
        false,
        false,
        Some("Max listed attachment objects checked per maintenance pass"),
    ),
    field(
        "attachment_orphan_prefix",
        "\"attachments/\"",
        "attachments/",
        false,
        false,
        Some("Storage prefix owned by Foundry attachments for orphan audit/delete"),
    ),
];

const PUBLISHED_SECTIONS: &[PublishedSection] = &[
    section(
        "00-app.toml",
        "Application",
        &[table(&["app"], None, false, APP_FIELDS)],
    ),
    section(
        "00-app.toml",
        "HTTP Server",
        &[table(&["server"], None, false, SERVER_FIELDS)],
    ),
    section(
        "10-http.toml",
        "HTTP Edge",
        &[
            table(&["http"], None, false, HTTP_FIELDS),
            table(
                &["http", "security_headers"],
                Some("HTTP Security Headers"),
                false,
                HTTP_SECURITY_HEADERS_FIELDS,
            ),
            table(
                &["http", "trusted_proxy"],
                Some("HTTP Trusted Proxy"),
                false,
                HTTP_TRUSTED_PROXY_FIELDS,
            ),
            table(
                &["http", "cors"],
                Some("HTTP CORS"),
                false,
                HTTP_CORS_FIELDS,
            ),
            table(
                &["http", "csrf"],
                Some("HTTP CSRF"),
                false,
                HTTP_CSRF_FIELDS,
            ),
            table(
                &["http", "rate_limit"],
                Some("HTTP Rate Limit"),
                false,
                HTTP_RATE_LIMIT_FIELDS,
            ),
        ],
    ),
    section(
        "00-app.toml",
        "Redis",
        &[table(&["redis"], None, false, REDIS_FIELDS)],
    ),
    section(
        "20-database.toml",
        "Database (PostgreSQL)",
        &[
            table(&["database"], None, false, DATABASE_FIELDS),
            table(
                &["database", "write_pool"],
                Some("Database Write Pool Overrides"),
                true,
                DATABASE_POOL_FIELDS,
            ),
            table(
                &["database", "read_pool"],
                Some("Database Read Pool Overrides"),
                true,
                DATABASE_POOL_FIELDS,
            ),
            table(
                &["database", "models"],
                Some("Database Model Defaults"),
                false,
                DATABASE_MODEL_FIELDS,
            ),
        ],
    ),
    section(
        "30-security.toml",
        "Authentication",
        &[
            table(&["auth"], None, false, AUTH_FIELDS),
            table(
                &["auth", "tokens"],
                Some("Token Settings"),
                false,
                AUTH_TOKEN_FIELDS,
            ),
            example(
                Some("Optional per-guard token TTL overrides:"),
                Some("Optional per-guard token TTL overrides:"),
                &[
                    "# [auth.tokens.guards.admin]",
                    "# access_token_ttl_minutes = 43200",
                    "# refresh_token_ttl_days = 30",
                    "#",
                    "# [auth.tokens.guards.user]",
                    "# access_token_ttl_minutes = 4320",
                    "# refresh_token_ttl_days = 3",
                ],
                &[
                    "# AUTH__TOKENS__GUARDS__ADMIN__ACCESS_TOKEN_TTL_MINUTES=43200",
                    "# AUTH__TOKENS__GUARDS__ADMIN__REFRESH_TOKEN_TTL_DAYS=30",
                    "# AUTH__TOKENS__GUARDS__USER__ACCESS_TOKEN_TTL_MINUTES=4320",
                    "# AUTH__TOKENS__GUARDS__USER__REFRESH_TOKEN_TTL_DAYS=3",
                ],
            ),
            table(
                &["auth", "sessions"],
                Some("Session Settings"),
                false,
                AUTH_SESSION_FIELDS,
            ),
            table(
                &["auth", "password_resets"],
                Some("Password Reset Tokens"),
                false,
                AUTH_PASSWORD_RESET_FIELDS,
            ),
            table(
                &["auth", "email_verification"],
                Some("Email Verification Tokens"),
                false,
                AUTH_EMAIL_VERIFICATION_FIELDS,
            ),
            table(
                &["auth", "lockout"],
                Some("Login Lockout"),
                false,
                AUTH_LOCKOUT_FIELDS,
            ),
            table(
                &["auth", "mfa"],
                Some("Multi-Factor Authentication"),
                false,
                AUTH_MFA_FIELDS,
            ),
            example(
                Some("Required roles per guard:"),
                Some("Required roles per guard (TOML/JSON-style arrays are supported):"),
                &[
                    "# [auth.mfa.required_roles]",
                    "# admin = [\"developer\", \"super_admin\"]",
                ],
                &["# AUTH__MFA__REQUIRED_ROLES__ADMIN=[\"developer\",\"super_admin\"]"],
            ),
            example(
                Some("Define guards (one per authentication portal):"),
                Some("Guard Drivers (per guard name)"),
                &[
                    "# [auth.guards.api]",
                    "# driver = \"token\"  # \"token\", \"session\", or \"custom\"",
                    "#",
                    "# [auth.guards.web]",
                    "# driver = \"session\"",
                ],
                &[
                    "# AUTH__GUARDS__API__DRIVER=token",
                    "# AUTH__GUARDS__WEB__DRIVER=session",
                ],
            ),
        ],
    ),
    section(
        "30-security.toml",
        "Audit",
        &[table(&["audit"], None, false, AUDIT_FIELDS)],
    ),
    section(
        "40-runtime.toml",
        "Runtime",
        &[table(&["runtime"], None, false, RUNTIME_FIELDS)],
    ),
    section(
        "40-runtime.toml",
        "Jobs (Background Queue)",
        &[
            table(&["jobs"], None, false, JOBS_FIELDS),
            example(
                Some("Queue priorities (lower number = claimed first):"),
                Some("Queue priorities (lower number = claimed first):"),
                &[
                    "# [jobs.queue_priorities]",
                    "# high = 1",
                    "# default = 5",
                    "# low = 10",
                ],
                &[
                    "# JOBS__QUEUE_PRIORITIES__HIGH=1",
                    "# JOBS__QUEUE_PRIORITIES__DEFAULT=5",
                    "# JOBS__QUEUE_PRIORITIES__LOW=10",
                ],
            ),
        ],
    ),
    section(
        "40-runtime.toml",
        "Scheduler (Cron)",
        &[table(&["scheduler"], None, false, SCHEDULER_FIELDS)],
    ),
    section(
        "40-runtime.toml",
        "WebSocket",
        &[table(&["websocket"], None, false, WEBSOCKET_FIELDS)],
    ),
    section(
        "40-runtime.toml",
        "Logging",
        &[table(&["logging"], None, false, LOGGING_FIELDS)],
    ),
    section(
        "40-runtime.toml",
        "Observability (Dashboard & Tracing)",
        &[
            table(
                &["observability"],
                None,
                false,
                OBSERVABILITY_FIELDS,
            ),
            table(
                &["observability", "websocket"],
                None,
                true,
                OBSERVABILITY_WEBSOCKET_FIELDS,
            ),
        ],
    ),
    section(
        "40-runtime.toml",
        "Cache",
        &[table(&["cache"], None, false, CACHE_FIELDS)],
    ),
    section(
        "30-security.toml",
        "Hashing (Password)",
        &[table(&["hashing"], None, false, HASHING_FIELDS)],
    ),
    section(
        "30-security.toml",
        "Encryption",
        &[table(&["crypt"], None, false, CRYPT_FIELDS)],
    ),
    section(
        "50-localization.toml",
        "Internationalization",
        &[table(&["i18n"], None, false, I18N_FIELDS)],
    ),
    section(
        "50-localization.toml",
        "TypeScript",
        &[table(&["typescript"], None, false, TYPESCRIPT_FIELDS)],
    ),
    section(
        "20-database.toml",
        "Datatable",
        &[table(&["datatable"], None, false, DATATABLE_FIELDS)],
    ),
    section(
        "60-email.toml",
        "Email",
        &[
            table(&["email"], None, false, EMAIL_FIELDS),
            table(&["email", "from"], Some("From Address"), false, EMAIL_FROM_FIELDS),
            example(
                Some("SMTP mailer:"),
                Some("SMTP Mailer"),
                &[
                    "# [email.mailers.smtp]",
                    "# driver = \"smtp\"",
                    "# host = \"smtp.example.com\"",
                    "# port = 587",
                    "# username = \"\"",
                    "# password = \"\"",
                    "# encryption = \"starttls\"  # \"starttls\", \"tls\", or \"none\"",
                    "# timeout_secs = 30",
                ],
                &[
                    "# EMAIL__MAILERS__SMTP__DRIVER=smtp",
                    "# EMAIL__MAILERS__SMTP__HOST=smtp.example.com",
                    "# EMAIL__MAILERS__SMTP__PORT=587",
                    "# EMAIL__MAILERS__SMTP__USERNAME=",
                    "# EMAIL__MAILERS__SMTP__PASSWORD=",
                    "# EMAIL__MAILERS__SMTP__ENCRYPTION=starttls  # \"starttls\", \"tls\", or \"none\"",
                    "# EMAIL__MAILERS__SMTP__TIMEOUT_SECS=30",
                ],
            ),
            example(
                Some("Amazon SES mailer:"),
                Some("Amazon SES Mailer"),
                &[
                    "# [email.mailers.ses]",
                    "# driver = \"ses\"",
                    "# key = \"\"",
                    "# secret = \"\"",
                    "# region = \"us-east-1\"",
                    "# timeout_secs = 30",
                ],
                &[
                    "# EMAIL__MAILERS__SES__DRIVER=ses",
                    "# EMAIL__MAILERS__SES__KEY=",
                    "# EMAIL__MAILERS__SES__SECRET=",
                    "# EMAIL__MAILERS__SES__REGION=us-east-1",
                    "# EMAIL__MAILERS__SES__TIMEOUT_SECS=30",
                ],
            ),
            example(
                Some("Postmark mailer:"),
                Some("Postmark Mailer"),
                &[
                    "# [email.mailers.postmark]",
                    "# driver = \"postmark\"",
                    "# server_token = \"\"",
                    "# timeout_secs = 30",
                ],
                &[
                    "# EMAIL__MAILERS__POSTMARK__DRIVER=postmark",
                    "# EMAIL__MAILERS__POSTMARK__SERVER_TOKEN=",
                    "# EMAIL__MAILERS__POSTMARK__TIMEOUT_SECS=30",
                ],
            ),
            example(
                Some("Resend mailer:"),
                Some("Resend Mailer"),
                &[
                    "# [email.mailers.resend]",
                    "# driver = \"resend\"",
                    "# api_key = \"\"",
                    "# timeout_secs = 30",
                ],
                &[
                    "# EMAIL__MAILERS__RESEND__DRIVER=resend",
                    "# EMAIL__MAILERS__RESEND__API_KEY=",
                    "# EMAIL__MAILERS__RESEND__TIMEOUT_SECS=30",
                ],
            ),
            example(
                Some("Mailgun mailer:"),
                Some("Mailgun Mailer"),
                &[
                    "# [email.mailers.mailgun]",
                    "# driver = \"mailgun\"",
                    "# domain = \"\"",
                    "# api_key = \"\"",
                    "# region = \"us\"  # \"us\" or \"eu\"",
                    "# timeout_secs = 30",
                ],
                &[
                    "# EMAIL__MAILERS__MAILGUN__DRIVER=mailgun",
                    "# EMAIL__MAILERS__MAILGUN__DOMAIN=",
                    "# EMAIL__MAILERS__MAILGUN__API_KEY=",
                    "# EMAIL__MAILERS__MAILGUN__REGION=us  # \"us\" or \"eu\"",
                    "# EMAIL__MAILERS__MAILGUN__TIMEOUT_SECS=30",
                ],
            ),
            example(
                Some("Log mailer (development - logs instead of sending):"),
                Some("Log Mailer (development - logs instead of sending)"),
                &[
                    "# [email.mailers.log]",
                    "# driver = \"log\"",
                    "# target = \"email.outbound\"",
                ],
                &[
                    "# EMAIL__MAILERS__LOG__DRIVER=log",
                    "# EMAIL__MAILERS__LOG__TARGET=email.outbound",
                ],
            ),
        ],
    ),
    section(
        "70-storage.toml",
        "Storage (File System)",
        &[
            table(&["storage"], None, false, STORAGE_FIELDS),
            example(
                Some("Local disk:"),
                Some("Local Disk"),
                &[
                    "# [storage.disks.local]",
                    "# driver = \"local\"",
                    "# root = \"storage/app\"",
                    "# url = \"/storage\"  # Public URL prefix (optional)",
                    "# visibility = \"private\"  # \"public\" or \"private\"",
                ],
                &[
                    "# STORAGE__DISKS__LOCAL__DRIVER=local",
                    "# STORAGE__DISKS__LOCAL__ROOT=storage/app",
                    "# STORAGE__DISKS__LOCAL__URL=/storage  # Public URL prefix (optional)",
                    "# STORAGE__DISKS__LOCAL__VISIBILITY=private  # \"public\" or \"private\"",
                ],
            ),
            example(
                Some("S3-compatible disk:"),
                Some("S3-Compatible Disk"),
                &[
                    "# [storage.disks.s3]",
                    "# driver = \"s3\"",
                    "# bucket = \"\"",
                    "# region = \"\"",
                    "# key = \"\"",
                    "# secret = \"\"",
                    "# endpoint = \"\"  # Custom endpoint for MinIO, R2, etc.",
                    "# url = \"\"  # Public URL prefix (optional)",
                    "# use_path_style = false",
                    "# visibility = \"private\"",
                ],
                &[
                    "# STORAGE__DISKS__S3__DRIVER=s3",
                    "# STORAGE__DISKS__S3__BUCKET=",
                    "# STORAGE__DISKS__S3__REGION=",
                    "# STORAGE__DISKS__S3__KEY=",
                    "# STORAGE__DISKS__S3__SECRET=",
                    "# STORAGE__DISKS__S3__ENDPOINT=  # Custom endpoint for MinIO, R2, etc.",
                    "# STORAGE__DISKS__S3__URL=  # Public URL prefix (optional)",
                    "# STORAGE__DISKS__S3__USE_PATH_STYLE=false",
                    "# STORAGE__DISKS__S3__VISIBILITY=private",
                ],
            ),
        ],
    ),
];

const CONFIG_HEADER: &str = "\
# =============================================================================
# Foundry Framework Configuration
#
# This file contains all available configuration options with their defaults.
# Required fields are uncommented. Optional fields are commented out so users
# can opt in only to what they need.
#
# Environment variable overlay: any key can be overridden via env vars using
# double-underscore notation. Example: DATABASE__URL=postgres://...
# =============================================================================
";

fn split_config_header(title: &str) -> String {
    format!(
        "\
# =============================================================================
# Foundry Framework Configuration - {title}
#
# Foundry loads every direct config/*.toml file in lexical filename order,
# merges the tables in memory, then applies environment variable overrides.
# Environment variables use double-underscore notation, e.g. DATABASE__URL=...
# =============================================================================
"
    )
}

const ENV_HEADER: &str = "\
# =============================================================================
# Foundry Framework - Environment Variables
#
# All configuration values can be overridden via environment variables using
# double-underscore notation: SECTION__KEY=value
#
# Nested config: AUTH__TOKENS__ACCESS_TOKEN_TTL_MINUTES=30
# Boolean values: true / false
# Integer values: 3000
#
# Copy this file to .env and fill in your values:
#   cp .env.example .env
# =============================================================================
";

pub(super) fn render_sample_config() -> String {
    render_document(CONFIG_HEADER, RenderTarget::Config)
}

pub(super) fn render_sample_config_files() -> Vec<(&'static str, String)> {
    let mut files = Vec::new();

    for section in PUBLISHED_SECTIONS {
        if files
            .iter()
            .any(|(filename, _)| *filename == section.config_file)
        {
            continue;
        }

        files.push((
            section.config_file,
            render_document(
                &split_config_header(config_file_title(section.config_file)),
                RenderTarget::ConfigFile(section.config_file),
            ),
        ));
    }

    files
}

pub(super) fn render_sample_env() -> String {
    render_document(ENV_HEADER, RenderTarget::Env)
}

#[derive(Clone, Copy)]
enum RenderTarget {
    Config,
    ConfigFile(&'static str),
    Env,
}

fn render_document(header: &str, target: RenderTarget) -> String {
    let mut out = String::from(header);
    let mut rendered_sections = 0usize;

    for section in PUBLISHED_SECTIONS.iter() {
        if let RenderTarget::ConfigFile(filename) = target {
            if section.config_file != filename {
                continue;
            }
        }

        if rendered_sections > 0 {
            out.push('\n');
        }
        rendered_sections += 1;

        push_section_banner(&mut out, section.title);

        for (part_index, part) in section.parts.iter().enumerate() {
            if part_index > 0 {
                out.push('\n');
            }

            match (target, part) {
                (
                    RenderTarget::Config | RenderTarget::ConfigFile(_),
                    PublishedPart::Table(table),
                ) => push_config_table(&mut out, table),
                (RenderTarget::Env, PublishedPart::Table(table)) => push_env_table(&mut out, table),
                (
                    RenderTarget::Config | RenderTarget::ConfigFile(_),
                    PublishedPart::Example(example),
                ) => push_example(&mut out, example.toml_heading, example.toml_lines),
                (RenderTarget::Env, PublishedPart::Example(example)) => {
                    push_example(&mut out, example.env_heading, example.env_lines)
                }
            }
        }
    }

    out
}

fn config_file_title(filename: &str) -> &str {
    match filename {
        "00-app.toml" => "Application",
        "10-http.toml" => "HTTP",
        "20-database.toml" => "Database",
        "30-security.toml" => "Security",
        "40-runtime.toml" => "Runtime",
        "50-localization.toml" => "Localization",
        "60-email.toml" => "Email",
        "70-storage.toml" => "Storage",
        _ => "Custom",
    }
}

fn push_section_banner(out: &mut String, title: &str) {
    out.push_str(
        "# -----------------------------------------------------------------------------\n",
    );
    let _ = writeln!(out, "# {title}");
    out.push_str(
        "# -----------------------------------------------------------------------------\n",
    );
}

fn push_config_table(out: &mut String, table: &PublishedTable) {
    if table.commented_header {
        let _ = writeln!(out, "# [{}]", table.path.join("."));
    } else {
        let _ = writeln!(out, "[{}]", table.path.join("."));
    }

    for field in table.fields {
        push_config_field(out, field);
    }
}

fn push_config_field(out: &mut String, field: &PublishedField) {
    if field.config_required {
        let _ = write!(out, "{} = {}", field.key, field.toml_value);
    } else {
        let _ = write!(out, "# {} = {}", field.key, field.toml_value);
    }

    if let Some(comment) = field.comment {
        let _ = write!(out, "  # {comment}");
    }

    out.push('\n');
}

fn push_env_table(out: &mut String, table: &PublishedTable) {
    if let Some(heading) = table.env_heading {
        let _ = writeln!(out, "# {heading}");
    }

    let prefix = render_env_prefix(table.path);
    for field in table.fields {
        push_env_field(out, &prefix, field);
    }
}

fn push_env_field(out: &mut String, prefix: &str, field: &PublishedField) {
    let name = format!("{prefix}__{}", field.key.to_ascii_uppercase());

    if field.env_required {
        let _ = write!(out, "{name}={}", field.env_value);
    } else {
        let _ = write!(out, "# {name}={}", field.env_value);
    }

    if let Some(comment) = field.comment {
        let _ = write!(out, "  # {comment}");
    }

    out.push('\n');
}

fn push_example(out: &mut String, heading: Option<&str>, lines: &[&str]) {
    if let Some(heading) = heading {
        let _ = writeln!(out, "# {heading}");
    }

    for line in lines {
        let _ = writeln!(out, "{line}");
    }
}

fn render_env_prefix(path: &[&str]) -> String {
    path.iter()
        .map(|segment| segment.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join("__")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use regex::Regex;

    use super::{render_sample_config, render_sample_config_files, render_sample_env};

    #[test]
    fn published_outputs_cover_all_config_repository_root_sections() {
        let expected = config_repository_root_sections();

        assert_eq!(config_root_sections(&render_sample_config()), expected);
        assert_eq!(env_root_sections(&render_sample_env()), expected);
    }

    #[test]
    fn published_config_and_env_share_same_root_order() {
        assert_eq!(
            config_root_order(&render_sample_config()),
            env_root_order(&render_sample_env())
        );
    }

    #[test]
    fn published_split_config_file_names_are_stable() {
        let names = render_sample_config_files()
            .into_iter()
            .map(|(filename, _)| filename)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "00-app.toml",
                "10-http.toml",
                "20-database.toml",
                "30-security.toml",
                "40-runtime.toml",
                "50-localization.toml",
                "60-email.toml",
                "70-storage.toml",
            ]
        );
    }

    #[test]
    fn published_env_variables_are_unique() {
        let output = render_sample_env();
        let mut seen = BTreeSet::new();

        for name in env_variable_names(&output) {
            assert!(
                seen.insert(name.clone()),
                "duplicate env variable published: {name}"
            );
        }
    }

    #[test]
    fn published_toml_tables_are_unique() {
        let output = render_sample_config();
        let mut seen = BTreeSet::new();

        for table in toml_table_names(&output) {
            assert!(
                seen.insert(table.clone()),
                "duplicate TOML table published: {table}"
            );
        }
    }

    #[test]
    fn published_database_config_includes_migration_lock_timeout_default() {
        let output = render_sample_config();
        assert!(output.contains(
            "# migration_lock_timeout_ms = 0  # Migration advisory-lock wait timeout (0 = wait forever)"
        ));
        assert!(output.contains(
            "# log_query_bindings = false  # Include SQL binding values when log_queries=true (dev only)"
        ));
        assert!(output.contains(
            "# redact_sql_literals = true  # Redact SQL literals/comments in logs and /_foundry/sql"
        ));
    }

    #[test]
    fn published_http_edge_config_includes_safe_defaults() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("[http]"));
        assert!(output.contains(
            "# max_body_size_bytes = 0  # 0 = no global body-size cap; route middleware can still cap"
        ));
        assert!(output.contains("[http.security_headers]"));
        assert!(output.contains("# enabled = true"));
        assert!(output.contains("[http.trusted_proxy]"));
        assert!(output.contains("# enabled = true  # Enabled by default so Cloudflare client IP headers work automatically"));
        assert!(output
            .contains("# headers = [\"cf-connecting-ip\", \"x-real-ip\", \"x-forwarded-for\"]"));
        assert!(output.contains("[http.cors]"));
        assert!(output.contains("[http.csrf]"));
        assert!(output.contains("# cookie_same_site = \"lax\"  # \"lax\", \"strict\", or \"none\"; none requires secure cookies"));
        assert!(output.contains("[http.rate_limit]"));
        assert!(output.contains("# enabled = true  # Enabled by default with actor-or-IP keys"));
        assert!(output.contains("# by = \"actor_or_ip\"  # \"ip\", \"actor\", or \"actor_or_ip\""));
        assert!(env.contains("# HTTP__MAX_BODY_SIZE_BYTES=0"));
        assert!(env.contains("# HTTP__CSRF__COOKIE_SAME_SITE=lax"));
        assert!(env.contains("# HTTP__TRUSTED_PROXY__TRUSTED_CIDRS=[\"173.245.48.0/20\""));
        assert!(env.contains("\"2c0f:f248::/32\"]"));
        assert!(env.contains("# HTTP__RATE_LIMIT__BY=actor_or_ip"));
    }

    #[test]
    fn published_websocket_config_includes_edge_bounds() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("[websocket]"));
        assert!(output.contains("# max_message_size_bytes = 1048576  # 0 = use transport default"));
        assert!(output.contains("# max_frame_size_bytes = 1048576"));
        assert!(output.contains("# max_write_buffer_size_bytes = 1048576"));
        assert!(output.contains("# auth_revalidation_interval_seconds = 30"));
        assert!(output.contains("# max_connections_global = 10000"));
        assert!(output.contains("# max_connections_per_ip = 100"));
        assert!(output.contains(
            "# max_subscriptions_per_connection = 100  # 0 = unlimited active subscriptions per connection"
        ));
        assert!(output.contains("# max_room_length = 256"));
        assert!(output.contains("# max_ack_id_length = 128"));
        assert!(output.contains(
            "# query_token_enabled = true  # Allow browser clients to pass short-lived bearer auth as a query token"
        ));
        assert!(output.contains("# query_token_name = \"token\""));
        assert!(output.contains("# query_token_max_length = 4096"));
        assert!(output.contains("# allowed_origins = []  # Exact Origin allow-list"));
        assert!(env.contains("# WEBSOCKET__MAX_MESSAGE_SIZE_BYTES=1048576"));
        assert!(env.contains("# WEBSOCKET__AUTH_REVALIDATION_INTERVAL_SECONDS=30"));
        assert!(env.contains("# WEBSOCKET__MAX_CONNECTIONS_GLOBAL=10000"));
        assert!(env.contains("# WEBSOCKET__MAX_CONNECTIONS_PER_IP=100"));
        assert!(env.contains("# WEBSOCKET__MAX_SUBSCRIPTIONS_PER_CONNECTION=100"));
        assert!(env.contains("# WEBSOCKET__QUERY_TOKEN_ENABLED=true"));
        assert!(env.contains("# WEBSOCKET__QUERY_TOKEN_MAX_LENGTH=4096"));
    }

    #[test]
    fn published_email_config_includes_http_provider_timeouts() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("# [email.mailers.resend]"));
        assert!(output.contains("# [email.mailers.postmark]"));
        assert!(output.contains("# [email.mailers.mailgun]"));
        assert!(output.contains("# [email.mailers.ses]"));
        assert!(output.contains("# timeout_secs = 30"));
        assert!(output.contains("# max_attachment_bytes = 26214400"));
        assert!(output.contains("# max_total_attachment_bytes = 26214400"));
        assert!(env.contains("# EMAIL__MAX_ATTACHMENT_BYTES=26214400"));
        assert!(env.contains("# EMAIL__MAX_TOTAL_ATTACHMENT_BYTES=26214400"));
        assert!(env.contains("# EMAIL__MAILERS__RESEND__TIMEOUT_SECS=30"));
        assert!(env.contains("# EMAIL__MAILERS__POSTMARK__TIMEOUT_SECS=30"));
        assert!(env.contains("# EMAIL__MAILERS__MAILGUN__TIMEOUT_SECS=30"));
        assert!(env.contains("# EMAIL__MAILERS__SES__TIMEOUT_SECS=30"));
    }

    #[test]
    fn published_auth_lifecycle_config_includes_worker_pruning_defaults() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("# prune_retention_days = 30  # Auto-prune expired/revoked tokens older than N days (0 = app-owned/manual)"));
        assert!(output.contains("# [auth.tokens.guards.admin]"));
        assert!(output.contains("# access_token_ttl_minutes = 43200"));
        assert!(output
            .contains("# cookie_domain = \"\"  # Optional Set-Cookie Domain; empty omits Domain"));
        assert!(output.contains("[auth.password_resets]"));
        assert!(output.contains(
            "# expiry_minutes = 60  # Password reset token lifetime (0 = no expiry/auto-prune)"
        ));
        assert!(output.contains("[auth.email_verification]"));
        assert!(output.contains("# expiry_minutes = 1440  # Email verification token lifetime (0 = no expiry/auto-prune)"));
        assert!(env.contains("# AUTH__TOKENS__PRUNE_RETENTION_DAYS=30"));
        assert!(env.contains("# AUTH__SESSIONS__COOKIE_SAME_SITE=lax"));
        assert!(env.contains("# AUTH__PASSWORD_RESETS__EXPIRY_MINUTES=60"));
        assert!(env.contains("# AUTH__EMAIL_VERIFICATION__EXPIRY_MINUTES=1440"));
    }

    #[test]
    fn published_audit_config_includes_redaction_defaults() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("[audit]"));
        assert!(output.contains(
            "# redact_sensitive_fields = true  # Redact common credential-like model columns in audit JSON"
        ));
        assert!(output.contains("\"password_hash\""));
        assert!(output.contains("\"refresh_token\""));
        assert!(env.contains("# AUDIT__REDACT_SENSITIVE_FIELDS=true"));
        assert!(env.contains("\"password_hash\""));
    }

    #[test]
    fn published_runtime_config_includes_tokio_sizing_knobs() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("[runtime]"));
        assert!(output.contains(
            "# worker_threads = 0  # Tokio worker thread count for sync runners (0 = Tokio default)"
        ));
        assert!(output.contains(
            "# max_blocking_threads = 0  # Tokio blocking thread cap for sync runners (0 = Tokio default)"
        ));
        assert!(env.contains("# RUNTIME__WORKER_THREADS=0"));
        assert!(env.contains("# RUNTIME__MAX_BLOCKING_THREADS=0"));
    }

    #[test]
    fn published_cache_config_includes_operational_hardening_defaults() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("[cache]"));
        assert!(output.contains("# error_mode = \"strict\"  # \"strict\" or \"fail_open\""));
        assert!(output.contains("# key_max_length = 512"));
        assert!(output.contains("# remember_singleflight = true"));
        assert!(output.contains("# remember_distributed_lock = false"));
        assert!(output.contains("# remember_lock_ttl_ms = 30000"));
        assert!(env.contains("# CACHE__ERROR_MODE=strict"));
        assert!(env.contains("# CACHE__KEY_MAX_LENGTH=512"));
        assert!(env.contains("# CACHE__REMEMBER_SINGLEFLIGHT=true"));
        assert!(env.contains("# CACHE__REMEMBER_DISTRIBUTED_LOCK=false"));
        assert!(env.contains("# CACHE__REMEMBER_LOCK_TTL_MS=30000"));
    }

    #[test]
    fn published_datatable_config_includes_export_bounds() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("[datatable]"));
        assert!(output.contains("# max_per_page = 500  # Max JSON rows per page (0 = unlimited)"));
        assert!(output.contains(
            "# max_export_rows = 50000  # Max rows generated into XLSX downloads/jobs (0 = unlimited)"
        ));
        assert!(env.contains("# DATATABLE__MAX_PER_PAGE=500"));
        assert!(env.contains("# DATATABLE__MAX_EXPORT_ROWS=50000"));
    }

    #[test]
    fn published_storage_config_includes_upload_cost_controls() {
        let output = render_sample_config();
        let env = render_sample_env();

        assert!(output.contains("[storage]"));
        assert!(output.contains("# max_upload_size_bytes = 104857600  # Total multipart file bytes per request (0 = no storage-level cap)"));
        assert!(output
            .contains("# max_upload_file_size_bytes = 52428800  # Per-file multipart upload cap"));
        assert!(
            output.contains("# max_upload_files = 20  # Max uploaded files per multipart request")
        );
        assert!(output.contains("# upload_temp_retention_seconds = 3600"));
        assert!(output.contains("# image_max_input_bytes = 52428800"));
        assert!(output.contains("# image_max_pixels = 50000000"));
        assert!(output.contains("# attachment_orphan_audit_enabled = true"));
        assert!(output.contains("# attachment_orphan_delete_enabled = false"));
        assert!(output.contains("# attachment_orphan_prefix = \"attachments/\""));
        assert!(env.contains("# STORAGE__MAX_UPLOAD_SIZE_BYTES=104857600"));
        assert!(env.contains("# STORAGE__UPLOAD_TEMP_RETENTION_SECONDS=3600"));
        assert!(env.contains("# STORAGE__IMAGE_MAX_INPUT_BYTES=52428800"));
        assert!(env.contains("# STORAGE__ATTACHMENT_ORPHAN_AUDIT_ENABLED=true"));
        assert!(env.contains("# STORAGE__ATTACHMENT_ORPHAN_PREFIX=attachments/"));
    }

    fn config_repository_root_sections() -> BTreeSet<String> {
        let pattern = Regex::new(r#"self\.section\("([a-z0-9_]+)"\)"#).unwrap();
        pattern
            .captures_iter(include_str!("mod.rs"))
            .map(|caps| caps[1].to_string())
            .collect()
    }

    fn config_root_sections(output: &str) -> BTreeSet<String> {
        config_root_order(output).into_iter().collect()
    }

    fn env_root_sections(output: &str) -> BTreeSet<String> {
        env_root_order(output).into_iter().collect()
    }

    fn config_root_order(output: &str) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();

        for table in toml_table_names(output) {
            let root = table.split('.').next().unwrap().to_string();
            if seen.insert(root.clone()) {
                ordered.push(root);
            }
        }

        ordered
    }

    fn env_root_order(output: &str) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();

        for name in env_variable_names(output) {
            let root = name.split("__").next().unwrap().to_ascii_lowercase();
            if seen.insert(root.clone()) {
                ordered.push(root);
            }
        }

        ordered
    }

    fn toml_table_names(output: &str) -> Vec<String> {
        let pattern = Regex::new(r#"(?m)^#?\s*\[([a-z0-9_.]+)\]\s*$"#).unwrap();
        pattern
            .captures_iter(output)
            .map(|caps| caps[1].to_string())
            .collect()
    }

    fn env_variable_names(output: &str) -> Vec<String> {
        let pattern = Regex::new(r#"(?m)^#?\s*([A-Z0-9]+(?:__[A-Z0-9_]+)+)="#).unwrap();
        pattern
            .captures_iter(output)
            .map(|caps| caps[1].to_string())
            .collect()
    }
}
