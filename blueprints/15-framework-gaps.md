# Foundry Framework Gaps Blueprint

> **Status:** ✅ Complete
> **Created:** 2026-04-12
> **Completed:** 2026-04-12
> **Purpose:** Final gap-fill to bring Foundry to full production framework completeness.

---

# What Already Exists

- `.paginate(Pagination)` returns `Paginated<T> { data, pagination, total }` — but no standardized JSON response format
- `CreateManyModel` for bulk insert, upsert with `on_conflict` — bulk ops exist
- `QueryExecutionOptions` has `label` and `timeout` — but no SQL logging to tracing
- `SoftDeleteScope` (ActiveOnly, WithTrashed, OnlyTrashed) — scopes exist for soft deletes only
- Dashboard routes at `/_foundry/` prefix (health, ready, runtime, metrics, jobs/stats, jobs/failed, openapi.json)
- `ObservabilityConfig.base_path` controls all dashboard URLs

---

# Phase 1: Query & Response Layer

## 1.1 Pagination Response Format

**Existing:** `Paginated<T>` has data + pagination + total but no JSON serialization with meta/links.

**Add:** `PaginatedResponse<T>` that serializes to standardized API format:

```rust
// Framework provides:
#[derive(Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub data: Vec<T>,
    pub meta: PaginationMeta,
    pub links: PaginationLinks,
}

#[derive(Serialize)]
pub struct PaginationMeta {
    pub current_page: u64,
    pub per_page: u64,
    pub total: u64,
    pub last_page: u64,
}

#[derive(Serialize)]
pub struct PaginationLinks {
    pub next: Option<String>,
    pub prev: Option<String>,
}
```

**Config:** default items per page in `[database]` section:
```toml
[database]
default_per_page = 15
```

**DX:**
```rust
let paginated = User::query()
    .where_(User::ACTIVE.eq(true))
    .paginate(&app, Pagination::new(page, per_page)).await?;

Ok(Json(paginated.to_response("/api/v1/users")))
// Returns: { data: [...], meta: {current_page, per_page, total, last_page}, links: {next, prev} }
```

**Files:** `src/database/query.rs` — add `to_response()` on `Paginated<T>`, add `PaginatedResponse` struct

---

## 1.2 Cursor-Based Pagination

For large datasets where offset pagination is slow.

**DX:**
```rust
let page = User::query()
    .order_by(User::ID.asc())
    .cursor_paginate(&app, CursorPagination {
        after: request.after_cursor,
        per_page: 20,
        column: User::ID,
    }).await?;

Ok(Json(page))
// Returns: { data: [...], meta: { has_next, has_prev }, cursors: { next, prev } }
```

**Internal:** `WHERE id > $cursor ORDER BY id ASC LIMIT $per_page + 1` (fetch one extra to detect `has_next`).

**Files:** `src/database/query.rs` — add `CursorPagination`, `CursorPaginated<T>`, `cursor_paginate()` method

---

## 1.3 API Resource Transformers

Shape model → API response (hide fields, rename, add computed).

**DX:**
```rust
pub struct UserResource;

impl ApiResource<User> for UserResource {
    fn transform(user: &User) -> serde_json::Value {
        json!({
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "member_since": user.created_at.format("%Y-%m-%d"),
        })
    }
}

// In handler:
Ok(Json(UserResource::collection(&users)))
// Or single: Json(UserResource::make(&user))
```

**Trait:**
```rust
pub trait ApiResource<T> {
    fn transform(item: &T) -> serde_json::Value;

    fn make(item: &T) -> serde_json::Value {
        Self::transform(item)
    }

    fn collection(items: &[T]) -> Vec<serde_json::Value> {
        items.iter().map(Self::transform).collect()
    }

    fn paginated(paginated: &Paginated<T>, base_url: &str) -> serde_json::Value {
        json!({
            "data": paginated.data.iter().map(Self::transform).collect::<Vec<_>>(),
            "meta": { ... },
            "links": { ... },
        })
    }
}
```

**Files:** `src/http/resource.rs` — new module

---

## 1.4 Model Scopes

Reusable query filters defined on models.

**DX:**
```rust
// Define scopes as methods on the model (via impl block):
impl User {
    pub fn active(query: ModelQuery<Self>) -> ModelQuery<Self> {
        query.where_(User::ACTIVE.eq(true))
    }

    pub fn recent(query: ModelQuery<Self>) -> ModelQuery<Self> {
        query.order_by(User::CREATED_AT.desc())
    }

    pub fn by_role(query: ModelQuery<Self>, role: &str) -> ModelQuery<Self> {
        query.where_(User::ROLE.eq(role))
    }
}

// Usage — chainable:
let users = User::query()
    .scope(User::active)
    .scope(|q| User::by_role(q, "admin"))
    .scope(User::recent)
    .paginate(&app, pagination).await?;
```

**Add `.scope()` method to `ModelQuery<M>`:**
```rust
pub fn scope(self, f: impl FnOnce(Self) -> Self) -> Self {
    f(self)
}
```

This is trivially simple — just a method that takes a closure. The "scope" is any function `ModelQuery<M> -> ModelQuery<M>`.

**Files:** `src/database/query.rs` — add `.scope()` to `ModelQuery`, `ProjectionQuery`

---

# Phase 2: HTTP & DX Layer

## 2.1 Named Routes & URL Generation

**DX:**
```rust
// Register with name:
r.route_named("users.index", "/api/v1/users", get(list_users));
r.route_named("users.show", "/api/v1/users/:id", get(show_user));

// Generate URL:
let url = app.route_url("users.show", &[("id", "123")])?;
// Returns: "/api/v1/users/123"
```

**Internal:** Route registry stored on `AppContext`, maps name → path pattern. URL generation does simple string replacement.

**Files:** `src/http/mod.rs` — add `route_named()`, `src/http/routes.rs` — RouteRegistry, `src/foundation/app.rs` — `route_url()` accessor

---

## 2.2 Middleware Groups

**DX:**
```rust
const WEB_MIDDLEWARE: MiddlewareGroupId = MiddlewareGroupId::new("web");
const API_MIDDLEWARE: MiddlewareGroupId = MiddlewareGroupId::new("api");

App::builder()
    .middleware_group(WEB_MIDDLEWARE, vec![
        Csrf::new().build(),
        Compression::new().build(),
    ])
    .middleware_group(API_MIDDLEWARE, vec![
        RateLimit::new(100).per_minute().by_actor_or_ip().build(),
        Compression::new().build(),
    ])
    .run_http()?;

// Use in routes:
r.route_with_options("/dashboard", get(dashboard),
    HttpRouteOptions::new().middleware_group(WEB_MIDDLEWARE));

r.route_with_options("/api/users", get(list_users),
    HttpRouteOptions::new().middleware_group(API_MIDDLEWARE));
```

**Internal:** `AppBuilder` and `MiddlewareGroups` store semantic `MiddlewareGroupId` keys. `HttpRouteOptions::middleware_group()` resolves the typed group at router-build time.

**Files:** `src/foundation/app.rs` — `middleware_group()` on builder, `src/http/mod.rs` — `middleware_group()` on options

---

## 2.3 ETag / Conditional Responses

**DX:**
```rust
// Middleware — automatic for JSON responses:
App::builder()
    .register_middleware(ETag::new().build())

// Or per-handler:
async fn show_user(Auth(user): Auth<User>) -> impl IntoResponse {
    ETagResponse::new(Json(user))  // auto-computes ETag from body hash
}
```

**Internal:** Middleware computes SHA-256 hash of response body, sets `ETag` header. On request, checks `If-None-Match` — if matches, returns 304 Not Modified.

**Files:** `src/http/middleware.rs` — add `ETag` variant to `MiddlewareConfig`

---

# Phase 3: Auth Flows

All auth flows must support multi-guard (User, Admin, Merchant) — the guard system determines which model table to use.

## 3.1 Password Reset Flow

**Migration:**
```sql
CREATE TABLE password_reset_tokens (
    email TEXT NOT NULL,
    guard TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX idx_password_reset ON password_reset_tokens (email, guard);
```

**Framework provides `PasswordResetManager`:**
```rust
let resets = app.password_resets()?;

// Send reset link (generates token, stores hash, sends email)
resets.send_reset_link::<User>(&email, |token, user| {
    EmailMessage::new("Reset your password")
        .to(&user.email)
        .text(format!("Reset link: https://app.com/reset?token={token}"))
}).await?;

// Validate and reset
resets.reset::<User>(&email, &token, |user| async move {
    // Consumer updates password
    user.update().set(User::PASSWORD, new_hash).save(&app).await
}).await?;
```

**Files:** `src/auth/password_reset.rs`, migration

---

## 3.2 Email Verification Flow

**Migration:**
```sql
ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMPTZ;
-- (Consumer adds this column to their model)
```

**Framework provides `EmailVerificationManager`:**
```rust
let verify = app.email_verification()?;

// Send verification email (generates token, sends email)
verify.send_verification::<User>(&user, |token, user| {
    EmailMessage::new("Verify your email")
        .to(&user.email)
        .text(format!("Verify: https://app.com/verify?token={token}"))
}).await?;

// Verify
verify.verify::<User>(&user_id, &token).await?;
// Sets email_verified_at = NOW() on the user's row
```

**Uses same `password_reset_tokens` table** with a different `guard` prefix like `verify:{guard}`.

**Files:** `src/auth/email_verification.rs`

---

## 3.3 Remember Me (Session Extension)

**DX:**
```rust
let sessions = app.sessions()?;
let session_id = sessions.create_with_remember::<User>(&user_id, remember).await?;
// If remember=true, uses extended TTL (e.g., 30 days instead of 2 hours)
```

**Config:**
```toml
[auth.sessions]
ttl_minutes = 120
remember_ttl_days = 30
```

**Internal:** `SessionManager::create` checks a `remember` flag and uses the extended TTL.

**Files:** `src/auth/session.rs` — add `remember` parameter, `src/config/mod.rs` — add `remember_ttl_days`

---

# Phase 4: SQL & Observability

## 4.1 SQL Query Logging

Log all SQL queries to tracing for debugging. Configurable.

**Config:**
```toml
[database]
log_queries = true        # default: false (enable in dev)
slow_query_threshold_ms = 500  # log slow queries even when log_queries is false
```

**Internal:** In `QueryExecutor` implementations, before executing, emit:
```rust
tracing::debug!(target: "foundry.sql", sql = %sql, bindings = ?bindings, label = ?options.label, "query");
// After execution:
tracing::debug!(target: "foundry.sql", duration_ms = elapsed, rows = count, "query completed");
```

For slow queries:
```rust
if elapsed > slow_threshold {
    tracing::warn!(target: "foundry.sql", sql = %sql, duration_ms = elapsed, "slow query detected");
}
```

**Dashboard:** Add `/_foundry/sql` endpoint that shows recent slow queries (stored in a ring buffer).

**Files:** `src/database/runtime.rs` — add logging to query execution, `src/config/mod.rs` — add log config

---

## 4.2 Dashboard Consolidation

Ensure all dashboard endpoints follow consistent pattern:

```
/_foundry/health          — liveness probe
/_foundry/ready           — readiness probe
/_foundry/runtime         — runtime snapshot
/_foundry/metrics         — Prometheus format
/_foundry/jobs/stats      — job statistics
/_foundry/jobs/failed     — failed jobs
/_foundry/openapi.json    — OpenAPI spec
/_foundry/sql             — slow query log (NEW)
/_foundry/ws/channels     — registered WebSocket channels
/_foundry/ws/presence/:channel — live presence members
/_foundry/ws/history/:channel  — recent buffered messages (metadata by default)
/_foundry/ws/stats        — global + per-channel WebSocket counters
```

All gated by `ObservabilityConfig.base_path` (configurable). All JSON except metrics (text/plain).

---

# Phase 5: Email Templates

## 5.1 Mail Templates

Multiple rendering interfaces (like Laravel's Mailable):

**DX:**
```rust
// Raw text
let msg = EmailMessage::new("Welcome!")
    .to(&user.email)
    .text("Welcome to our app.");

// Raw HTML
let msg = EmailMessage::new("Welcome!")
    .to(&user.email)
    .html("<h1>Welcome!</h1><p>Thanks for joining.</p>");

// Template with variables (simple string replacement)
let msg = EmailMessage::new("Welcome!")
    .to(&user.email)
    .template("welcome", json!({
        "name": user.name,
        "app_name": "MyApp",
        "verify_url": verify_url,
    }));
```

**Template loading:** Templates stored as files in `templates/emails/`:
```
templates/emails/welcome.html
templates/emails/welcome.txt
```

Framework loads and renders with simple `{{variable}}` replacement. No complex template engine — just string substitution. If the consumer wants Tera/Handlebars, they can render to string and use `.html()`.

**Files:** `src/email/template.rs` — template loader + renderer, `src/config/mod.rs` — template path config

---

# Phase 6: Database Advanced

## 6.1 Read Replicas

Separate read and write database connections.

**Config:**
```toml
[database]
url = "postgres://localhost/foundry"           # write (primary)
read_url = "postgres://replica/foundry"        # read (replica, optional)
```

**Internal:** `DatabaseManager` holds two pools when `read_url` is configured. `ModelQuery::get()` and `::first()` use the read pool. `CreateModel`, `UpdateModel`, `DeleteModel` use the write pool.

**DX:** Transparent — consumer doesn't need to change anything. Queries auto-route. Force write pool:
```rust
User::query().use_write_pool().get(&app).await?;
```

**Files:** `src/database/runtime.rs` — dual pool support

---

# Phase 7: Job Priority Queues

**Why priorities still matter with spawn-per-job:** The semaphore controls *how many* jobs run concurrently. But *which* job gets claimed next depends on claim order. Without priority, it's FIFO. With priority, high-priority jobs skip the queue.

**Config:**
```toml
[jobs.queues.high]
priority = 1    # claimed first

[jobs.queues.default]
priority = 5

[jobs.queues.low]
priority = 10   # claimed last
```

**Internal:** `claim_job()` currently iterates `registry.queues` in arbitrary order. Change to sort by priority — check high-priority queues first.

**Consumer DX:**
```rust
impl Job for UrgentAlert {
    const ID: JobId = JobId::new("urgent_alert");
    const QUEUE: Option<QueueId> = Some(QueueId::new("high"));
    // ...
}
```

**Files:** `src/jobs/mod.rs` — priority-sorted claim, `src/config/mod.rs` — queue priority config

---

# Implementation Order

| Phase | Items | Scope |
|-------|-------|-------|
| 1 | Pagination response, cursor pagination, API resources, model scopes | Query/Response |
| 2 | Named routes, middleware groups, ETag | HTTP/DX |
| 3 | Password reset, email verification, remember me | Auth flows |
| 4 | SQL query logging, dashboard consolidation | Observability |
| 5 | Mail templates | Email |
| 6 | Read replicas | Database |
| 7 | Job priority queues | Jobs |

---

# Assumptions

- Template system is simple `{{variable}}` replacement, not a full engine
- Read replicas are transparent to the consumer (auto-routing)
- Password reset and email verification use the same token table pattern
- All dashboard endpoints share the same `base_path` config
- Queue priorities are integer-based (lower = higher priority)
- Model scopes are just functions `ModelQuery<M> -> ModelQuery<M>` — no macro needed
- API Resources are a trait the consumer implements — no derive macro needed
