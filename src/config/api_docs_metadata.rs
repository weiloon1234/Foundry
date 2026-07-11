use std::fmt::Write as _;

/// Optional one-line description per top-level module for the index.
/// New modules work without an entry here — they just show no description.
pub(crate) fn module_description(stem: &str) -> &'static str {
    match stem {
        "app_enum" => "Enum metadata and serialization (FoundryAppEnum)",
        "audit" => "Built-in audit logging with automatic model mutation tracking and redaction",
        "attachments" => "File attachments with lifecycle (HasAttachments)",
        "auth" => "Auth: guards, policies, tokens, sessions, password reset, email verification",
        "cache" => "In-memory and Redis-backed caching (CacheManager)",
        "cli" => "CLI command registration (CommandRegistry)",
        "config" => "TOML-based configuration (ConfigRepository, AppConfig, etc.)",
        "contract" => {
            "Normalized contract manifest for generated SDKs, OpenAPI, validation, and realtime"
        }
        "countries" => "Built-in country data (250 countries)",
        "database" => "AST-first query system: models, relations, projections, compiler",
        "datatable" => "Server-side datatables: filtering, sorting, pagination, XLSX export",
        "email" => "Multi-driver email: SMTP, Mailgun, Postmark, Resend, SES",
        "events" => "Domain event bus with typed listeners",
        "foundation" => "Core: App, AppBuilder, AppContext, AppTransaction, Error, ServiceProvider",
        "http" => "HTTP: routes, middleware (CORS, CSRF, rate limit, etc.), cookies, resources",
        "http_client" => {
            "Outbound HTTP: pooled transport, timeouts, safe retries, typed responses, and fakes"
        }
        "i18n" => "Internationalization: locale extraction, translation catalogs",
        "imaging" => "Image processing pipeline (resize, crop, rotate, format conversion)",
        "jobs" => "Background job queue with leased at-least-once delivery",
        "kernel" => "5 runtime kernels: HTTP, CLI, Scheduler, Worker, WebSocket",
        "logging" => "Structured logging, observability, health probes, diagnostics",
        "metadata" => "Key-value metadata for models (HasMetadata)",
        "notifications" => "Multi-channel notifications: email, database, broadcast",
        "openapi" => "OpenAPI 3.1.0 spec generation (ApiSchema, RouteDoc)",
        "plugin" => "Compile-time plugin system with dependency validation",
        "public" => "Stable convenience re-exports for consumer applications",
        "redis" => "Namespaced Redis wrapper (RedisManager, RedisConnection)",
        "scheduler" => "Cron + interval scheduling with Redis-safe leadership",
        "settings" => "Typed persistent application settings with cache integration",
        "storage" => "File storage: local + S3, multipart uploads, file validation",
        "support" => "Utilities: typed IDs, datetime/clock, Collection<T>, crypto, hashing, locks",
        "testing" => "Test infrastructure: TestApp, clients/fakes, assertions, and model factories",
        "translations" => "Model field translations across locales (HasTranslations)",
        "typescript" => "TypeScript contracts, route helpers, validation metadata, and SDK export",
        "validation" => "Validation: 38+ rules, custom rules, request validation extractor",
        "websocket" => "Channel-based WebSocket with presence and typed messages",
        _ => "",
    }
}

pub(crate) fn append_module_notes(group_key: &str, content: &mut String) {
    let notes = module_notes(group_key);
    if notes.is_empty() {
        return;
    }

    writeln!(content, "## Notes").unwrap();
    writeln!(content).unwrap();
    for note in notes {
        writeln!(content, "- {note}").unwrap();
    }
    writeln!(content).unwrap();
}

pub(crate) fn ensure_single_trailing_newline(content: &mut String) {
    let trimmed_len = content.trim_end().len();
    content.truncate(trimmed_len);
    content.push('\n');
}

fn module_notes(group_key: &str) -> &'static [&'static str] {
    match group_key {
        "config" => &[
            "`AppConfig` fields: `name`, `environment`, `timezone`, `signing_key`, `background_shutdown_timeout_ms`.",
            "`AuditConfig.redact_sensitive_fields` is enabled by default and redacts common credential-like model columns in audit JSON.",
            "`HttpConfig` is optional and additive: global body cap, request timeout, CORS, and CSRF are opt-in; trusted proxy is enabled by default for Cloudflare CIDRs, rate limiting is enabled by default with `actor_or_ip`, and security headers are enabled by default with HSTS off.",
            "`CacheConfig.error_mode` defaults to `strict`; `remember_singleflight` is enabled by default and distributed remember locks are opt-in.",
            "`DatabaseConfig.migration_lock_timeout_ms` defaults to `0`; `db:migrate` and `db:rollback` wait forever for the migration advisory lock unless overridden.",
            "`DatabaseConfig.connect_lazy` and the `write_pool` / `read_pool` override sections support serverless pool tuning without changing the legacy flat pool keys.",
            "`DatabaseConfig.redact_sql_literals` is enabled by default so SQL logs and `/_foundry/sql` retain query shape without common literal values.",
            "`DatatableConfig` caps JSON `per_page` and XLSX export row counts by default; `0` disables each cap.",
            "`JobsConfig` includes `shutdown_timeout_ms` for active worker job draining; `0` aborts active jobs immediately.",
            "`JobsConfig.history_retention_days` defaults to `30`; `0` keeps `job_history` forever.",
            "`ObservabilityConfig.enabled` gates `/_foundry/*` route registration; `capture_enabled` gates passive runtime capture.",
            "`RuntimeConfig.worker_threads` and `max_blocking_threads` default to `0`, which keeps Tokio defaults for Foundry-owned sync runners.",
            "`SchedulerConfig` includes `shutdown_timeout_ms` for active schedule task draining; `0` aborts active schedules immediately.",
            "`WebSocketConfig` bounds inbound message/frame sizes, query auth token length, and client-supplied channel, room, event, ack, and subscription cardinality.",
        ],
        "support" => &[
            "`run_blocking(label, work)` isolates CPU-heavy or blocking synchronous work on Tokio's blocking pool and maps task panics into Foundry errors.",
            "`HashManager::hash()`, `HashManager::check()`, and `HashManager::needs_rehash()` remain synchronous; wrap password hashing or checking in `run_blocking` inside async handlers or model mutators.",
        ],
        "cache" => &[
            "Cache keys are validated before backend access; Redis nil/missing keys are distinct from backend failures.",
            "`remember()` uses local single-flight by default and can coordinate across workers with an opt-in distributed lock.",
            "`cache.error_mode = \"fail_open\"` logs backend I/O failures and continues, while validation, serialization, and callback errors remain strict.",
        ],
        "audit" => &[
            "`#[foundry(audit_exclude)]` still removes a field entirely from audit payloads.",
            "`audit.redact_sensitive_fields = true` masks common credential-like field names with `[redacted]` in before/after/changes JSON.",
            "`audit.sensitive_fields` adds project-specific names; set `redact_sensitive_fields = false` to return to explicit model-only exclusions.",
        ],
        "datatable" => &[
            "JSON responses clamp `DatatableRequest.per_page` to `datatable.max_per_page` unless the cap is `0`.",
            "XLSX downloads and queued exports apply `datatable.max_export_rows` before loading rows into memory; `0` disables the cap.",
        ],
        "email" => &[
            "Built-in HTTP mailers use `timeout_secs = 30` by default; `0` disables the reqwest timeout for local debugging.",
            "`EmailConfig.max_attachment_bytes` and `max_total_attachment_bytes` bound resolved attachment payloads before provider delivery; `0` disables each cap.",
            "The built-in SES driver uses the SES SendEmail API and rejects attachments clearly instead of silently dropping them.",
            "Provider error bodies are truncated and obvious secret fields are redacted before they are returned or logged.",
        ],
        "http" => &[
            "`HttpConfig.security_headers` is applied globally by default with HSTS disabled until explicitly enabled.",
            "`HttpConfig.trusted_proxy` honors forwarded client IP headers only from configured CIDRs; the default CIDR set trusts Cloudflare ranges, and `TrustedProxy::new()` uses the same Cloudflare-safe default.",
            "Config-derived CORS validates origins, methods, and headers at boot; wildcard origins with credentials are rejected.",
            "Config-derived CSRF is opt-in; code-registered `Csrf` remains source-compatible and path exclusions are segment-aware.",
            "Config-derived body-limit, request-timeout, and rate-limit rejections return JSON `ErrorResponse` bodies with HTTP 413, 408, and 429.",
            "Actor-only rate limits require an authenticated actor; use `actor_or_ip` when a global rate limit needs an IP fallback.",
            "IP rate limits use `TrustedProxy` real IP when available and otherwise fall back to TCP peer connect info on the real server path.",
        ],
        "http_client" => &[
            "The default client has no base URL, a 10-second connect timeout, a 30-second per-attempt request timeout, concurrency 64, and conservative retries for read-only methods.",
            "Mutation methods are not retried unless explicitly selected. `raw()` bypasses Foundry retry, timeout, concurrency, tracing, and fake behavior.",
            "Framework traces redact URL credentials and query values and never record header values or bodies.",
        ],
        "jobs" => &[
            "`JobsConfig.shutdown_timeout_ms` defaults to `30000`; `0` aborts active jobs immediately on shutdown.",
            "Shutdown-aborted jobs are left unacked so lease expiry and the existing requeue flow make them runnable again.",
            "Job handler panics are handled as normal job failures and use the existing retry/dead-letter flow.",
            "`job_history` is pruned by workers with a distributed lock; consumer apps do not need to register a cleanup scheduler.",
            "`spawn_worker(app)` is managed by the app lifecycle and remains capped by `app.background_shutdown_timeout_ms`.",
        ],
        "logging" => &[
            "`/_foundry/runtime` returns the structured `RuntimeSnapshot`; `/_foundry/metrics` exposes the same runtime counter families in Prometheus text format.",
            "Foundry does not store Prometheus samples; scrape retention belongs to Prometheus or your metrics backend.",
            "`ObservabilityConfig.enabled` controls `/_foundry/*` route registration; `capture_enabled` controls passive runtime capture while preserving route availability.",
            "Runtime counters, HTTP samples, SQL slow queries, N+1 suspects, and WebSocket channel counters are bounded process memory and reset on restart.",
            "`/_foundry/sql` returns slow-query stats, top-slowest ranking, and potential HTTP N+1 suspects while preserving the existing `slow_queries` key; SQL literals and comments are redacted by default.",
        ],
        "scheduler" => &[
            "Schedule handler panics are handled as schedule failures and route through `ScheduleOptions::on_failure`.",
            "Scheduler hooks are isolated: hook panics are logged and do not crash the scheduler task.",
            "`SchedulerConfig.shutdown_timeout_ms` defaults to `30000`; `0` aborts active schedules immediately on shutdown.",
        ],
        "websocket" => &[
            "WebSocket handshakes use HTTP trusted-proxy config for client IP metadata; forwarded IP headers are ignored unless the TCP peer is trusted.",
            "Empty `websocket.allowed_origins` permits same-origin browser handshakes in production-like environments and rejects cross-origin browser handshakes.",
            "Inbound messages, frames, query auth tokens, subscriptions, and client-supplied identifiers are bounded by `WebSocketConfig`.",
            "`websocket.query_token_enabled` stays on by default for browser compatibility; keep issued WebSocket tokens short-lived because query strings can be logged outside Foundry.",
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::{append_module_notes, ensure_single_trailing_newline, module_description};

    #[test]
    fn generated_markdown_has_one_trailing_newline() {
        let mut content = "content\n\n\n".to_string();
        ensure_single_trailing_newline(&mut content);
        assert_eq!(content, "content\n");
    }

    #[test]
    fn module_descriptions_cover_recent_public_modules() {
        assert_eq!(
            module_description("audit"),
            "Built-in audit logging with automatic model mutation tracking and redaction"
        );
        assert_eq!(
            module_description("contract"),
            "Normalized contract manifest for generated SDKs, OpenAPI, validation, and realtime"
        );
        assert_eq!(
            module_description("http_client"),
            "Outbound HTTP: pooled transport, timeouts, safe retries, typed responses, and fakes"
        );
        assert_eq!(
            module_description("public"),
            "Stable convenience re-exports for consumer applications"
        );
        assert_eq!(
            module_description("settings"),
            "Typed persistent application settings with cache integration"
        );
        assert_eq!(
            module_description("typescript"),
            "TypeScript contracts, route helpers, validation metadata, and SDK export"
        );
    }

    #[test]
    fn module_notes_include_background_shutdown_metadata() {
        let mut config = String::new();
        append_module_notes("config", &mut config);
        assert!(config.contains("background_shutdown_timeout_ms"));
        assert!(config.contains("JobsConfig"));
        assert!(config.contains("HttpConfig"));
        assert!(config.contains("CacheConfig.error_mode"));
        assert!(config.contains("redact_sql_literals"));
        assert!(config.contains("history_retention_days"));
        assert!(config.contains("ObservabilityConfig.enabled"));
        assert!(config.contains("RuntimeConfig"));
        assert!(config.contains("SchedulerConfig"));
        assert!(config.contains("WebSocketConfig"));
        assert!(config.contains("0` aborts"));

        let mut support = String::new();
        append_module_notes("support", &mut support);
        assert!(support.contains("run_blocking"));
        assert!(support.contains("HashManager::hash()"));
        assert!(support.contains("HashManager::needs_rehash()"));

        let mut jobs = String::new();
        append_module_notes("jobs", &mut jobs);
        assert!(jobs.contains("JobsConfig.shutdown_timeout_ms"));
        assert!(jobs.contains("lease expiry"));
        assert!(jobs.contains("retry/dead-letter"));
        assert!(jobs.contains("job_history"));
        assert!(jobs.contains("spawn_worker(app)"));
        assert!(jobs.contains("app.background_shutdown_timeout_ms"));

        let mut scheduler = String::new();
        append_module_notes("scheduler", &mut scheduler);
        assert!(scheduler.contains("Schedule handler panics"));
        assert!(scheduler.contains("SchedulerConfig.shutdown_timeout_ms"));

        let mut logging = String::new();
        append_module_notes("logging", &mut logging);
        assert!(logging.contains("/_foundry/runtime"));
        assert!(logging.contains("/_foundry/metrics"));
        assert!(logging.contains("Prometheus"));
        assert!(logging.contains("capture_enabled"));
        assert!(logging.contains("literals and comments are redacted"));

        let mut http = String::new();
        append_module_notes("http", &mut http);
        assert!(http.contains("security_headers"));
        assert!(http.contains("trusted_proxy"));
        assert!(http.contains("CSRF"));
        assert!(http.contains("413"));
        assert!(http.contains("actor_or_ip"));

        let mut cache = String::new();
        append_module_notes("cache", &mut cache);
        assert!(cache.contains("single-flight"));
        assert!(cache.contains("fail_open"));
        assert!(cache.contains("backend failures"));

        let mut audit = String::new();
        append_module_notes("audit", &mut audit);
        assert!(audit.contains("audit_exclude"));
        assert!(audit.contains("redacted"));

        let mut datatable = String::new();
        append_module_notes("datatable", &mut datatable);
        assert!(datatable.contains("max_per_page"));
        assert!(datatable.contains("max_export_rows"));

        let mut email = String::new();
        append_module_notes("email", &mut email);
        assert!(email.contains("timeout_secs"));
        assert!(email.contains("max_attachment_bytes"));
        assert!(email.contains("rejects attachments"));
        assert!(email.contains("redacted"));

        let mut websocket = String::new();
        append_module_notes("websocket", &mut websocket);
        assert!(websocket.contains("trusted-proxy"));
        assert!(websocket.contains("bounded"));
    }
}
