# Foundry Framework Improvements Blueprint

> **Status:** Active planning
> **Created:** 2026-04-12
> **Purpose:** Roadmap for bringing Foundry to production-readiness with security, testing, DX, and advanced features before building the reference sample project (Foundry Commerce).

---

# Current State Summary

Foundry has 21 modules implemented covering: foundation, 5 kernels, HTTP (6 middleware types), WebSocket, scheduler (distributed), CLI, validation (36 rules), auth (multi-guard + token + session), events, jobs (Redis/memory), config, logging (NDJSON + probes), database (full ORM), email (6 drivers), storage (local + S3), support (Collection, crypto), Redis, plugin, i18n, app_enum, datatable.

**What's missing for production 2026:** Security hardening (CSRF, sanitization, per-user rate limiting), testing infrastructure, caching abstraction, DX tooling, notifications, observability, and API polish.

---

# Tier 1: Security + Testing (Critical — Build Before Sample Project)

## 1.1 CSRF Middleware

**Status: Done**

Double-submit cookie pattern. Framework generates a CSRF token, stores it in a cookie, and validates that the request includes a matching token in a header or form field.

### Config

```toml
[security]
csrf_cookie_name = "foundry_csrf"
csrf_header_name = "X-CSRF-TOKEN"
csrf_form_field = "_token"
csrf_ttl_minutes = 120
csrf_secure = true
```

### Consumer DX

```rust
// Register middleware globally for session-based portals
App::builder()
    .register_middleware(Csrf::new().build())
    .run_http()?;

// Or per-route group
r.route_with_options("/admin/users", post(create_user),
    HttpRouteOptions::new()
        .guard(AuthGuard::Admin)
        .middleware(Csrf::new().build()),
);

// Extractor in handler to get current token (for rendering in forms/meta tags)
async fn form(csrf: CsrfToken) -> impl IntoResponse {
    Html(format!("<input type='hidden' name='_token' value='{}'/>", csrf.value()))
}
```

### Internal Design

- New file: `src/http/middleware/csrf.rs` (or within existing `middleware.rs`)
- Add `Csrf` variant to `MiddlewareConfig` enum with priority 25 (after SecurityHeaders, before RateLimit)
- `CsrfToken` extractor reads token from cookie, generates if absent
- Middleware validates POST/PUT/PATCH/DELETE requests have matching token in header or form body
- Exempt paths configurable: `.exclude("/api/*")` (API routes use bearer tokens, not CSRF)
- Token generation reuses `Token::base64(32)` from support module

### Module Shape

```text
src/http/middleware.rs (modify)
  - Add Csrf variant to MiddlewareConfig
  - Add CsrfToken extractor
  - CSRF validation middleware function
```

---

## 1.2 HTML Sanitization Utility + SPA Serving

**Status: Done**

### Philosophy

Foundry is API-first (JSON responses). No server-side HTML rendering, no template engine.

- **SQL injection:** Already safe — parameterized queries via `DbValue`
- **XSS in JSON:** Not a risk — JSON is data, not executable HTML
- **Mass assignment:** Already safe — explicit `.set(Column, value)`
- **trim/lowercase:** Use Rust stdlib — `body.email.trim().to_lowercase()`, no framework wrapper needed

**What the framework provides:**
1. `sanitize_html()` — one utility function for rich text CMS fields that will be rendered via `v-html` on the frontend
2. `serve_spa()` — serve the SPA frontend alongside the API

### sanitize_html Utility

```rust
use foundry::support::sanitize_html;

// Admin CMS — allow formatting tags, strip XSS vectors
let safe_body = sanitize_html(
    &body.content,
    &["b", "i", "em", "strong", "p", "br", "a", "ul", "ol", "li",
      "h1", "h2", "h3", "img", "table", "tr", "td", "th"],
);
// Strips: <script>, <iframe>, onclick=, onload=, javascript:, style=
// Keeps: allowed tags with safe attributes (href, src, alt, class)

// User comment — strip ALL HTML
let safe_comment = sanitize_html(&body.comment, &[]);
```

### SPA Serving

```rust
App::builder()
    .register_routes(api::routes)         // /api/* → JSON
    .serve_spa("dist/")                   // Everything else → SPA (serves index.html as fallback)
    .run_http()?;
```

Internally: Axum's `ServeDir` from `tower-http` with fallback to `index.html` for client-side routing.

### Module Shape

```text
src/support/sanitize.rs (new)
  - pub fn sanitize_html(input: &str, allowed_tags: &[&str]) -> String
  - pub fn strip_tags(input: &str) -> String
src/http/spa.rs (new)
  - SPA serving middleware (ServeDir + index.html fallback)
```

---

## 1.3 Per-User Rate Limiting

**Status: Done**

Extend existing `RateLimit` middleware to support keying by authenticated actor ID instead of (or in addition to) IP address.

### Consumer DX

```rust
// Rate limit by actor (authenticated user)
r.route_with_options("/api/orders", post(create_order),
    HttpRouteOptions::new()
        .guard(AuthGuard::Api)
        .middleware(RateLimit::new(60).per_minute().by_actor().build()),
);

// Rate limit by IP (default, existing behavior)
r.route_with_options("/auth/login", post(login),
    HttpRouteOptions::new()
        .middleware(RateLimit::new(5).per_minute().build()),  // by_ip is default
);

// Combined: per-IP for anonymous, per-actor for authenticated
r.route_with_options("/api/search", get(search),
    HttpRouteOptions::new()
        .guard(AuthGuard::Api)
        .middleware(RateLimit::new(100).per_minute().by_actor_or_ip().build()),
);
```

### Internal Design

- Add `RateLimitBy` enum: `Ip`, `Actor`, `ActorOrIp`
- Add `by_actor()`, `by_actor_or_ip()` builder methods to `RateLimit`
- In the rate limit middleware, extract key from `Actor` extension (if present) or fall back to IP
- Redis key format: `rl:{prefix}:actor:{actor_id}:{window}` or `rl:{prefix}:ip:{ip}:{window}`

### Module Shape

```text
src/http/middleware.rs (modify)
  - Add RateLimitBy enum
  - Add by_actor()/by_actor_or_ip() to RateLimit builder
  - Update rate_limit_middleware to extract actor key
```

---

## 1.4 Test Helpers

**Status: Done** — TestApp, TestClient, TestResponse in src/testing/client.rs

Provide a first-class testing experience: bootstrap a test app, make HTTP requests, manage test database transactions.

### Consumer DX

```rust
use foundry::testing::*;

#[tokio::test]
async fn test_create_user() {
    let app = TestApp::builder()
        .register_provider(AppServiceProvider)
        .register_routes(api::routes)
        .with_database()    // auto-migrate, auto-rollback
        .build().await;

    // Seed data using factories
    let admin = app.factory::<AdminUser>().create().await;

    // Make HTTP requests without starting a server
    let response = app.client()
        .post("/api/users")
        .bearer_auth(&admin_token)
        .json(&json!({ "email": "new@example.com", "name": "New User" }))
        .send().await;

    assert_eq!(response.status(), 201);
    let user: User = response.json().await;
    assert_eq!(user.email, "new@example.com");

    // Assert database state
    app.assert_database_has::<User>("email", "new@example.com").await;
}

#[tokio::test]
async fn test_login_returns_token_pair() {
    let app = TestApp::builder()
        .register_provider(AppServiceProvider)
        .with_database()
        .build().await;

    let user = app.factory::<User>()
        .state("password", "secret123")
        .create().await;

    let response = app.client()
        .post("/auth/login")
        .json(&json!({ "email": user.email, "password": "secret123" }))
        .send().await;

    assert_eq!(response.status(), 200);
    let pair: TokenPair = response.json().await;
    assert!(!pair.access_token.is_empty());
}
```

### Components

| Component | Purpose |
|-----------|---------|
| `TestApp` | Bootstraps app with test config, manages lifecycle |
| `TestClient` | HTTP client that sends requests to the app's router directly (no TCP) |
| `TestResponse` | Response wrapper with `.json()`, `.text()`, `.status()`, `.header()` |
| `DatabaseTestTransaction` | Wraps all test queries in a transaction, auto-rolls back |

### Module Shape

```text
src/testing/mod.rs (new)
  - TestApp builder
  - TestClient
  - TestResponse
src/testing/database.rs (new)
  - DatabaseTestTransaction
  - assert_database_has / assert_database_missing helpers
```

---

## 1.5 Model Factories

**Status: Done** — Factory trait, FactoryBuilder in src/testing/factory.rs

Define model factories with default values for testing. Inspired by Laravel's factory pattern.

### Consumer DX

```rust
// Define a factory for User
impl Factory for User {
    fn definition() -> FactoryDefinition<Self> {
        FactoryDefinition::new()
            .set(User::EMAIL, || format!("user-{}@example.com", Token::hex(4).unwrap()))
            .set(User::NAME, || "Test User".to_string())
            .set(User::PASSWORD, || "hashed_password".to_string())
            .set(User::ACTIVE, || true)
    }
}

// Usage in tests
let user = app.factory::<User>().create().await;                      // defaults
let admin = app.factory::<User>().state("active", false).create().await; // override
let users = app.factory::<User>().count(10).create().await;           // bulk
let draft = app.factory::<User>().make();                              // in-memory only
```

### Module Shape

```text
src/testing/factory.rs (new)
  - Factory trait
  - FactoryDefinition builder
  - FactoryBuilder (count, state, create, make)
```

---

## 1.6 Cache Layer

**Status: Done**

Generic cache abstraction with multiple drivers. Provides `get/put/remember/forget` with TTL support.

### Consumer DX

```rust
// Access cache via app context
let cache = app.cache()?;

// Basic get/put
cache.put("user:123", &user, Duration::from_secs(300)).await?;
let user: Option<User> = cache.get("user:123").await?;

// Remember pattern (get or compute + cache)
let user: User = cache.remember("user:123", Duration::from_secs(300), || async {
    User::query().where_(User::ID.eq(123)).first(&app).await
}).await?;

// Forget
cache.forget("user:123").await?;

// Tags (invalidate a group)
cache.tags(["users"]).put("user:123", &user, Duration::from_secs(300)).await?;
cache.tags(["users"]).flush().await?;  // invalidate all user-tagged entries
```

### Config

```toml
[cache]
default = "redis"

[cache.stores.redis]
driver = "redis"
prefix = "cache:"
ttl_seconds = 3600

[cache.stores.memory]
driver = "memory"
max_entries = 10000
```

### Drivers

| Driver | Use case |
|--------|----------|
| `redis` | Production — uses existing RedisManager |
| `memory` | Development/testing — in-process HashMap with TTL |

### Module Shape

```text
src/cache/mod.rs (new)
  - CacheManager
  - CacheStore trait
  - TaggedCache wrapper
src/cache/redis.rs (new)
  - RedisCacheStore
src/cache/memory.rs (new)
  - MemoryCacheStore
```

---

# Tier 2: DX + Polish (Build After Tier 1, Before/During Sample)

## 2.1 CLI Scaffolding Commands

**Status: Done** — make:model, make:job, make:command added to lifecycle.rs

Built-in `make:*` commands that generate boilerplate files with correct structure.

### Commands

| Command | Generates |
|---------|-----------|
| `make:model User` | `src/app/models/user.rs` with Model derive, basic fields |
| `make:migration create_users` | `database/migrations/YYYYMMDDHHNN_create_users.rs` |
| `make:job SendWelcomeEmail` | `src/app/jobs/send_welcome_email.rs` with Job impl |
| `make:command CleanupExpired` | `src/app/commands/cleanup_expired.rs` with Command impl |
| `make:provider PaymentProvider` | `src/app/providers/payment.rs` with ServiceProvider impl |
| `make:factory UserFactory` | `src/app/factories/user.rs` with Factory impl |

### Consumer DX

```bash
cargo run -- make:model Product --migration --factory
# Creates:
#   src/app/models/product.rs
#   database/migrations/202604121500_create_products.rs
#   src/app/factories/product.rs
```

---

## 2.2 Graceful Shutdown

**Status: Done** — SIGTERM/SIGINT handling via shutdown_signal() in kernel/shutdown.rs, HTTP server uses with_graceful_shutdown

Handle SIGTERM/SIGINT gracefully: drain in-flight HTTP requests, finish current job, release scheduler leadership.

### Internal Design

- HTTP kernel: Axum's `Server::with_graceful_shutdown` with configurable drain timeout
- Worker kernel: Stop polling for new jobs, finish current job within grace period
- Scheduler kernel: Release leadership, cancel pending ticks
- Config: `[server] shutdown_timeout_seconds = 30`

---

## 2.3 Response Compression

**Status: Done** — Compression middleware (gzip + brotli) via tower-http CompressionLayer

Gzip/Brotli compression middleware via `tower-http::compression`.

### Consumer DX

```rust
App::builder()
    .register_middleware(Compression::new().build())
    .run_http()?;
```

---

## 2.4 Notifications (Multi-Channel)

**Status: Done** — Adapter pattern with `NotificationChannel` trait, typed `NotificationChannelId`, channel registry (project-level registration via `register_notification_channel()`), 3 built-in channels (Email, Database, Broadcast via Foundry WebSocket), `Notification` + `Notifiable` traits, `app.notify()`, `route_notification_for()` per-channel routing, `NOTIFY_EMAIL`/`NOTIFY_DATABASE`/`NOTIFY_BROADCAST` constants

### Queued Dispatch (`notify_queued`)

**Status: Done**

Dispatch notifications asynchronously via the job system instead of blocking the request.

#### Design

```rust
// Sync — blocks until all channels complete:
app.notify(&user, &OrderShipped { order }).await?;

// Async — dispatches as a job, returns immediately:
app.notify_queued(&user, &OrderShipped { order }).await?;
```

#### How it works

1. `notify_queued` requires `N: Notification + Serialize + DeserializeOwned + Debug` (same bounds as `Job`)
2. Framework provides `SendNotificationJob<N>` which wraps the notification + notifiable_id + channel list
3. On `handle()`, the job deserializes the notification and replays through the channel registry
4. The job is registered automatically when `notify_queued` is first called (or at bootstrap)

#### Consumer DX

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct OrderShipped { pub order_id: String, pub customer_name: String }

impl Notification for OrderShipped {
    fn notification_type(&self) -> &str { "order_shipped" }
    fn via(&self) -> Vec<NotificationChannelId> { vec![NOTIFY_EMAIL, NOTIFY_DATABASE] }
    fn to_email(&self, notifiable: &dyn Notifiable) -> Option<EmailMessage> { ... }
    fn to_database(&self) -> Option<serde_json::Value> { ... }
}

// Queue it — returns immediately, worker picks it up
app.notify_queued(&user, &OrderShipped { order_id: "123".into(), ... }).await?;
```

#### Implementation

- New file: `src/notifications/job.rs`
- New trait: `QueueableNotification: Notification + Serialize + DeserializeOwned + Debug + 'static`
- `SendNotificationJob` stores: `notifiable_id: String`, `channels: Vec<NotificationChannelId>`, `notification_data: serde_json::Value`, `notification_type: String`
- On handle: reconstruct notification from JSON, look up channels, dispatch
- Challenge: deserializing `dyn Notification` from JSON requires a type registry or the consumer to provide the deserialization. Simplest: store raw JSON data per channel (to_email result, to_database result) at dispatch time, then replay the pre-rendered payloads in the job. This avoids the type erasure problem entirely.

Send notifications across multiple channels from a single notification definition.

### Consumer DX

```rust
pub struct OrderShippedNotification {
    pub order: Order,
}

impl Notification for OrderShippedNotification {
    fn channels(&self) -> Vec<NotificationChannel> {
        vec![NotificationChannel::Email, NotificationChannel::Database]
    }

    fn to_email(&self, notifiable: &dyn Notifiable) -> Result<EmailMessage> {
        EmailMessage::new()
            .to(notifiable.email())
            .subject("Your order has shipped!")
            .body(format!("Order #{} is on its way.", self.order.id))
    }

    fn to_database(&self) -> Result<serde_json::Value> {
        Ok(json!({
            "type": "order_shipped",
            "order_id": self.order.id.to_string(),
            "message": "Your order has shipped!",
        }))
    }
}

// Send it
app.notify(&user, OrderShippedNotification { order }).await?;
```

### Channels

| Channel | Backend |
|---------|---------|
| `Email` | Existing EmailManager |
| `Database` | `notifications` table (requires migration) |
| `Broadcast` | WebSocket publish |

---

## 2.5 Prometheus Metrics

**Status: Done** — /metrics endpoint with Prometheus text format, based on RuntimeDiagnostics snapshot

Expose `/metrics` endpoint with request count, duration histograms, job counts, custom counters.

### Consumer DX

```rust
App::builder()
    .enable_observability_with(ObservabilityOptions::new().metrics(true))
    .run_http()?;

// GET /metrics returns Prometheus text format
```

---

# Job System Improvements

**Status: Blueprint complete — see [job-system](13-job-system.md)**

4 phases planned:
1. **Critical:** Worker concurrency, per-job timeout, graceful shutdown — ✅ Done
2. **High:** Job middleware (before/after/failed hooks) — ✅ Done
3. **Medium:** Batching, chaining, rate limiting, unique jobs — ✅ Done
4. **Low:** Job status tracking, dashboard endpoint — ✅ Done

---

# Tier 3: Advanced (Post-Sample, When Needed)

## 3.1 API Versioning

**Status: Done** — `HttpRegistrar::group(prefix, f)` and `api_version(version, f)` for path-prefix grouping.

## 3.2 OpenAPI Generation

**Status: Done** — `#[derive(ApiSchema)]` proc macro, `RouteDoc` builder on `HttpRouteOptions`, `/_foundry/openapi.json` endpoint. Maps validation rules to OpenAPI constraints, supports structs + enums.

## 3.3 Distributed Tracing

**Status: Done** — Optional OTEL via `otel` feature flag. OTLP exporter, configurable endpoint + service name, transparent `tracing` integration.

## 3.4 WebSocket Presence Channels

**Status: Done** — `WebSocketChannelOptions::presence(true)`, `ctx.presence_members()`, `ctx.presence_count()`. Backed by Redis SADD/SREM with auto-cleanup on disconnect.

## 3.5 Search Integration

**Status: Done** — `ModelQuery::search(&[columns], query)` generates PostgreSQL `to_tsvector @@ plainto_tsquery`. Also added `Condition::Raw` for arbitrary SQL conditions.

---

# Sample Project: Foundry Commerce

**Build after Tier 1 is complete.**

## Overview

Multi-tenant SaaS e-commerce platform demonstrating every Foundry feature.

## Portals & Guards

| Portal | Path | Guard | Model | Auth Method |
|--------|------|-------|-------|-------------|
| API | `/api/*` | `api` (token) | `User` | Bearer token (mobile/SPA) |
| Admin | `/admin/*` | `admin` (session) | `AdminUser` | Session cookie |
| Merchant | `/merchant/*` | `merchant` (session) | `Merchant` | Session cookie |

## Models

| Model | Relations | Features |
|-------|-----------|----------|
| `User` | has_many Orders | Authenticatable (api guard) |
| `AdminUser` | — | Authenticatable (admin guard) |
| `Merchant` | belongs_to User, has_many Products | Authenticatable (merchant guard), soft deletes |
| `Product` | belongs_to Merchant, has_many OrderItems | File upload (images), validation |
| `Order` | belongs_to User, has_many OrderItems | Lifecycle hooks, status enum |
| `OrderItem` | belongs_to Order, belongs_to Product | Computed totals |
| `Category` | many_to_many Products | Tag-like pivot |

## Key Features Exercised

- **Auth:** 3 guards (token + 2 session), CSRF on session portals, per-user rate limiting on API
- **CRUD:** Full REST API for products, orders
- **Datatables:** Admin user list, merchant product list, order list
- **Validation:** Create/update forms with sanitization
- **Jobs:** Order confirmation email (queued), inventory update
- **File uploads:** Product images to S3/local storage
- **Events:** OrderCreated → send email, update stats
- **Cache:** Product catalog caching
- **Testing:** Full test suite using TestApp + factories
- **CLI:** `db:migrate`, `db:seed`, custom commands

## Project Structure

```text
foundry-commerce/
├── src/
│   ├── app/
│   │   ├── ids.rs
│   │   ├── models/
│   │   │   ├── user.rs
│   │   │   ├── admin_user.rs
│   │   │   ├── merchant.rs
│   │   │   ├── product.rs
│   │   │   ├── order.rs
│   │   │   └── order_item.rs
│   │   ├── providers/
│   │   │   ├── auth.rs
│   │   │   └── app.rs
│   │   ├── portals/
│   │   │   ├── api/
│   │   │   ├── admin/
│   │   │   └── merchant/
│   │   ├── jobs/
│   │   ├── events/
│   │   └── notifications/
│   ├── bootstrap/
│   │   ├── http.rs
│   │   └── cli.rs
│   └── main.rs
├── config/
│   ├── app.toml
│   ├── auth.toml
│   ├── database.toml
│   └── cache.toml
├── database/
│   ├── migrations/
│   └── seeders/
├── tests/
│   ├── api/
│   ├── admin/
│   └── factories/
└── Cargo.toml
```

---

# Implementation Order

## Phase 1: Security (Tier 1a)
1. CSRF middleware
2. Input sanitization (Sanitize trait + derive macro)
3. Per-user rate limiting

## Phase 2: Testing (Tier 1b)
4. Test helpers (TestApp, TestClient, TestResponse)
5. Model factories (Factory trait)
6. Database test transactions

## Phase 3: Caching (Tier 1c)
7. Cache module (CacheManager, Redis driver, memory driver)
8. Cache tags

## Phase 4: DX (Tier 2)
9. CLI scaffolding commands
10. Graceful shutdown
11. Response compression
12. Notifications (email + database channels)

## Phase 5: Sample Project
13. Foundry Commerce — scaffold, models, auth, CRUD, tests

## Phase 6: Advanced (Tier 3 — as needed)
14. API versioning
15. Metrics
16. Tracing
17. Presence channels
18. Search

---

# Assumptions

- Tier 1 is **blocking** for the sample project
- Tier 2 can be built incrementally during/after the sample project
- Tier 3 is **not blocking** — build when specific need arises
- Each module follows existing Foundry patterns: trait-based, config-driven, registered via ServiceProvider
- New middleware follows existing `MiddlewareConfig` enum pattern with priority ordering
- New modules are registered in `AppBuilder` bootstrap like existing modules
- Testing module is `#[cfg(test)]`-gated where appropriate but core types are always available
