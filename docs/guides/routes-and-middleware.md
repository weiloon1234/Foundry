# Routes & Middleware Guide

HTTP routing, middleware stack, named URLs, API versioning, rate limiting, and more.

> For auth guards, permissions, and policies on routes, see [Auth Guide](auth.md).

---

## Quick Start

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route("/health", get(health));
    r.route("/posts", get(list_posts));
    r.route("/posts", post(create_post));
    Ok(())
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}
```

Register in bootstrap:

```rust
App::builder()
    .register_routes(routes)
    .run_http()?;
```

Define middleware group IDs once alongside your other semantic identifiers and reuse the constants
for both registration and route references:

```rust
const API_MIDDLEWARE: MiddlewareGroupId = MiddlewareGroupId::new("api");
const WEB_MIDDLEWARE: MiddlewareGroupId = MiddlewareGroupId::new("web");
```

Use `MiddlewareGroupId::owned(value)` only when an identifier genuinely comes from dynamic
application configuration; raw strings are intentionally not accepted by registration or routes.

---

## Route Registration

### Basic Routes

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route("/posts", get(list_posts));
    r.route("/posts", post(create_post));
    r.route("/posts/:id", get(show_post));
    r.route("/posts/:id", put(update_post));
    r.route("/posts/:id", delete(delete_post));
    Ok(())
}
```

### Routes with Options

Attach guards, permissions, middleware, and rate limits to individual routes:

```rust
r.route_with_options("/posts", post(create_post),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .permission(Permission::PostsWrite)
        .rate_limit(RateLimit::new(10).per_minute().by_actor()));
```

For MFA completion routes, opt in to pending tokens explicitly. This is what allows the short-lived
`auth:mfa_pending` token to reach `/auth/mfa/verify` while every other guarded route continues to
reject it.

```rust
r.route_with_options("/auth/mfa/verify", post(foundry::auth::mfa::routes::verify),
    HttpRouteOptions::new()
        .guard(Guard::Admin)
        .allow_mfa_pending_token());
```

Built-in audit logging is activated the same way: opt an admin route tree into an audit area once,
then let child routes inherit it.

```rust
r.route_with_options("/admin/users", post(create_admin_user),
    HttpRouteOptions::new()
        .guard(Guard::Admin)
        .audit_area("admin"));
```

See [Auth Guide](auth.md) for setting up `Guard` and `Permission` enums.

### Scope DSL

For portal-style route modules, prefer `scope()` so path prefixes, relative route names, shared tags, and shared access rules live in one place:

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.api_version(1, |r| {
        r.scope("/admin", |admin| {
            admin.name_prefix("admin").audit_area("admin");

            admin.scope("/auth", |auth| {
                auth.name_prefix("auth").tag("admin:auth");

                auth.post("/login", "login", login, |route| {
                    route.public();
                    route.summary("Admin login");
                    route.request::<AdminLoginRequest>();
                    route.response::<TokenPair>(200);
                });

                auth.get("/me", "me", me, |route| {
                    route.guard(Guard::Admin);
                    route.summary("Get authenticated admin profile");
                    route.response::<AdminMeResponse>(200);
                });

                Ok(())
            })?;

            admin.scope("/profile", |profile| {
                profile
                    .name_prefix("profile")
                    .tag("admin:profile")
                    .guard(Guard::Admin);

                profile.put("", "update", update_profile, |route| {
                    route.summary("Update admin profile");
                    route.request::<UpdateAdminProfileRequest>();
                    route.response::<AdminMeResponse>(200);
                });

                Ok(())
            })?;

            Ok(())
        })?;

        Ok(())
    })?;

    Ok(())
}
```

This registers named routes such as `admin.auth.login`, `admin.auth.me`, and `admin.profile.update` automatically.

### Area-Gated Audit Logging

Foundry's built-in audit writer is area-gated by default. A mutation is only written to `audit_logs`
when all of the following are true:

- the current HTTP route/scope/group resolves to an audit area
- the model has not opted out with `#[foundry(audit = false)]`

Audit activation is code-driven; config only controls payload redaction. `config:publish` and
`env:publish` emit `[audit]` / `AUDIT__*` defaults for credential-like field redaction.

The usual project-level setup is one line on the outer admin scope:

```rust
r.scope("/admin", |admin| {
    admin
        .name_prefix("admin")
        .guard(Guard::Admin)
        .audit_area("admin");

    admin.post("/users", "store", create_admin_user, |_| {});

    admin.scope("/sensitive", |sensitive| {
        sensitive.name_prefix("sensitive").audit_disabled();
        sensitive.post("/exports", "exports", export_sensitive_data, |_| {});
        Ok(())
    })?;

    Ok(())
})?;
```

Use `audit_disabled()` for exceptions inside an audited parent scope, or override with a different
area such as `support.audit_area("support")`.

Handlers can also read the resolved area through the public `CurrentRequest` extractor:

```rust
async fn create_admin_user(
    request: CurrentRequest,
    State(app): State<AppContext>,
) -> Result<StatusCode> {
    assert_eq!(request.audit_area.as_deref(), Some("admin"));
    Ok(StatusCode::CREATED)
}
```

### Route Groups

Prefix a set of routes without nesting into a separate router:

```rust
r.group("/admin", |r| {
    r.route("/dashboard", get(dashboard));     // /admin/dashboard
    r.route("/users", get(admin_users));        // /admin/users
    r.route("/users/:id", get(admin_user));     // /admin/users/:id
    Ok(())
})?;
```

Groups can nest:

```rust
r.group("/api", |r| {
    r.group("/v1", |r| {
        r.route("/posts", get(v1_posts));       // /api/v1/posts
        Ok(())
    })?;
    Ok(())
})?;
```

If a whole group shares the same guard, middleware, or documentation tags, use `group_with_options()` to define those defaults once:

```rust
r.group_with_options(
    "/api/admin",
    HttpRouteOptions::new()
        .guard(Guard::Admin)
        .audit_area("admin")
        .middleware_group(API_MIDDLEWARE)
        .tag("admin"),
    |r| {
        r.route("/users", get(list_admin_users));
        r.route_with_options(
            "/stats",
            get(admin_stats),
            HttpRouteOptions::new().summary("Admin stats"),
        );
        Ok(())
    },
)?;
```

Per-route options inside the group are merged with the group defaults.

Use `group()` and `group_with_options()` when you want the low-level API directly. Use `scope()` when you also want relative route names and the higher-level route builder.

### API Versioning

Shorthand for `/api/v{N}` groups:

```rust
r.api_version(1, |r| {
    r.route("/users", get(list_users_v1));     // /api/v1/users
    r.route("/posts", get(list_posts_v1));     // /api/v1/posts
    Ok(())
})?;

r.api_version(2, |r| {
    r.route("/users", get(list_users_v2));     // /api/v2/users
    Ok(())
})?;
```

### Nest & Merge

For integrating external Axum routers:

```rust
// Nest under a prefix
let admin_router = Router::new().route("/stats", get(stats));
r.nest("/admin", admin_router);  // /admin/stats

// Merge at the same level (no prefix)
let health_router = Router::new().route("/healthz", get(healthz));
r.merge(health_router);  // /healthz
```

---

## Named Routes & URL Generation

### Registering Named Routes

With `scope()`, route names are usually relative and compose automatically:

```rust
r.scope("/admin", |admin| {
    admin.name_prefix("admin");

    admin.scope("/users", |users| {
        users.name_prefix("users");
        users.get("", "index", list_users, |_| {});
        users.get("/:id", "show", show_user, |_| {});
        Ok(())
    })?;

    Ok(())
})?;
```

The example above registers `admin.users.index` and `admin.users.show`.

The lower-level named route APIs remain available when you want to register names manually:

```rust
#[derive(Clone, Copy, FoundryId)]
#[foundry(id = RouteId)]
enum Route {
    #[foundry(value = "posts.list")]
    PostsList,
    #[foundry(value = "posts.show")]
    PostsShow,
    #[foundry(value = "password.reset")]
    PasswordReset,
    #[foundry(value = "posts.create")]
    PostsCreate,
}

r.route_named(Route::PostsList, "/posts", get(list_posts));
r.route_named(Route::PostsShow, "/posts/:id", get(show_post));
r.route_named(Route::PasswordReset, "/reset/:token", get(reset_form));

// Named + options
r.route_named_with_options(Route::PostsCreate, "/posts", post(create_post),
    HttpRouteOptions::new().guard(Guard::User));
```

### Resource Routes

For common CRUD route sets, use `resource()` or `resource_with_options()`:

```rust
r.resource_with_options(
    "posts",
    "/posts",
    HttpResourceRoutes::new()
        .index(get(list_posts))
        .store(post(create_post))
        .show(get(show_post))
        .update(put(update_post))
        .destroy(delete(delete_post)),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .middleware_group(API_MIDDLEWARE)
        .tag("posts"),
);
```

This registers conventional named routes such as `posts.index`, `posts.store`, `posts.show`, `posts.update`, and `posts.destroy`.

### Generating URLs

In a handler:

```rust
async fn some_handler(State(app): State<AppContext>) -> Result<impl IntoResponse> {
    let url = app.route_url(Route::PostsShow, &[("id", "42")])?;
    // → "/posts/42"

    Ok(Json(json!({ "url": url })))
}
```

### Frontend Route URLs

Named routes are also exported to TypeScript during `types:export`. This lets
frontend code use the same route id SSOT instead of hand-writing internal API
paths:

```typescript
import { RouteIds, createRouteUrlBuilder } from "@shared/types/generated";

const adminRouteUrl = createRouteUrlBuilder({ basePath: "/api/v1/admin" });

api.get(adminRouteUrl(RouteIds.admin.users.show, { id: userId }));
// -> "/users/123" for an Axios client whose baseURL is "/api/v1/admin"
```

`RouteManifest.ts` is generated from registered named routes. It supports Axum
`{id}` / `{*path}` params and legacy `:id` params, URL-encodes substituted
values, and fails fast for duplicate route ids during export.

This does not replace request validation. Handlers should keep accepting trusted
input through Foundry extractors such as `JsonValidated<T>` and `Validated<T>`.

### Signed URLs

Generate tamper-proof URLs with expiry (for password resets, email verification, etc.):

```rust
let url = app.signed_route_url(
    Route::PasswordReset,
    &[("token", &reset_token)],
    DateTime::now().add_days(1),  // expires in 24 hours
)?;
// → "/reset/abc123?expires=1704067200&signature=hmac_sha256..."
```

Verify in a handler:

```rust
async fn reset_form(State(app): State<AppContext>, request: Request) -> Result<impl IntoResponse> {
    app.verify_signed_url(&request.uri().to_string())?;
    // Returns Error if expired, tampered, duplicated, or appended after signing
    Ok(Html("reset form"))
}
```

Signed URL verification rejects duplicate `expires` or `signature` parameters and rejects query
parameters appended after `signature`, so only the originally signed URL shape is accepted.
`app.signing_key` must be a base64 key that decodes to at least 32 bytes. Generate one with
`key:generate`; `doctor --deploy` reports missing keys as warnings and invalid or weak configured
keys as failures.

---

## Middleware Stack

### Global Middleware

Register middleware that runs on every request:

```rust
App::builder()
    .register_middleware(MiddlewareConfig::from(Compression))
    .register_middleware(MiddlewareConfig::from(
        SecurityHeaders::new()
            .content_security_policy("default-src 'self'")
    ))
    .register_middleware(MiddlewareConfig::from(
        Cors::new()
            .allow_origins(["https://app.example.com"])
            .allow_credentials()
    ))
    .run_http()?;
```

### Global HTTP Config

Foundry can also derive global edge middleware from split files under `config/`, for example
`config/10-http.toml`. Foundry loads every direct `config/*.toml` file in lexical order, merges
them in memory, then applies `.env` overrides. This is additive: existing route middleware and
code-registered global middleware keep working, and an explicitly registered middleware kind wins
over the config-derived duplicate.

```toml
[http]
max_body_size_bytes = 0        # 0 = no global cap
request_timeout_ms = 0         # 0 = no global timeout

[http.security_headers]
enabled = true
hsts = false                   # enable only after HTTPS is guaranteed
frame_options = "DENY"
referrer_policy = "strict-origin-when-cross-origin"
content_security_policy = ""

[http.trusted_proxy]
enabled = true
# Defaults to Cloudflare ranges. Add loopback/private CIDRs if a local reverse proxy
# sits between Cloudflare and Foundry.
trusted_cidrs = ["173.245.48.0/20", "103.21.244.0/22", "..."]
headers = ["cf-connecting-ip", "x-real-ip", "x-forwarded-for"]

[http.cors]
enabled = false
allowed_origins = []
allowed_methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
allowed_headers = ["authorization", "content-type", "x-request-id", "x-csrf-token"]
allow_credentials = false
max_age_seconds = 600

[http.csrf]
enabled = false
cookie_name = "foundry_csrf"
header_name = "x-csrf-token"
cookie_secure = true
cookie_path = "/"
cookie_same_site = "lax"       # lax | strict | none; none requires secure cookies
exclude_paths = []

[http.rate_limit]
enabled = true
max_requests = 600
window_seconds = 60
by = "actor_or_ip"             # ip | actor | actor_or_ip
key_prefix = "http:"
```

Compatibility defaults keep hard caps, CORS, and CSRF opt-in. Trusted proxy is enabled by default
for Cloudflare CIDRs only, rate limiting is enabled by default with `actor_or_ip`, and security
headers are enabled by default with HSTS off, so local HTTP and first deploys stay usable while
still publishing the production hardening knobs.

### Execution Order

Middleware runs in **priority order** — lower numbers run first (outermost layer):

```
Request
  │
  ├─ 0  TrustedProxy         ← extract real IP from proxy headers
  ├─ 1  MaintenanceMode      ← return 503 if in maintenance
  ├─ 10 Cors                 ← handle CORS preflight
  ├─ 20 SecurityHeaders      ← add security headers
  ├─ 25 Csrf                 ← validate CSRF tokens
  ├─ 30 RateLimit            ← check rate limits
  ├─ 40 MaxBodySize          ← enforce body size
  ├─ 50 RequestTimeout       ← enforce timeout
  ├─ 55 ETag                 ← conditional response (304)
  ├─ 60 Compression          ← compress response
  │
  ├─ [per-route middleware]   ← from HttpRouteOptions
  ├─ [auth middleware]        ← if route requires guard
  │
  └─ Handler
```

You don't need to worry about order — the framework sorts by priority automatically.

---

## Middleware Reference

### Compression

Gzip + Brotli based on `Accept-Encoding`:

```rust
MiddlewareConfig::from(Compression)
```

### CORS

```rust
MiddlewareConfig::from(
    Cors::new()
        .allow_origins(["https://app.example.com", "https://admin.example.com"])
        .allow_any_method()
        .allow_any_header()
        .allow_credentials()
        .max_age(3600)
)
```

For development:

```rust
MiddlewareConfig::from(Cors::new().allow_any_origin())
```

### Security Headers

Adds HSTS, CSP, X-Frame-Options, X-Content-Type-Options, Referrer-Policy:

```rust
MiddlewareConfig::from(
    SecurityHeaders::new()
        .content_security_policy("default-src 'self'; script-src 'self' 'unsafe-inline'")
        .frame_options("SAMEORIGIN")
)
```

Defaults (applied without any builder calls):

| Header | Default Value |
|--------|--------------|
| `Strict-Transport-Security` | `max-age=31536000; includeSubDomains` |
| `X-Content-Type-Options` | `nosniff` |
| `X-Frame-Options` | `DENY` |
| `Referrer-Policy` | `strict-origin-when-cross-origin` |
| `X-XSS-Protection` | `0` |

### CSRF

Double-submit cookie pattern for state-changing requests:

```rust
MiddlewareConfig::from(
    Csrf::new()
        .exclude("/api")       // skip CSRF for API routes (use token auth instead)
)
```

**How it works:**

- GET/HEAD/OPTIONS → generates CSRF token, sets a `foundry_csrf` cookie readable by JS
- POST/PUT/PATCH/DELETE → validates `X-CSRF-Token` header matches cookie
- Returns 403 if mismatch

The CSRF cookie uses `Path=/` and `SameSite=Lax`. It is intentionally not
`HttpOnly`, because browser JavaScript must read the token and echo it in the
request header. Exclusions are segment-aware: `.exclude("/api")` skips `/api`
and `/api/...`, but does not skip `/apiary`.

**Frontend integration:**

```javascript
const token = document.cookie.split('; ')
    .find(row => row.startsWith('foundry_csrf='))?.split('=')[1];

fetch('/form', {
    method: 'POST',
    headers: { 'X-CSRF-Token': token },
    body: formData,
});
```

**Extract token in handler** (e.g., to embed in HTML form):

```rust
async fn form(CsrfToken(token): CsrfToken) -> impl IntoResponse {
    Html(format!(r#"<input type="hidden" name="_token" value="{token}">"#))
}
```

### Rate Limiting

```rust
// Global: 1000 requests per hour per IP
MiddlewareConfig::from(RateLimit::new(1000).per_hour())

// Per-route: 10 per minute per authenticated user
HttpRouteOptions::new()
    .rate_limit(RateLimit::new(10).per_minute().by_actor())
```

**Strategies:**

| Method | Key | Auth required? | Use case |
|--------|-----|---------------|----------|
| (default) | Client IP | No | Global rate limit |
| `.by_actor()` | Actor ID | Yes | Per-user limits |
| `.by_actor_or_ip()` | Actor ID or IP | No | Actor if authenticated, IP as fallback |

**Response headers on rate-limited requests:**

```
X-RateLimit-Limit: 1000
X-RateLimit-Remaining: 42
X-RateLimit-Reset: 1704067200
```

On limit exceeded: **429 Too Many Requests** with `Retry-After` header.
Foundry returns a JSON error body:

```json
{ "message": "Rate limit exceeded", "status": 429 }
```

Actor-only limits run only after an authenticated actor exists, so use route-level rate limits for
strict per-actor limits. `actor_or_ip` uses the actor once authenticated and the client IP
otherwise, which is the safer global fallback. IP keys use `RealIp` from `TrustedProxy` when
available and fall back to TCP peer connect info on the real server path.

### Max Body Size

```rust
MiddlewareConfig::from(MaxBodySize::mb(10))    // 10 MB limit
MiddlewareConfig::from(MaxBodySize::kb(512))   // 512 KB limit
MiddlewareConfig::from(MaxBodySize::bytes(1024)) // 1024 bytes
```

Returns **413 Payload Too Large** with a JSON error body if exceeded.

### Request Timeout

```rust
MiddlewareConfig::from(RequestTimeout::secs(30))
MiddlewareConfig::from(RequestTimeout::mins(5))
```

Returns **408 Request Timeout** with a JSON error body if exceeded.

### File Downloads

Use Foundry's download helpers when returning user-facing filenames:

```rust
use axum::http::header::CONTENT_DISPOSITION;
use foundry::http::download::attachment_content_disposition;

Response::builder()
    .header(CONTENT_DISPOSITION, attachment_content_disposition("report.xlsx"));
```

The helper strips path-like/control-character input, prevents header injection, emits a safe ASCII
`filename`, and includes RFC 5987 `filename*` for Unicode clients.

### ETag

Automatic conditional responses — returns 304 Not Modified when content hasn't changed:

```rust
MiddlewareConfig::from(ETag::new())
```

Computes SHA-256 of response body. If client sends `If-None-Match` header matching the ETag, returns 304 with no body. Skips responses larger than 10 MB.

### Trusted Proxy

Extract real client IP from proxy headers:

```rust
MiddlewareConfig::from(TrustedProxy::cloudflare())
```

Resolution order: `CF-Connecting-IP` → `X-Real-IP` → `X-Forwarded-For` (first entry).
When configured through `[http.trusted_proxy]`, Foundry only honors those headers if the TCP peer IP
matches `trusted_cidrs`. The default CIDR set trusts Cloudflare proxy ranges, so
code-registered `TrustedProxy::new()` and `TrustedProxy::cloudflare()` accept Cloudflare by
default. Add `trusted_cidr("127.0.0.1/32")` or config loopback CIDRs when Nginx/Caddy sits on the
same host between Cloudflare and Foundry, and reserve `trust_all()` for controlled tests.

The resolved IP is available via the `RealIp` extractor:

```rust
async fn handler(RealIp(ip): RealIp) -> impl IntoResponse {
    Json(json!({ "your_ip": ip.to_string() }))
}
```

### Maintenance Mode

Returns 503 for all requests (bypassed with secret):

```rust
MiddlewareConfig::from(
    MaintenanceMode::new()
        .bypass_secret("my-secret")
)
```

**CLI commands:**

```bash
cargo run -- down --secret=my-secret    # enter maintenance mode
cargo run -- up                          # exit maintenance mode
```

**Bypass via header:**

```bash
curl -H "X-Maintenance-Bypass: my-secret" https://app.example.com
```

---

## Middleware Groups

Define a middleware bundle once under a semantic ID, then reuse the same typed constant on routes:

### Define

```rust
App::builder()
    .middleware_group(API_MIDDLEWARE, vec![
        MiddlewareConfig::from(RateLimit::new(1000).per_hour()),
        MiddlewareConfig::from(Compression),
    ])
    .middleware_group(WEB_MIDDLEWARE, vec![
        MiddlewareConfig::from(Csrf::new()),
        MiddlewareConfig::from(SecurityHeaders::new()),
        MiddlewareConfig::from(Compression),
    ])
```

### Apply

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    // API routes get "api" group middleware
    r.api_version(1, |r| {
        r.route_with_options("/users", get(list_users),
            HttpRouteOptions::new()
                .guard(Guard::User)
                .middleware_group(API_MIDDLEWARE));
        Ok(())
    })?;

    // Web routes get "web" group middleware
    r.group("/dashboard", |r| {
        r.route_with_options("/", get(dashboard),
            HttpRouteOptions::new()
                .guard(Guard::Admin)
                .middleware_group(WEB_MIDDLEWARE));
        Ok(())
    })?;

    Ok(())
}
```

Group middleware is prepended before any per-route middleware. You can combine a group with additional per-route middleware:

```rust
HttpRouteOptions::new()
    .middleware_group(API_MIDDLEWARE)
    .middleware(MiddlewareConfig::from(MaxBodySize::mb(50)))  // on top of group
```

---

## Per-Route Middleware

Apply middleware to a single route via `HttpRouteOptions`:

```rust
r.route_with_options("/upload", post(upload_file),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .middleware(MiddlewareConfig::from(MaxBodySize::mb(100)))
        .middleware(MiddlewareConfig::from(RequestTimeout::mins(5))));
```

Per-route middleware runs **after** global middleware and **before** the auth check.

---

## SPA Serving

Serve a frontend SPA (React, Vue, etc.) with client-side routing fallback:

```rust
App::builder()
    .serve_spa("frontend/dist")
    .register_routes(api_routes)
    .run_http()?;
```

All requests not matched by API routes fall back to `frontend/dist/index.html`. Static assets (JS, CSS, images) are served directly from the directory.

---

## Route Listing

See all named routes:

```bash
cargo run -- routes:list
```

```
NAME                           PATH
posts.list                     /api/v1/posts
posts.show                     /api/v1/posts/:id
posts.create                   /api/v1/posts
password.reset                 /reset/:token
admin.dashboard                /admin/dashboard
```

---

## API Resources

Transform models into consistent JSON response shapes:

```rust
struct UserResource;

impl ApiResource<User> for UserResource {
    fn transform(user: &User) -> Value {
        json!({
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "joined": user.created_at.format(),
        })
    }
}
```

Use in handlers:

```rust
async fn list_users(State(app): State<AppContext>) -> impl IntoResponse {
    let db = app.database()?;
    let users = User::model_query().all(&*db).await?;
    Json(UserResource::collection(&users))
}

async fn show_user(State(app): State<AppContext>, Path(id): Path<String>) -> impl IntoResponse {
    let db = app.database()?;
    let user = User::model_query()
        .where_col(User::ID, &id)
        .first(&*db).await?
        .ok_or_else(|| Error::not_found("user not found"))?;
    Json(UserResource::make(&user))
}

// Paginated response with meta + links
async fn paginated_users(State(app): State<AppContext>, Query(page): Query<Pagination>) -> impl IntoResponse {
    let db = app.database()?;
    let paginated = User::model_query()
        .paginate(page, &*db).await?;
    Json(UserResource::paginated(&paginated, "/api/v1/users"))
}
```

---

## Complete Example

```rust
use foundry::prelude::*;

const API_MIDDLEWARE: MiddlewareGroupId = MiddlewareGroupId::new("api");
const WEB_MIDDLEWARE: MiddlewareGroupId = MiddlewareGroupId::new("web");

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(AppServiceProvider)

        // Global middleware
        .register_middleware(MiddlewareConfig::from(TrustedProxy::cloudflare()))
        .register_middleware(MiddlewareConfig::from(Compression))
        .register_middleware(MiddlewareConfig::from(
            SecurityHeaders::new()
                .content_security_policy("default-src 'self'")
        ))

        // Middleware groups
        .middleware_group(API_MIDDLEWARE, vec![
            MiddlewareConfig::from(RateLimit::new(1000).per_hour()),
        ])
        .middleware_group(WEB_MIDDLEWARE, vec![
            MiddlewareConfig::from(Csrf::new().exclude("/api")),
        ])

        // SPA frontend
        .serve_spa("frontend/dist")

        // Routes
        .register_routes(routes)
        .run_http()
}

fn routes(r: &mut HttpRegistrar) -> Result<()> {
    // Public
    r.route("/health", get(|| async { Json(json!({ "ok": true })) }));

    // API v1
    r.api_version(1, |r| {
        r.route_named(Route::PostsList, "/posts", get(list_posts));

        r.route_named_with_options(Route::PostsCreate, "/posts", post(create_post),
            HttpRouteOptions::new()
                .guard(Guard::User)
                .permission(Permission::PostsWrite)
                .middleware_group(API_MIDDLEWARE)
                .rate_limit(RateLimit::new(30).per_minute().by_actor()));

        r.route_named_with_options(Route::PostsShow, "/posts/:id", get(show_post),
            HttpRouteOptions::new()
                .guard(Guard::User)
                .middleware_group(API_MIDDLEWARE));

        Ok(())
    })?;

    // Admin dashboard
    r.group("/admin", |r| {
        r.route_with_options("/", get(dashboard),
            HttpRouteOptions::new()
                .guard(Guard::Admin)
                .permission(Permission::AdminAccess)
                .middleware_group(WEB_MIDDLEWARE));
        Ok(())
    })?;

    Ok(())
}
```
