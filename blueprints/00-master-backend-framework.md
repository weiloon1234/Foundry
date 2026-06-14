# Rust Backend Framework Blueprint (Kernel + Modular Architecture)

> **Status:** Active development
> **Last updated:** 2026-04-12

## Overview

This document defines a **framework-level architecture** for a modern Rust backend framework.

The goal:

> Allow application projects to remain thin, focusing only on **bootstrap + registration**, while the framework handles runtime, orchestration, and infrastructure.

---

# Framework Naming

**Status: ✅ Done** — Named **Foundry**

Reason:
- conveys building systems
- aligns with backend + infra mindset
- strong brand positioning

---

# Architectural Style

This framework follows:

**Modular Layered Architecture with Application Kernels**

Influenced by:
- Clean Architecture
- Laravel Kernel / Service Provider pattern
- Hexagonal Architecture (partial)

---

# Core Philosophy

## Project SHOULD NOT:
- manage server lifecycle
- manually wire dependencies everywhere
- duplicate infrastructure logic

## Project SHOULD:
- define domains
- define use cases
- define portals
- register modules into framework

---

# Final Goal (Consumer Experience)

**Status: ✅ Done** — Matches actual API

```rust
use foundry::prelude::*;

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(AppServiceProvider)
        .register_routes(app::portals::router)
        .register_commands(app::commands::register)
        .register_schedule(app::schedules::register)
        .register_validation_rule("mobile", MobileRule)
        .run_http()?;

    Ok(())
}
```

This is the **target developer experience**.

---

# Framework Structure

**Status: ✅ Done** — All 21 modules implemented. New modules added beyond the original blueprint: `plugin/`, `i18n/`, `redis/`, `storage/`, `email/`, `app_enum/`.

```text
src/
├── foundation/    ✅ App, Builder, Container, ServiceProvider, Error
├── kernel/        ✅ HTTP, CLI, Scheduler, WebSocket, Worker kernels
├── http/          ✅ Routing, route options, API versioning (group/api_version), auth middleware, 9 middleware types (CORS, SecurityHeaders, CSRF, RateLimit w/ per-user, MaxBodySize, Timeout, TrustedProxy, Compression, SPA serving)
├── websocket/     ✅ Channels, pub/sub, rooms, presence (join/leave events), heartbeat, rate limiting, max connections, auth callbacks, force disconnect, client events, lifecycle hooks, message ACK, replay on subscribe
├── scheduler/     ✅ Cron, interval, distributed leadership via Redis
├── cli/           ✅ Command registration, Clap integration, make:model/job/command scaffolding
├── validation/    ✅ 30 built-in rules + custom rule registration + #[derive(Validate)] macro + app_enum validation
├── auth/          ✅ Actor, role, permission, policy, Authenticatable, Auth<M>, TokenManager, SessionManager, multi-guard (token + session)
├── events/        ✅ Event dispatch, typed listeners, job dispatch
├── jobs/          ✅ Spawn-per-job (semaphore-bounded), timeout, shutdown, middleware hooks, batching, chaining, rate limiting, unique jobs, status tracking, dashboard API
├── config/        ✅ TOML + env overlay, 11 typed config sections
├── logging/       ✅ NDJSON structured logging, file sink, Prometheus /metrics, optional OpenTelemetry tracing (otel feature)
├── database/      ✅ Full ORM, PostgreSQL FTS (.search()), Condition::Raw, audit trail (actor in hooks)
├── openapi/       ✅ #[derive(ApiSchema)], RouteDoc builder, OpenAPI 3.1 spec generation, /_foundry/openapi.json
├── email/         ✅ Multi-mailer email system (SMTP + log drivers), queue integration
├── storage/       ✅ Multi-disk storage (local + S3), upload helpers, multipart extractors
├── cache/         ✅ CacheManager with Redis + Memory drivers, get/put/remember/forget/flush
├── notifications/ ✅ Adapter pattern, NotificationChannel trait, typed NotificationChannelId, channel registry, 3 built-in channels (Email/Database/Broadcast), project-level custom channels, per-channel routing, queued dispatch via SendNotificationJob
├── testing/       ✅ TestApp, TestClient, TestResponse (no-TCP testing), Factory trait, FactoryBuilder
├── support/       ✅ Collection<T>, semantic IDs, HashManager, CryptManager, Token, SHA-256, sanitize_html
├── redis/         ✅ Public namespaced Redis app API
├── plugin/        ✅ Dependency resolution, assets, scaffolding, CLI
├── i18n/          ✅ I18nManager, i18next-compatible JSON, per-request locale, Axum extractor
├── app_enum/      ✅ App enum system — #[derive(AppEnum)] with FoundryAppEnum trait, ToDbValue/FromDbValue, Serialize/Deserialize, validation integration, metadata
├── prelude.rs
└── lib.rs
```

---

# 1. foundation/

**Status: ✅ Done — Exceeds blueprint**

The heart of the framework.

## Responsibilities
- App builder
- Dependency container
- Lifecycle management
- Module/service provider system

## Key Components

### App
- global runtime container

### Builder
- fluent bootstrap API

### Container
- service registry / DI

### ServiceProvider
- module registration pattern

### Drift
Blueprint listed 4 components. Implementation includes:
- Transaction-aware service resolution
- Plugin integration hooks
- Structured `Error` enum: `Message`, `Http`, `NotFound`, `Other` with consistent JSON responses
- `From<ValidationErrors>` and `From<AuthError>` conversions for unified error handling

---

# 2. kernel/

**Status: ✅ Done — Exceeds blueprint**

Runtime boot layers.

## Types

- HTTP Kernel
- CLI Kernel
- Scheduler Kernel
- WebSocket Kernel

### Drift
Blueprint listed 4 kernel types. Implementation added:
- **Worker Kernel** — background job processing
- Integrated observability per kernel

---

# 3. http/

**Status: ✅ Done — Security middleware system complete**

## Responsibilities
- routing
- middleware
- request/response
- guards

### Drift
- Route options system (auth guards, permissions per route)
- Auth middleware auto-injection based on route config
- Request logging built into kernel

### Middleware System (NEW)

Framework-provided security middleware, registered via `App::builder().register_middleware()`:

```rust
use foundry::prelude::*;

App::builder()
    .register_middleware(TrustedProxy::cloudflare().build())
    .register_middleware(Cors::new()
        .allow_origins(["https://myapp.com"])
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE, AUTHORIZATION])
        .build()
    )
    .register_middleware(SecurityHeaders::new().build())
    .register_middleware(RateLimit::new(60).per_minute().build())
    .register_middleware(MaxBodySize::mb(10).build())
    .register_middleware(RequestTimeout::secs(30).build())
    .run_http()?;
```

**Built-in middleware:**

| Middleware | Purpose | Implementation |
|------------|---------|----------------|
| `Cors` | Cross-origin access control | Wraps `tower-http::cors::CorsLayer` |
| `SecurityHeaders` | X-Content-Type-Options, X-Frame-Options, HSTS, Referrer-Policy, X-XSS-Protection | Custom Axum middleware |
| `RateLimit` | Fixed-window rate limiting by IP (Redis-backed, in-memory fallback) | Custom async middleware |
| `MaxBodySize` | Request body size limit | Wraps `tower-http::limit::RequestBodyLimitLayer` |
| `RequestTimeout` | Request timeout with 408 status | Wraps `tower-http::timeout::TimeoutLayer` |
| `TrustedProxy` | Correct client IP behind proxies/CDN (Cloudflare-aware) | Custom middleware, `RealIp` extension |

**Internal layer order (priority):** TrustedProxy (0) → CORS (10) → Security Headers (20) → Rate Limit (30) → Body Size (40) → Timeout (50) → Auth → Logging → Handler

### Drift
- Rate limiting uses Redis automatically when configured, falls back to in-memory for dev/testing.
- Per-route middleware supported via `HttpRouteOptions::middleware()`.
- Each middleware builder requires `.build()` to convert into `MiddlewareConfig` before registration.

### Per-Route Middleware

Middleware can be applied to specific routes via `HttpRouteOptions`:

```rust
registrar.route_with_options(
    "/api/users",
    get(users_handler),
    HttpRouteOptions::new()
        .guard("api")
        .middleware(RateLimit::new(100).per_minute().build())
        .middleware(MaxBodySize::mb(5).build()),
);
```

Per-route middleware runs between global middleware and auth middleware in the layer stack.

### TODO
- Per-route middleware groups (named presets, future)

---

# 4. websocket/

**Status: ✅ Done — Exceeds blueprint**

## Responsibilities
- connection handling
- channel system
- message routing

### Drift
Blueprint described basic connection/channel/routing. Implementation went far beyond:
- Pub/sub with rooms
- Auth integration
- Redis backend for distributed messaging
- Event bus integration (events can publish to WebSocket channels)

---

# 5. scheduler/

**Status: ✅ Done — Exceeds blueprint**

## Responsibilities
- cron jobs
- interval jobs
- job registry

### Drift
Blueprint described basic cron/interval. Implementation added:
- Distributed leadership election via Redis
- Lease management with atomic Lua scripts
- Failover when leader drops

---

# 6. cli/

**Status: ✅ Done** — Matches blueprint.

Artisan-like system.

## Responsibilities
- command registration
- argument parsing
- execution

---

# 7. validation/

**Status: ✅ Done — Exceeds blueprint**

Laravel-style validation engine with translation-aware messages.

## Features
- built-in rules
- custom rule registration
- chainable API
- translation-aware messages (i18n integration)
- custom messages per field+rule (inline, validator-level, translation file)
- custom attribute names (field → display name)

### Drift
Blueprint listed 3 features generically. Implementation includes:

**36 built-in rules:**
| Category | Rules |
|----------|-------|
| Presence | `required` |
| String | `min`, `max`, `alpha`, `alpha_numeric`, `digits`, `starts_with`, `ends_with` |
| Numeric | `numeric`, `integer`, `min_numeric`, `max_numeric`, `between` |
| Format | `email`, `url`, `uuid`, `regex`, `json`, `timezone` |
| Date/Time | `date`, `time`, `datetime`, `local_datetime` |
| IP | `ip`, `ipv4`, `ipv6` |
| List | `in_list`, `not_in` |
| Comparison | `confirmed`, `same`, `different`, `before`, `before_or_equal`, `after`, `after_or_equal` |
| Database (async) | `unique`, `exists` |
| File | `image`, `max_file_size`, `max_dimensions`, `min_dimensions`, `allowed_mimes`, `allowed_extensions` |

**Modifiers:** `nullable`, `bail`

**Custom rules:** `ValidationRule` trait with async support and `RuleContext` providing `AppContext` access.

**Request validation:** `RequestValidator` trait + `Validated<T>` Axum extractor for auto-validation in handlers.

### Translation-Aware Messages

All validation messages are resolved through a 5-tier priority chain:

1. **Inline `.with_message()`** — per-rule: `validator.field("email", &v).required().with_message("We need your email!")`
2. **Validator-level `custom_message()`** — per field+rule: `validator.custom_message("email", "required", "Custom message")`
3. **i18n custom** — per field+rule from translation file: `validation.custom.email.required`
4. **i18n default** — per rule from translation file: `validation.required`
5. **Hardcoded fallback** — built-in English messages with `{{attribute}}` placeholders

**Attribute name resolution** for the `{{attribute}}` placeholder:
1. `validator.custom_attribute("email", "email address")`
2. i18n `validation.attributes.email`
3. Raw field name

**Per-request locale:** `Validated<T>` extractor resolves locale from `Accept-Language` header (or `Locale` extension set by middleware) and passes it to the validator.

**Message placeholders:** Use i18next-compatible `{{var}}` syntax:
- `{{attribute}}` — field display name
- `{{min}}`, `{{max}}` — rule parameters
- `{{other}}` — comparison field name
- `{{value}}` — string prefix/suffix

**RequestValidator trait** supports Laravel FormRequest-style customization:

```rust
#[async_trait]
impl RequestValidator for CreateUser {
    async fn validate(&self, validator: &mut Validator) -> Result<()> {
        validator.field("email", &self.email).required().email().apply().await?;
        Ok(())
    }

    fn messages(&self) -> Vec<(String, String, String)> {
        vec![("email".into(), "required".into(), "We need your email!".into())]
    }

    fn attributes(&self) -> Vec<(String, String)> {
        vec![("email".into(), "email address".into())]
    }
}
```

### Derive Macro: `#[derive(Validate)]`

For the common case (~90%), use the derive macro instead of manual `RequestValidator` impls:

```rust
#[derive(Debug, Deserialize, Validate)]
#[validate(
    messages(email(unique = "This email is already registered.")),
    attributes(email = "email address"),
)]
pub struct CreateUser {
    #[validate(required, email, unique("users", "email"))]
    pub email: String,

    #[validate(required, min(8), confirmed("password_confirmation"))]
    pub password: String,

    #[validate(required)]
    pub password_confirmation: String,
}

// In handler — Validated<T> extractor auto-applies messages/attributes
async fn create_user(Validated(input): Validated<CreateUser>) -> Json<Value> {
    // input is validated, all rules passed
}
```

**Attribute syntax:**

| Syntax | Description |
|--------|-------------|
| `#[validate(required, email)]` | Simple rules (no params) |
| `#[validate(min(8), max(100))]` | Rules with params |
| `#[validate(required(message = "Custom!"))]` | Per-rule message override |
| `#[validate(unique("users", "email"))]` | Database rules (async) |
| `#[validate(confirmed("password_confirmation"))]` | Cross-field rules (generates `&self.other_field`) |
| `#[validate(bail, required, email)]` | Stop on first error |
| `#[validate(nullable, email)]` | Skip rules when empty |
| `#[validate(each(required, min(2)))]` | Array validation (`Vec<T>`) |
| `#[validate(rule("custom_name"))]` | Custom registered rule |
| `Option<T>` field | Auto-adds `nullable`, converts `None` to `""` |
| `#[validate(image, max_file_size(2048))]` | File validation rules (`UploadedFile` fields) |

Manual `RequestValidator` impls remain for conditional validation, dynamic rule selection, and other edge cases. Both approaches coexist.

### File Validation

Six file validation rules for `UploadedFile` and `Option<UploadedFile>` fields:

| Rule | Description | Example |
|------|-------------|---------|
| `image` | Validates file is an image (magic bytes) | `#[validate(image)]` |
| `max_file_size(kb)` | Maximum file size in KB | `#[validate(max_file_size(5120))]` |
| `max_dimensions(w, h)` | Maximum image dimensions in pixels | `#[validate(max_dimensions(1920, 1080))]` |
| `min_dimensions(w, h)` | Minimum image dimensions in pixels | `#[validate(min_dimensions(100, 100))]` |
| `allowed_mimes(...)` | Allowed MIME types (magic bytes + content-type fallback) | `#[validate(allowed_mimes("image/jpeg", "image/png"))]` |
| `allowed_extensions(...)` | Allowed file extensions | `#[validate(allowed_extensions("jpg", "png", "webp"))]` |

Uses `infer` crate for magic byte detection (reliable, no content-type header dependency) and `image` crate for dimension parsing (reads headers only, memory-efficient).

### Unified Multipart Extraction

`Validated<T>` auto-detects `Content-Type` and routes to JSON or multipart extraction transparently. One extractor handles both.

```rust
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateProfile {
    #[validate(required, min(2))]
    pub name: String,

    #[validate(image, max_file_size(2048))]
    pub avatar: Option<UploadedFile>,
}

// ONE extractor — works for multipart (with or without files) AND JSON
async fn handler(Validated(input): Validated<UpdateProfile>) -> Json<Value> {
    // name is validated, avatar is validated (image + size checks done)
}
```

**How it works:**
- `Validated<T>` checks `Content-Type: multipart/form-data` header
- Multipart path: uses `FromMultipart` trait (auto-generated by derive) to extract text fields + file fields
- JSON path: standard `Json<T>` deserialization (existing behavior)
- Both paths run the same `RequestValidator` validation after extraction
- `UploadedFile` implements `Deserialize` as always-error (file fields are never JSON-deserialized)

**`FromMultipart` trait:** Automatically generated by `#[derive(Validate)]` for all structs. Handles:
- Text fields (String, i32, etc.) via `FromStr`
- `Option<T>` fields (absent field = None)
- `UploadedFile` fields (streams to temp file, creates `UploadedFile`)
- `Vec<UploadedFile>` fields (accumulates multiple files under same field name)

**Translation file convention** (`locales/{locale}/validation.json`):

```json
{
  "validation": {
    "required": "The {{attribute}} field is required.",
    "email": "The {{attribute}} must be a valid email address.",
    "min": "The {{attribute}} must be at least {{min}} characters.",
    "unique": "The {{attribute}} has already been taken.",
    "attributes": {
      "email": "email address"
    },
    "custom": {}
  }
}
```

---

# 8. auth/

**Status: 🔄 Partially done**

## Responsibilities
- actor
- role
- permission
- policy

### What's Done
- `Actor` struct with id, guard, roles, permissions, claims
- `AuthError` enum (unauthorized, forbidden, internal)
- `BearerAuthenticator` trait
- `Policy` trait for authorization logic
- `AuthManager` with guard system
- `Authorizer` for permission checks
- `StaticBearerAuthenticator` (hardcoded token → actor)
- `CurrentActor` and `OptionalActor` Axum extractors
- Route-level auth via `AccessScope` (guards, permissions)
- `Authenticatable` trait — models declare which guard they back
- `Actor::resolve::<M>(&app)` — resolve actor to its backing model
- `AuthenticatedModel<M>` / `Auth<M>` extractor — one-step model extraction
- `AuthenticatableRegistry` — guard-to-model registry with duplicate guard detection
- `WebSocketContext::resolve_actor::<M>()` convenience
- Guard mismatch validation (User PAT cannot resolve as AdminUser)

### Actor → Model Resolution (Authenticatable)

See **[auth-actor-system](12-auth-actor-system.md)** for the full standalone blueprint covering:
- Guard registration and multi-guard setup
- `Authenticatable` trait and model resolution
- `Auth<M>` / `AuthenticatedModel<M>` extractor DX
- Multi-portal security (User, Admin, Merchant)
- WebSocket integration
- Consumer project structure

### TODO
- **Custom authenticator** — User will implement own auth method (not JWT)
- Login/session auth — session-based authentication flow
- PAT (Personal Access Token) system

---

# 9. events/

**Status: ✅ Done** — Matches blueprint.

## Responsibilities
- event dispatch
- listeners

### Drift
- Typed event listeners
- Job dispatch integration
- WebSocket publish integration

---

# 10. jobs/

**Status: ✅ Core done — Improvements planned**

See **[job-system](13-job-system.md)** for the full standalone blueprint covering planned improvements: worker concurrency, per-job timeout, graceful shutdown, job middleware, batching, chaining, rate limiting, unique jobs, status tracking.

## What's Done
- Job trait with typed ID, configurable retries + backoff
- JobDispatcher (dispatch/dispatch_later)
- Redis backend with Lua scripts for atomic claim/ack/retry
- Memory backend for testing
- Dead letter queue
- Lease-based locking with heartbeat renewal
- Worker kernel
- 3 framework-provided jobs (email, notification, datatable export)

## TODO (Phased — see job blueprint)
- **Phase 1 (Critical):** Worker concurrency, per-job timeout, graceful shutdown
- **Phase 2 (High):** Job middleware (before/after/failed hooks)
- **Phase 3 (Medium):** Batching, chaining, rate limiting, unique jobs
- **Phase 4 (Low):** Job status tracking, dashboard endpoint

---

# 11. config/

**Status: ✅ Done — Exceeds blueprint**

## Responsibilities
- env loading
- config merging

### Drift
Blueprint listed 2 features. Implementation includes:
- TOML config files
- Env variable overlay
- 10 typed config sections (database, redis, server, logging, i18n, etc.)

---
# 12. logging/

**Status: ✅ Done — Exceeds blueprint**

## Responsibilities
- tracing
- request ID

### Drift
Blueprint listed 2 features. Implementation includes:
- Liveness/readiness health probes
- Runtime diagnostics
- Atomic counters per subsystem
- Full observability stack

### Structured Logging System

NDJSON structured logging via `tracing-subscriber` JSON mode with configurable format and file sink.

**Configuration** (`config/logging.toml`):

```toml
[logging]
level = "info"       # trace, debug, info, warn, error
format = "json"      # json (default) or text (dev)
log_dir = "logs"     # default, date-based files: logs/YYYY-MM-DD.log. Set "" for stdout only.

[app]
timezone = "Asia/Kuala_Lumpur"  # log timestamps and file dates use this timezone
```

**Features:**

| Feature | Description |
|---------|-------------|
| NDJSON format | One JSON object per line with `timestamp`, `level`, `target`, `message` + structured fields |
| stdout sink | Default output for containers/cloud logging |
| Date-rotating file sink | `log_dir/YYYY-MM-DD.log`, auto-rotates at midnight using framework `Clock` |
| Timezone-aware timestamps | Log timestamps use the app's configured `[app] timezone` |
| Request logging | `method`, `path`, `status`, `duration_ms`, `request_id` on every request completion |
| Panic hook | Captures panics as structured error events with `target: "foundry.panic"` |
| Kernel events | Structured lifecycle events in worker, scheduler, and websocket kernels |

**Request completion log example:**
```json
{"timestamp":"2026-04-11T21:40:00Z","level":"INFO","target":"foundry::logging::middleware","message":"Request completed","method":"GET","path":"/api/users","status":200,"duration_ms":18,"request_id":"foundry-1"}
```

**Kernel event targets:**

| Target | Events |
|--------|--------|
| `foundry.worker` | Job succeeded, job dead-lettered |
| `foundry.scheduler` | Schedule executed, leadership acquired/lost |
| `foundry.websocket` | Connection opened/closed |
| `foundry.panic` | Thread panic with location and error |

**Module structure:**
```text
src/logging/
├── mod.rs           — Re-exports, init(), LogFormat enum
├── types.rs         — LogLevel, outcome enums
├── request_id.rs    — RequestId, generation, Axum extractor
├── diagnostics.rs   — RuntimeDiagnostics, atomic counters, snapshots
├── probes.rs        — ReadinessCheck, ProbeResult, registries
├── observability.rs — ObservabilityOptions, health routes
├── middleware.rs    — Request context middleware with duration tracking
├── file_writer.rs   — FileWriter implementing MakeWriter for file sink
```

---

# 13. database/

**Status: ✅ Done — MASSIVELY exceeds blueprint**

## Responsibilities
- connection
- transaction helpers

### Drift
Blueprint listed 2 features. Implementation is the most sophisticated module in the framework:

**Query System:**
- Full AST-based query builder (Select, Insert, Update, Delete)
- PostgreSQL compiler
- CTEs, window functions, JSON operations, set operations
- Parameterized queries with `DbValue`

**ORM:**
- `Model` trait + `#[derive(Model)]` macro
- Safe-by-default `ModelId<M>` UUIDv7 primary keys serialized as strings, with `primary_key_strategy = "manual"` for explicit opt-out
- `PersistedModel` trait for saved models
- `CreateDraft` / `UpdateDraft` patterns
- Default model timestamps with per-model opt-out, plus opt-in soft deletes with `with_trashed()`, `only_trashed()`, `restore()`, and `force_delete()`
- Model lifecycle hooks (creating, created, updating, updated, deleting, deleted)
- Field-level write mutators via `#[foundry(write_mutator = \"...\")]` on model fields
- Explicit generated read accessor methods via `#[foundry(read_accessor = \"...\")]`, e.g. `password_accessed()`
- `ModelQuery` fluent API with model-first `create()/update()/delete()` builders

**Model Output Direction:**
- DTO-first response/output design; models are not the framework resource layer
- No Laravel-style `hidden` field metadata on models today
- Read accessors are explicit generated methods, not magic field interception or automatic JSON rewriting

**Relations:**
- `has_many`, `has_one`, `belongs_to`, `many_to_many`
- Eager loading with `load()` and `load_missing()`
- Nested eager loading (`.with()`, `.with_many_to_many()`)
- Relation aggregates (`.count()`, `.sum()`, `.avg()`, `.min()`, `.max()`)
- `Loaded<T>` enum for type-safe relation state
- `with_pivot()` for many-to-many with pivot data

**Query Features:**
- `Projection` for custom read models
- `Paginated<T>` with `Collection<T>`
- Streaming query results
- CASE expressions, JSON path queries
- Lock clauses (FOR UPDATE, FOR SHARE)
- Upsert (ON CONFLICT)

**Schema Management:**
- Build-time discovered Rust migration files with raw-SQL-first `up()` / `down()` methods
- Build-time discovered Rust seeder files with raw-SQL-first `run()` methods
- CLI commands for `make:migration`, `make:seeder`, `db:migrate`, `db:rollback`, and `db:seed`
- Postgres 18+ baseline for `uuidv7()` defaults in SQL-authored schemas

**Collection Integration:**
- `ModelCollectionExt` trait on `Collection<T>`
- `model_keys()`, `load()`, `load_missing()`

---

# 14. email/

**Status: ✅ Done — Matches blueprint**

Multi-mailer outbound email system with SMTP and log drivers, custom driver registration, and queue integration.

## Responsibilities
- multi-mailer email management
- immediate and queued email delivery
- message composition with builder pattern
- file attachments (path and storage-backed)
- custom email driver registration

## Key Components

### EmailManager
- Resolved from `AppContext::email()`
- Config-driven default mailer resolution
- Named mailer lookup
- Custom driver registration via `ServiceRegistrar::register_email_driver()`
- Convenience methods: `send()`, `queue()`, `queue_later()` delegate to default mailer

### EmailMailer
- Cheap-clone handle to a resolved mailer driver
- Resolves sender address: message `from` > config `email.from` > error
- Resolves storage-backed attachments
- `send()`, `queue()`, `queue_later()` methods

### EmailDriver (trait)
- `send(&OutboundEmail) -> Result<()>`
- Built-in drivers: SMTP (lettre), Log (tracing)

### EmailMessage
- Builder pattern: `EmailMessage::new("Subject").to("user@example.com").text_body("Hello")`
- Serializable for queue delivery

### Built-in Drivers
- **SmtpEmailDriver** — lettre 0.11 with rustls (STARTTLS/TLS)
- **LogEmailDriver** — structured tracing output
- **ResendEmailDriver** — Resend API (`reqwest`)
- **PostmarkEmailDriver** — Postmark API (`reqwest`)
- **MailgunEmailDriver** — Mailgun API (`reqwest`, multipart)
- **SesEmailDriver** — AWS SES via HTTP with SigV4 signing (`reqwest` + `hmac`/`sha2`)

### Queue Integration
- `SendQueuedEmailJob` — Foundry job for queued email delivery
- Reuses existing job infrastructure (retry, backoff, dead-letter)
- Registered automatically by framework

## Config Section

```toml
[email]
default = "smtp"
queue = "default"

[email.from]
address = "hello@example.com"
name = "Foundry App"

[email.mailers.smtp]
driver = "smtp"
host = "127.0.0.1"
port = 587
username = ""
password = ""
encryption = "starttls"
timeout_secs = 30

[email.mailers.log]
driver = "log"
target = "email.outbound"

[email.mailers.resend]
driver = "resend"
api_key = "${RESEND_API_KEY}"

[email.mailers.postmark]
driver = "postmark"
server_token = "${POSTMARK_SERVER_TOKEN}"

[email.mailers.mailgun]
driver = "mailgun"
domain = "mg.example.com"
api_key = "${MAILGUN_API_KEY}"
region = "us"

[email.mailers.ses]
driver = "ses"
key = "${AWS_ACCESS_KEY_ID}"
secret = "${AWS_SECRET_ACCESS_KEY}"
region = "ap-southeast-1"
```

## Usage Examples

```rust
// Default mailer send
let email = app.email()?;
email.send(
    EmailMessage::new("Welcome")
        .to("user@example.com")
        .text_body("Your account is ready.")
).await?;

// Named mailer + queued
app.email()?
    .mailer("marketing")?
    .queue(
        EmailMessage::new("Export Ready")
            .to("ops@example.com")
            .text_body("The export is ready.")
    ).await?;

// With attachments
app.email()?
    .send(
        EmailMessage::new("Report")
            .to("user@example.com")
            .text_body("See attached.")
            .attach(EmailAttachment::from_storage("s3", "reports/quarterly.pdf"))
    ).await?;
```

## Module Structure

```text
src/email/
├── mod.rs           — EmailManager, registry builder, re-exports
├── adapter.rs       — (N/A — uses driver.rs)
├── config.rs        — EmailConfig, ResolvedSmtpConfig, ResolvedLogConfig
├── driver.rs        — EmailDriver trait, OutboundEmail
├── address.rs       — EmailAddress value type
├── message.rs       — EmailMessage builder
├── attachment.rs    — EmailAttachment enum, ResolvedAttachment
├── mailer.rs        — EmailMailer handle
├── job.rs           — SendQueuedEmailJob
├── smtp.rs          — SmtpEmailDriver (lettre)
├── log.rs           — LogEmailDriver (tracing)
```

---

# 15. storage/

**Status: ✅ Done — Matches blueprint**

Multi-disk file storage system with local and S3 backends, upload helpers, and HTTP multipart extractors.

## Responsibilities
- multi-disk storage management
- file put/get/delete/copy/move operations
- upload handling with UUIDv7 naming
- multipart form extraction

## Key Components

### StorageManager
- Resolved from `AppContext::storage()`
- Config-driven default disk resolution
- Named disk lookup
- Custom driver registration via `ServiceRegistrar::register_storage_driver()`

### StorageDisk
- Cheap-clone handle to a resolved disk adapter
- Delegates all operations to the underlying adapter

### StorageAdapter (trait)
- `put_bytes`, `put_file`, `get`, `delete`, `exists`, `copy`, `move_to`, `url`, `temporary_url`
- `StorageVisibility` enum: `Public`, `Private`

### Built-in Adapters
- **LocalStorageAdapter** — `tokio::fs` based, auto-creates parent directories, cross-device move fallback
- **S3StorageAdapter** — `object_store` (Apache Arrow) based, supports AWS S3 and S3-compatible services (MinIO, Cloudflare R2)

### Upload Helpers
- `UploadedFile` struct with `store()`, `store_on()`, `store_as()`, `store_as_on()` methods
- UUIDv7 filename generation with safe extension normalization
- Axum `FromRequest` extractor for single-file uploads
- `MultipartForm` extractor for multi-field multipart handling

## Config Section

```toml
[storage]
default = "local"

[storage.disks.local]
driver = "local"
root = "storage/app"
visibility = "private"

[storage.disks.s3]
driver = "s3"
bucket = "foundry-prod"
region = "ap-southeast-1"
key = "${AWS_ACCESS_KEY_ID}"
secret = "${AWS_SECRET_ACCESS_KEY}"
visibility = "private"
```

## Usage Examples

```rust
// Manager-first
let storage = app.storage()?;
storage.default_disk()?.put_bytes("avatars/a.txt", b"hello").await?;
storage.disk("s3")?.put_bytes("reports/x.csv", bytes).await?;

// Upload helper
let stored = upload.store(&app, "avatars").await?;
let stored = upload.store_on(&app, "s3", "avatars").await?;
```

## Module Structure

```text
src/storage/
├── mod.rs           — StorageManager, registry builder, re-exports
├── adapter.rs       — StorageAdapter trait, StorageVisibility
├── config.rs        — StorageConfig, ResolvedLocalConfig, ResolvedS3Config
├── disk.rs          — StorageDisk handle
├── stored_file.rs   — StoredFile struct
├── local.rs         — LocalStorageAdapter (tokio::fs)
├── s3.rs            — S3StorageAdapter (object_store)
├── upload.rs        — UploadedFile with store helpers + FromRequest
├── multipart.rs     — MultipartForm extractor
```

---

# 16. support/

**Status: ✅ Done — Exceeds blueprint**

## Responsibilities
- utilities

### Drift
Blueprint listed "utilities" generically. Implementation includes:
- **`Collection<T>`** — 30+ methods: map, filter, reject, flat_map, find, first_where, pluck, key_by, group_by, unique_by, partition_by, chunk, sort_by, sum_by, min_by, max_by, take, skip, for_each, tap, pipe
- **Semantic IDs** — type-safe identifiers plus generic `ModelId<M>` UUIDv7 wrappers for model primary keys
- **`RuntimeBackend`** — Redis + in-memory backends with `incr_with_ttl` for rate limiting
- **`HashManager`** — Argon2id password hashing with config-driven parameters, `hash()` + `check()`
- **`CryptManager`** — AES-256-GCM encryption for data at rest, `encrypt()` + `decrypt()`, optional (graceful degradation when key not configured)
- **`Token`** — Cryptographically secure random generation: alphanumeric strings, hex, base64, raw bytes

### HashManager

Config-driven password hashing via Argon2id (OWASP recommended).

**Config** (`config/hashing.toml`):
```toml
[hashing]
driver = "argon2"
memory_cost = 19456  # 19 MiB
time_cost = 2
parallelism = 1
```

**Usage:**
```rust
let hash = app.hash()?.hash("secret-password")?;
let valid = app.hash()?.check("secret-password", &hash)?;
```

Typical model-layer usage is through a field write mutator:

```rust
impl User {
    async fn hash_password(ctx: &ModelHookContext<'_>, value: String) -> Result<String> {
        ctx.app().hash()?.hash(&value)
    }
}
```

### CryptManager

AES-256-GCM encryption for sensitive data at rest. Optional — only registered when `[crypt] key` is configured.

**Config** (`config/app.toml`):
```toml
[crypt]
key = "${APP_KEY}"  # base64-encoded 32-byte key
```

**Usage:**
```rust
let encrypted = app.crypt()?.encrypt_string("sensitive data")?;
let decrypted = app.crypt()?.decrypt_string(&encrypted)?;
```

### Token

Standalone utility — no config, no container. Pure functions.

```rust
let api_key = Token::generate(32)?;     // alphanumeric
let hex_token = Token::hex(16)?;        // 32 hex chars
let b64_token = Token::base64(32)?;     // URL-safe base64
```

---

# 17. redis/

**Status: ✅ Done — Exceeds blueprint**

## Responsibilities
- app-facing Redis access
- namespace-safe key and channel construction
- low-level Redis operations without exposing an unsafe raw client

### Drift
The original blueprint only mentioned Redis indirectly through runtime features. Implementation now includes a dedicated public Redis surface:
- **`RedisManager`** — resolved from `AppContext::redis()`
- **`RedisKey` / `RedisChannel`** — typed namespaced wrappers
- **`RedisConnection`** — low-level Redis primitives (`get`, `set`, `set_ex`, `del`, `exists`, `expire`, `incr`, `publish`, `hget`, `hset`, `sadd`, `srem`, `smembers`)
- Default keys/channels use the app `redis.namespace`
- Rare cross-project integration is possible through explicit alternate-namespace helpers instead of disabling namespacing globally

---

# 18. i18n/

**Status: ✅ Done — Matches blueprint**

Internationalization system sharing the exact same translation JSON files with React/i18next frontend.

## Responsibilities
- translation catalog loading and lookup
- per-request locale resolution
- `{{variable}}` interpolation (i18next-compatible)

## Key Components

### I18nManager
- Loaded once at startup from `config/i18n.toml`
- Scans `{resource_path}/{locale}/*.json`, merges all files per locale into one catalog
- Supports nested JSON flattening (`errors.not_found` dot notation)
- Three-tier fallback: requested locale → fallback locale → key itself
- Warns on duplicate keys during load

### Config Section (`[i18n]`)

```toml
[i18n]
default_locale = "en"
fallback_locale = "en"
resource_path = "locales"
```

### Axum Extractor (`I18n`)

```rust
use foundry::prelude::*;
use foundry::t;

async fn handler(i18n: I18n) -> String {
    // No parameters
    t!(i18n, "Something went wrong")

    // Named parameters — order doesn't matter
    t!(i18n, "Hello {{name2}} and {{name}}", name2 = "Alice", name = "Bob")
}
```

**`t!` macro** provides clean named-parameter syntax matching the blueprint's `t("key", values! { ... })` style. The `t()` and `t_with()` methods are also available for dynamic use cases.

### Locale Detection
- Checks `Accept-Language` header (splits by `,`, strips quality values)
- Falls back to `default_locale` from config
- Custom middleware can set `Locale` extension before extractor runs

### Drift
Blueprint listed builder methods (`register_locales()`, `default_locale()`, etc.). Implementation is config-driven instead — all settings via `config/i18n.toml`, locales auto-discovered from filesystem. No builder methods needed.

### Graceful Degradation
When `[i18n]` config section is absent:
- `I18nManager` not registered in container
- `I18n` extractor returns a no-op that returns keys as-is
- Zero breaking changes to existing applications

---

# Project Structure (Consumer)

```text
my-app/
├── bootstrap/
│   ├── app.rs
│   ├── http.rs
│   ├── cli.rs
│   ├── scheduler.rs
│   └── websocket.rs
├── app/
│   ├── domains/
│   ├── use_cases/
│   ├── portals/
│   ├── providers/
│   ├── commands/
│   ├── schedules/
│   └── mod.rs
├── config/
│   ├── i18n.toml
│   └── ...
├── locales/
│   ├── en/
│   │   ├── common.json
│   │   └── validation.json
│   └── ms/
│       ├── common.json
│       └── validation.json
└── main.rs
```

---

# Registration System (Key Feature)

## What can be registered

- routes
- websocket routes
- commands
- cron jobs
- validation rules
- event listeners
- service providers
- middleware

---

# Example Registration

```rust
app.register_routes(router);
app.register_command(MyCommand);
app.register_schedule(schedule_fn);
app.register_validation_rule("phone", PhoneRule);
```

---

# Key Design Principles

1. Thin application layer
2. Strong framework kernel
3. Clear separation of concerns
4. Extensible via providers
5. Registry-driven system

---

# Long-Term Evolution

## Phase 1
- HTTP ✅
- Validation ✅
- CLI ✅
- Scheduler ✅

## Phase 2
- WebSocket ✅
- Events ✅
- Jobs ✅

## Phase 3
- Plugin system ✅ (implemented early)
- I18n system ✅ (implemented early — i18next-compatible, shared frontend+backend)
- Distributed job system ✅ Core done (Redis + memory backends, retry, dead-letter) — improvements planned in job blueprint
- Observability tools ✅ (NDJSON structured logging, file sink, request duration, panic hook, kernel events, health probes)

---

# Blueprint Gaps — Not in Original Blueprint But Now Needed

These were not in the original blueprint but are necessary for a production framework:

### Structured Error Types ✅ Done
- `Error` enum: `Message`, `Http`, `NotFound`, `Other`
- Consistent JSON responses: `{"message": "...", "status": N}`
- `From<ValidationErrors>` and `From<AuthError>` conversions

### Testing Utilities ✅ Done
- TestApp builder (simplified bootstrap without server)
- TestClient (HTTP requests directly to router, no TCP)
- TestResponse (json/text/status/header helpers)
- Factory trait + FactoryBuilder for model test data

### HTTP Middleware System ✅ Done
- CORS, Security Headers, Rate Limiting (Redis + in-memory), Body Size Limit, Timeout, Trusted Proxy (Cloudflare)
- `register_middleware()` on AppBuilder for global middleware
- `HttpRouteOptions::middleware()` for per-route middleware
- Foundry applies middleware in correct security order internally via priority system
- Each middleware: builder pattern with `.build()` → `MiddlewareConfig`
- Redis-backed rate limiting with `INCR` + `EXPIRE`, automatic when Redis is configured

### I18n System ✅ Done
- `I18nManager` — loads i18next-compatible JSON at startup, per-locale catalogs merged from multi-file
- `I18n` Axum extractor — per-request locale from `Accept-Language`, `t()` and `t_with()` API
- `t!` macro — named-parameter syntax: `t!(i18n, "Hello {{name}}", name = "WeiLoon")`
- `I18nConfig` section — `default_locale`, `fallback_locale`, `resource_path`
- Three-tier fallback chain: requested locale → fallback locale → key itself
- `{{variable}}` interpolation matching i18next format
- Nested JSON flattening (dot-notation keys)
- Graceful degradation when not configured (returns keys as-is)
- Config-driven (auto from `config/i18n.toml`), locales auto-discovered from filesystem

---

# Progress Summary

| Module | Blueprint | Actual | Status |
|--------|-----------|--------|--------|
| foundation/ | App, Builder, Container, ServiceProvider | Full DI, lifecycle, transactions, plugin, structured errors | ✅ Exceeds |
| kernel/ | HTTP, CLI, Scheduler, WebSocket | All 4 + Worker kernel | ✅ Exceeds |
| http/ | Routing, middleware, guards, SPA | Axum-based, route options, auth middleware, 8 middleware types (CORS, SecurityHeaders, CSRF, RateLimit w/ per-user, MaxBodySize, Timeout, TrustedProxy), SPA serving, cookie helpers | ✅ Exceeds |
| websocket/ | Connection, channels, routing | Pub/sub, rooms, auth, Redis backend | ✅ Exceeds |
| scheduler/ | Cron, interval, registry | Distributed leadership with Redis | ✅ Exceeds |
| cli/ | Commands, arg parsing | Clap integration, command registry | ✅ Done |
| validation/ | Built-in rules, custom rules, chainable API, derive macro | 36 rules (30 text + 6 file), modifiers, request validator, async custom rules, translation-aware messages, custom messages/attributes, `#[derive(Validate)]` proc macro, `FromMultipart` auto-generation, unified JSON/multipart extraction | ✅ Exceeds |
| auth/ | Actor, role, permission, policy, authenticatable, token, session | Authenticatable, Auth<M> extractor, multi-guard, TokenManager (PAT + refresh rotation), SessionManager (Redis-backed), config-driven guard drivers | ✅ Done |
| events/ | Dispatch, listeners | EventBus, typed listeners, job dispatch | ✅ Done |
| jobs/ | Background jobs, queue | Redis + memory, retry, dead-letter, leasing | ✅ Exceeds |
| config/ | Env loading, config merging | TOML + env overlay, 11 typed sections | ✅ Exceeds |
| logging/ | Tracing, request ID | NDJSON structured logging, file sink, request duration, panic hook, kernel events, health probes, diagnostics | ✅ Exceeds |
| database/ | Connection, transaction helpers | Full ORM: AST query builder, Model, relations, migrations | ✅ MASSIVELY exceeds |
| email/ | Multi-mailer email | SMTP + log + Resend + Postmark + Mailgun + SES drivers, queue integration, custom drivers, message builder | ✅ Done |
| storage/ | Multi-disk storage | StorageManager, local + S3 adapters, upload helpers, multipart extractors, custom drivers | ✅ Done |
| support/ | Utilities | Collection<T>, semantic IDs, RuntimeBackend, HashManager (argon2), CryptManager (AES-256-GCM), Token generation, SHA-256, sanitize_html/strip_tags | ✅ Exceeds |
| redis/ | App-facing Redis API | Namespaced RedisManager, RedisKey, RedisChannel, RedisConnection | ✅ Done |
| plugin/ | (Phase 3) | Dependency resolution, assets, scaffolding, CLI | ✅ Done early |
| i18n/ | (Not in original) | I18nManager, i18next JSON, per-request locale, `t!` macro, Axum extractor | ✅ Done |
| app_enum/ | (Not in original) | `#[derive(AppEnum)]` with FoundryAppEnum trait, ToDbValue/FromDbValue, Serialize/Deserialize, validation + derive integration, model integration, aliases, metadata | ✅ Done |

---

# Remaining Gaps

See **[framework-gaps](15-framework-gaps.md)** for the gap-fill blueprint (✅ complete).

See **[production-hardening](16-production-hardening.md)** for production hardening (✅ complete): structured errors, DB pool lifecycle, distributed locking, maintenance mode, signed routes.

See **[framework-models-image](17-framework-models-image-module.md)** for framework models & image module (✅ complete): image processing, attachments, metadata, translations, HasToken, migrate:publish.

# Completed Priority Items

1. ~~**HTTP Middleware System**~~ — ✅ Done (CORS, Security Headers, Rate Limit with Redis, Body Size, Timeout, Trusted Proxy, per-route middleware)
2. ~~**Redis Rate Limiting**~~ — ✅ Done (automatic when Redis is configured)
3. ~~**Per-Route Middleware**~~ — ✅ Done (via `HttpRouteOptions::middleware()`)
4. ~~**I18n System**~~ — ✅ Done (I18nManager, i18next-compatible JSON, per-request locale, Axum extractor, config-driven)
5. ~~**Structured Logging System**~~ — ✅ Done (NDJSON format, file sink, request duration, panic hook, kernel lifecycle events)
6. ~~**Authenticator (non-JWT)**~~ — ✅ Done (TokenManager PAT + SessionManager, config-driven guard drivers, Auth<M> extractor)
7. ~~**Testing Utilities**~~ — ✅ Done (TestApp, TestClient, TestResponse, Factory)
8. ~~**Distributed Job System**~~ — ✅ Core done (improvements planned in job blueprint)
9. ~~**Storage System**~~ — ✅ Done (multi-disk, local + S3, upload helpers, multipart extractors)
10. ~~**Email System**~~ — ✅ Done (multi-mailer, SMTP + log drivers, queue integration, custom drivers, message builder)
11. ~~**Security Utilities (Hash/Crypt/Token)**~~ — ✅ Done (HashManager argon2, CryptManager AES-256-GCM, Token generation)
12. ~~**App Enum System**~~ — ✅ Done (`#[derive(AppEnum)]`, FoundryAppEnum trait, ToDbValue/FromDbValue, Serialize/Deserialize, validation + model integration, aliases, metadata)
13. ~~**File Validation + Unified Multipart Extraction**~~ — ✅ Done (6 file rules in derive, `FromMultipart` auto-generation, `Validated<T>` multipart detection, `UploadedFile` Deserialize support)
14. ~~**Framework Gaps**~~ — ✅ Done (pagination response, cursor pagination, API resources, model scopes, named routes, middleware groups, ETag, password reset, email verification, remember me, SQL logging, mail templates, read replicas, job priorities)
15. ~~**Production Hardening**~~ — ✅ Done (structured error responses, DB pool lifecycle, distributed locking, maintenance mode, signed routes, config:publish, key:generate)
16. ~~**Image Module**~~ — ✅ Done (ImageProcessor with resize/crop/convert/blur/grayscale/rotate/flip/brightness/contrast, 8 formats)
17. ~~**Framework Models**~~ — ✅ Done (Attachments with upload pipeline, Metadata key-value store, Translations with locale-aware resolution)
18. ~~**HasToken Trait**~~ — ✅ Done (Laravel HasApiTokens — create_token, create_token_named, create_token_with_abilities, revoke_all_tokens)
19. ~~**CLI Publisher Commands**~~ — ✅ Done (config:publish, key:generate, migrate:publish, seed:publish, seed:countries)
20. ~~**Countries Table**~~ — ✅ Done (250 countries from ISO 3166-1, iso2 primary key, Country model with find/all/enabled/exists, seed:countries CLI)
21. ~~**Framework Migrations & Seeders**~~ — ✅ Done (9 sequential .rs migrations at `database/migrations/000000000001-9_*.rs`, 1 seeder at `database/seeders/`, consumer examples removed)
22. ~~**Scheduler Hardening**~~ — ✅ Done (error isolation per task, parallel spawning, overlap prevention via distributed lock, per-task ScheduleOptions with without_overlapping/environments/before/after/on_failure hooks, convenience methods: every_minute/hourly/daily/daily_at/weekly)
23. ~~**CLI Hardening**~~ — ✅ Done (root command with --version/--help/subcommand_required, `about` command for environment summary, `routes:list` command for named route inspection. Total: 21 builtin commands)

---

# Final Summary

This framework aims to:

- centralize infrastructure
- standardize backend patterns
- reduce boilerplate
- enforce clean architecture

---

# Final Statement

> Build once in the framework, reuse everywhere in projects.

> Project = configuration + registration
> Framework = execution + orchestration
