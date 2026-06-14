# Rust Auth & Actor System Blueprint (Framework-Level)

## Overview

This document defines the design of the **authentication, actor, and guard system** in Foundry.

Goal:

> Provide a multi-guard, model-backed authentication system where each guard maps to an authenticator (how tokens are validated) and an authenticatable model (what entity the token represents), with typed extractors that resolve the model automatically in handlers.

This is a **blueprint + implementation status document**. Sections marked with implementation status reflect what is already built.

---

# Core Concepts

## Actor

**Status: Done**

The lightweight auth payload that represents an authenticated entity. Created by middleware, stored in request extensions.

```rust
pub struct Actor {
    pub id: String,
    pub guard: GuardId,
    pub roles: BTreeSet<RoleId>,
    pub permissions: BTreeSet<PermissionId>,
    pub claims: Option<Value>,
}
```

The Actor knows **who** you are (id, guard, roles, permissions) but not **what** you are (no model data). It is the bridge between authentication (middleware) and authorization (handlers).

## Guard

**Status: Done**

A named authentication strategy. Each guard has:
- A `GuardId` (typed string identifier, e.g., `"api"`, `"admin"`, `"merchant"`)
- A `BearerAuthenticator` implementation (how tokens are validated)
- An `Authenticatable` model (what database entity the actor resolves to)

This is equivalent to Laravel's guard system where different guards authenticate different user types.

## Authenticatable

**Status: Done**

A trait that database models implement to declare themselves as the backing model for a guard. This enables `actor.resolve::<User>(&app).await` — equivalent to Laravel's `$request->user()`.

---

# Consumer Project Structure

A typical multi-guard project looks like:

```text
src/
├── app/
│   ├── ids.rs              # Guard IDs, permission IDs, role IDs
│   ├── models/
│   │   ├── user.rs         # User model + Authenticatable impl
│   │   ├── admin_user.rs   # AdminUser model + Authenticatable impl
│   │   └── merchant.rs     # Merchant model + Authenticatable impl
│   ├── providers/
│   │   └── auth.rs         # AuthServiceProvider — registers guards + models
│   ├── portals/
│   │   ├── api/            # User-facing routes (guard: "api")
│   │   ├── admin/          # Admin routes (guard: "admin")
│   │   └── merchant/       # Merchant routes (guard: "merchant")
│   └── authenticators/
│       ├── api_auth.rs     # Custom BearerAuthenticator for user tokens
│       └── admin_auth.rs   # Custom BearerAuthenticator for admin tokens
├── config/
│   └── auth.toml           # default_guard, bearer_prefix
└── main.rs
```

---

# Step-by-Step: Registering a New Guard (Consumer Guide)

## Step 1: Define Guard and Permission IDs

```rust
// src/app/ids.rs
use foundry::prelude::*;

#[derive(Clone, Copy)]
pub enum AuthGuard {
    Api,
    Admin,
    Merchant,
}

impl From<AuthGuard> for GuardId {
    fn from(value: AuthGuard) -> Self {
        match value {
            AuthGuard::Api => GuardId::new("api"),
            AuthGuard::Admin => GuardId::new("admin"),
            AuthGuard::Merchant => GuardId::new("merchant"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Ability {
    UsersView,
    UsersManage,
    OrdersView,
    ReportsExport,
}

impl From<Ability> for PermissionId {
    fn from(value: Ability) -> Self {
        match value {
            Ability::UsersView => PermissionId::new("users:view"),
            Ability::UsersManage => PermissionId::new("users:manage"),
            Ability::OrdersView => PermissionId::new("orders:view"),
            Ability::ReportsExport => PermissionId::new("reports:export"),
        }
    }
}
```

## Step 2: Define the Model with `Authenticatable`

```rust
// src/app/models/user.rs
use foundry::prelude::*;

#[derive(Clone, Debug, foundry::Model)]
#[foundry(model = "users")]
pub struct User {
    pub id: ModelId<User>,
    pub email: String,
    pub name: String,
    pub active: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[async_trait]
impl Authenticatable for User {
    fn guard() -> GuardId {
        GuardId::new("api")
    }

    async fn resolve_from_actor<E>(actor: &Actor, executor: &E) -> Result<Option<Self>>
    where
        E: QueryExecutor,
    {
        User::query()
            .where_(User::ID.eq(&actor.id))
            .where_(User::ACTIVE.eq(true))
            .first(executor)
            .await
    }
}
```

```rust
// src/app/models/admin_user.rs
use foundry::prelude::*;

#[derive(Clone, Debug, foundry::Model)]
#[foundry(model = "admin_users")]
pub struct AdminUser {
    pub id: ModelId<AdminUser>,
    pub email: String,
    pub name: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[async_trait]
impl Authenticatable for AdminUser {
    fn guard() -> GuardId {
        GuardId::new("admin")
    }

    async fn resolve_from_actor<E>(actor: &Actor, executor: &E) -> Result<Option<Self>>
    where
        E: QueryExecutor,
    {
        AdminUser::query()
            .where_(AdminUser::ID.eq(&actor.id))
            .first(executor)
            .await
    }
}
```

```rust
// src/app/models/merchant.rs
use foundry::prelude::*;

#[derive(Clone, Debug, foundry::Model)]
#[foundry(model = "merchants")]
pub struct Merchant {
    pub id: ModelId<Merchant>,
    pub user_id: ModelId<User>,
    pub business_name: String,
    pub status: MerchantStatus,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[async_trait]
impl Authenticatable for Merchant {
    fn guard() -> GuardId {
        GuardId::new("merchant")
    }

    async fn resolve_from_actor<E>(actor: &Actor, executor: &E) -> Result<Option<Self>>
    where
        E: QueryExecutor,
    {
        Merchant::query()
            .where_(Merchant::ID.eq(&actor.id))
            .where_(Merchant::STATUS.eq(MerchantStatus::Active))
            .first(executor)
            .await
    }
}
```

## Step 3: Implement a Custom Authenticator (or use StaticBearerAuthenticator for dev)

```rust
// src/app/authenticators/api_auth.rs
use foundry::prelude::*;

/// Validates user bearer tokens against the database.
/// Replace this with your own token validation logic (PAT, JWT, session, etc.)
pub struct ApiTokenAuthenticator {
    app: AppContext,
}

impl ApiTokenAuthenticator {
    pub fn new(app: AppContext) -> Self {
        Self { app }
    }
}

#[async_trait]
impl BearerAuthenticator for ApiTokenAuthenticator {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>> {
        // Example: look up token in a `personal_access_tokens` table
        let db = self.app.database()?;
        let record = db
            .raw_query(
                "SELECT user_id, abilities FROM personal_access_tokens WHERE token_hash = $1 AND revoked_at IS NULL",
                &[hash_token(token).into()],
            )
            .await?;

        match record.first() {
            Some(row) => {
                let user_id: String = row.get("user_id")?;
                let abilities: Vec<String> = row.get_json("abilities")?;
                Ok(Some(
                    Actor::new(user_id, GuardId::new("api"))
                        .with_permissions(abilities.into_iter().map(PermissionId::owned)),
                ))
            }
            None => Ok(None),
        }
    }
}
```

For development/testing, use the built-in `StaticBearerAuthenticator`:

```rust
StaticBearerAuthenticator::new()
    .token("dev-user-token", Actor::new("user-1", AuthGuard::Api)
        .with_permissions([Ability::UsersView, Ability::OrdersView]))
    .token("dev-admin-token", Actor::new("admin-1", AuthGuard::Admin)
        .with_permissions([Ability::UsersManage, Ability::ReportsExport]))
```

## Step 4: Register Everything in a Service Provider

```rust
// src/app/providers/auth.rs
use foundry::prelude::*;
use crate::app::ids::AuthGuard;
use crate::app::models::{User, AdminUser, Merchant};

pub struct AuthServiceProvider;

#[async_trait]
impl ServiceProvider for AuthServiceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        // 1. Register guard authenticators (how tokens are validated)
        registrar.register_guard(
            AuthGuard::Api,
            StaticBearerAuthenticator::new()
                .token("user-token", Actor::new("user-1", AuthGuard::Api)
                    .with_permissions([Ability::UsersView, Ability::OrdersView]))
                .token("user-2-token", Actor::new("user-2", AuthGuard::Api)),
        )?;

        registrar.register_guard(
            AuthGuard::Admin,
            StaticBearerAuthenticator::new()
                .token("admin-token", Actor::new("admin-1", AuthGuard::Admin)
                    .with_permissions([Ability::UsersManage, Ability::ReportsExport])),
        )?;

        registrar.register_guard(
            AuthGuard::Merchant,
            StaticBearerAuthenticator::new()
                .token("merchant-token", Actor::new("merchant-1", AuthGuard::Merchant)
                    .with_permissions([Ability::OrdersView])),
        )?;

        // 2. Register authenticatable models (how actors resolve to models)
        registrar.register_authenticatable::<User>()?;
        registrar.register_authenticatable::<AdminUser>()?;
        registrar.register_authenticatable::<Merchant>()?;

        // 3. Register policies (optional — for custom authorization logic)
        registrar.register_policy(PolicyKey::IsAdmin, AdminPolicy)?;

        Ok(())
    }
}
```

## Step 5: Define Routes with Guards

```rust
// src/app/portals/api/mod.rs
use foundry::prelude::*;
use crate::app::ids::{AuthGuard, Ability};

pub fn routes(r: &mut HttpRegistrar) -> Result<()> {
    // Public routes — no guard
    r.route("/health", get(health));

    // User routes — "api" guard
    r.route_with_options("/me", get(my_profile),
        HttpRouteOptions::new().guard(AuthGuard::Api),
    );

    r.route_with_options("/orders", get(list_orders),
        HttpRouteOptions::new()
            .guard(AuthGuard::Api)
            .permission(Ability::OrdersView),
    );

    Ok(())
}
```

```rust
// src/app/portals/admin/mod.rs
pub fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route_with_options("/admin/users", get(list_users),
        HttpRouteOptions::new()
            .guard(AuthGuard::Admin)
            .permission(Ability::UsersManage),
    );

    r.route_with_options("/admin/reports/export", post(export_report),
        HttpRouteOptions::new()
            .guard(AuthGuard::Admin)
            .permission(Ability::ReportsExport),
    );

    Ok(())
}
```

## Step 6: Write Handlers

```rust
// ── Recommended: Auth<M> extractor (one step, model ready) ──

async fn my_profile(Auth(user): Auth<User>) -> Result<Json<User>> {
    Ok(Json(user))
}

async fn list_users(Auth(admin): Auth<AdminUser>) -> Result<Json<Vec<User>>> {
    let db = admin.app.database()?;  // if you need more queries
    // ...
}

// ── When you need both actor metadata + model ──

async fn detailed_profile(
    CurrentActor(actor): CurrentActor,
    State(app): State<AppContext>,
) -> Result<Json<serde_json::Value>> {
    let user = actor.resolve::<User>(&app).await?
        .ok_or(Error::not_found("user"))?;

    Ok(Json(serde_json::json!({
        "user": user,
        "roles": actor.roles,
        "permissions": actor.permissions,
    })))
}

// ── When you just need the actor (no model resolution) ──

async fn whoami(CurrentActor(actor): CurrentActor) -> Result<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "id": actor.id,
        "guard": actor.guard,
    })))
}

// ── Optional auth (public route that behaves differently when logged in) ──

async fn homepage(actor: OptionalActor) -> Result<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "logged_in": actor.as_ref().is_some(),
        "actor_id": actor.as_ref().map(|a| &a.id),
    })))
}
```

## Step 7: Wire in main.rs

```rust
// src/main.rs
use foundry::prelude::*;

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(AuthServiceProvider)
        .register_routes(api::routes)
        .register_routes(admin::routes)
        .register_routes(merchant::routes)
        .run_http()
}
```

---

# Handler DX Reference

All extractors at a glance:

| Extractor | What you get | When to use |
|-----------|-------------|-------------|
| `Auth(user): Auth<User>` | `User` model | Most common — resolved automatically, like `$request->user()` |
| `CurrentActor(actor): CurrentActor` | `Actor` struct | When you need roles/permissions/claims, or want manual resolution |
| `OptionalActor(actor): OptionalActor` | `Option<Actor>` | Public routes where auth is optional |
| `actor.resolve::<User>(&app)` | `Result<Option<User>>` | Manual resolution from actor to model |
| `ctx.resolve_actor::<User>()` | `Result<Option<User>>` | WebSocket handler model resolution |

---

# WebSocket Integration

**Status: Done**

WebSocket channels use the same guard system as HTTP routes:

```rust
pub fn realtime(registrar: &mut WebSocketRegistrar) -> Result<()> {
    registrar.channel_with_options(
        CHAT_CHANNEL,
        handle_chat,
        WebSocketChannelOptions::new()
            .guard(AuthGuard::Api)
            .permission(Ability::WsChat),
    )?;

    registrar.channel_with_options(
        ADMIN_NOTIFICATIONS,
        handle_admin_notification,
        WebSocketChannelOptions::new()
            .guard(AuthGuard::Admin),
    )?;

    Ok(())
}

async fn handle_chat(ctx: WebSocketContext, payload: Value) -> Result<()> {
    let user = ctx.resolve_actor::<User>().await?
        .ok_or(Error::not_found("user"))?;

    ctx.publish("message", serde_json::json!({
        "from": user.name,
        "body": payload["body"],
    })).await
}
```

---

# Auth Config

```toml
# config/auth.toml
[auth]
default_guard = "api"
bearer_prefix = "Bearer"
```

- `default_guard` — used when a route has `.guard()` set to `Guarded` but no explicit guard ID
- `bearer_prefix` — the prefix before the token in the Authorization header

---

# Security Model

## Multi-Guard Isolation

Each guard is a completely separate authentication domain:

1. **Route level**: `.guard(AuthGuard::Admin)` tells the middleware to use the **admin** authenticator
2. **Authenticator level**: The admin authenticator only recognizes admin tokens — a user token is unknown → 401
3. **Extractor level**: `Auth<AdminUser>` checks `actor.guard == AdminUser::guard()` — guard mismatch → error

A User PAT **cannot** access Admin routes. Two separate walls.

## Permission Enforcement

Permissions are checked at the middleware level before the handler runs:

```rust
HttpRouteOptions::new()
    .guard(AuthGuard::Api)
    .permission(Ability::OrdersView)  // actor must have this permission
```

If the actor doesn't have the required permission → 403 Forbidden (never reaches the handler).

## Policy System

For complex authorization decisions beyond simple permission checks:

```rust
pub struct AdminPolicy;

#[async_trait]
impl Policy for AdminPolicy {
    async fn evaluate(&self, actor: &Actor, _app: &AppContext) -> Result<bool> {
        Ok(actor.has_role(RoleKey::Admin))
    }
}

// In handler:
let is_admin = app.authorizer()?
    .allows_policy(&actor, PolicyKey::IsAdmin)
    .await?;
```

---

# Internal Architecture

## Request Flow

```text
HTTP Request
    │
    ▼
Route matched → HttpRouteOptions has .guard("api")
    │
    ▼
http_auth_middleware runs:
    1. Extract bearer token from Authorization header
    2. Look up authenticator for guard "api"
    3. Call authenticator.authenticate(token) → Actor { id, guard: "api", ... }
    4. Check actor has required permissions
    5. Insert Actor + AppContext into request extensions
    │
    ▼
Handler runs:
    Auth<User> extractor:
        1. Pull Actor from extensions (delegates to CurrentActor)
        2. Validate actor.guard == User::guard()
        3. Call User::resolve_from_actor(actor, db) → User model
        4. Return Auth(user) ready for handler
```

## Key Types

| Type | Location | Purpose |
|------|----------|---------|
| `Actor` | `src/auth/mod.rs` | Lightweight auth payload |
| `GuardId` | `src/support/identifiers.rs` | Typed guard identifier |
| `BearerAuthenticator` | `src/auth/mod.rs` | Trait: token → Actor |
| `Authenticatable` | `src/auth/mod.rs` | Trait: Actor → Model |
| `AuthenticatedModel<M>` | `src/auth/mod.rs` | Extractor: combines auth + resolution |
| `Auth<M>` | `src/auth/mod.rs` | Alias for `AuthenticatedModel<M>` |
| `CurrentActor` | `src/auth/mod.rs` | Extractor: Actor from extensions |
| `OptionalActor` | `src/auth/mod.rs` | Extractor: Option<Actor> |
| `AuthManager` | `src/auth/mod.rs` | Guard registry + token validation |
| `Authorizer` | `src/auth/mod.rs` | Permission + policy checks |
| `AuthenticatableRegistry` | `src/auth/mod.rs` | Guard → model type-erased registry |
| `Policy` | `src/auth/mod.rs` | Trait: custom authorization logic |
| `StaticBearerAuthenticator` | `src/auth/mod.rs` | Dev/test: hardcoded token → actor map |

## Registration Flow

```text
App::builder()
    .register_provider(AuthServiceProvider)
         │
         ▼
    ServiceProvider::register(&mut registrar)
         │
         ├── registrar.register_guard(guard_id, authenticator)
         │       → GuardRegistryBuilder stores authenticator
         │
         ├── registrar.register_authenticatable::<User>()
         │       → AuthenticatableRegistryBuilder stores type-erased resolver
         │
         └── registrar.register_policy(policy_id, policy)
                 → PolicyRegistryBuilder stores policy
         │
         ▼
    Bootstrap freezes registries:
         ├── GuardRegistryBuilder → AuthManager (immutable)
         ├── AuthenticatableRegistryBuilder → AuthenticatableRegistry (immutable)
         └── PolicyRegistryBuilder → Authorizer (immutable)
         │
         ▼
    All stored as Arc singletons in Container
```

---

# Token Authentication (PAT)

**Status: Done — core implementation complete**

## Overview

Token-based authentication for API, mobile, and CLI clients. Uses short-lived access tokens + long-lived refresh tokens with rotation. Tokens stored as SHA-256 hashes in a `personal_access_tokens` database table.

Equivalent to Laravel Sanctum's token system.

## Config

```toml
# config/auth.toml

[auth]
default_guard = "api"
bearer_prefix = "Bearer"

[auth.guards.api]
driver = "token"        # framework auto-creates TokenAuthenticator for this guard

[auth.guards.admin]
driver = "token"        # each guard with driver = "token" uses the shared TokenManager

[auth.tokens]
access_token_ttl_minutes = 15       # short-lived, limits damage if stolen
refresh_token_ttl_days = 30         # long-lived, rotated on each use
token_length = 32                   # bytes of randomness per token
rotate_refresh_tokens = true        # old refresh token invalidated on use
```

## Database Schema

```sql
CREATE TABLE personal_access_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    guard TEXT NOT NULL,                      -- "api", "admin"
    actor_id TEXT NOT NULL,                   -- user.id / admin.id (TEXT for UUID or int)
    name TEXT NOT NULL DEFAULT '',            -- human label: "mobile-app", "cli", "My Laptop"
    access_token_hash TEXT NOT NULL,          -- SHA-256 hex of plaintext access token
    refresh_token_hash TEXT,                  -- SHA-256 hex of plaintext refresh token
    abilities JSONB DEFAULT '[]',            -- scoped permissions (future)
    expires_at TIMESTAMPTZ NOT NULL,          -- access token expiry
    refresh_expires_at TIMESTAMPTZ,           -- refresh token expiry
    last_used_at TIMESTAMPTZ,                 -- updated on each access token validation
    revoked_at TIMESTAMPTZ,                   -- soft-revoke (NULL = active)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Partial indexes: only non-revoked tokens for fast lookups
CREATE INDEX idx_pat_access_hash ON personal_access_tokens (access_token_hash) WHERE revoked_at IS NULL;
CREATE INDEX idx_pat_refresh_hash ON personal_access_tokens (refresh_token_hash) WHERE revoked_at IS NULL;
CREATE INDEX idx_pat_actor ON personal_access_tokens (guard, actor_id);
```

## TokenManager

The framework provides `TokenManager` as a singleton in the container, accessible via `app.tokens()`.

```rust
pub struct TokenManager {
    db: Arc<DatabaseManager>,
    config: TokenConfig,
}
```

### Public API

```rust
impl TokenManager {
    /// Issue a new access + refresh token pair.
    /// Plaintext tokens returned to the client; SHA-256 hashes stored in DB.
    pub async fn issue<M: Authenticatable>(&self, actor_id: &str) -> Result<TokenPair>

    /// Issue with a human-readable name (e.g., "My iPhone", "CLI").
    pub async fn issue_named<M: Authenticatable>(&self, actor_id: &str, name: &str) -> Result<TokenPair>

    /// Validate a refresh token and issue a new pair.
    /// If rotate_refresh_tokens is true, the old token row is revoked.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenPair>

    /// Revoke a specific access token.
    pub async fn revoke(&self, access_token: &str) -> Result<()>

    /// Revoke all tokens for an actor + guard. Returns count of revoked tokens.
    pub async fn revoke_all<M: Authenticatable>(&self, actor_id: &str) -> Result<u64>

    /// Validate an access token. Returns Actor if valid.
    /// Used internally by TokenAuthenticator.
    pub async fn validate(&self, access_token: &str) -> Result<Option<Actor>>
}
```

### TokenPair Response

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,         // seconds until access_token expires
    pub token_type: String,      // "Bearer"
}
```

### TokenAuthenticator

Implements `BearerAuthenticator`. Auto-created during bootstrap for guards with `driver = "token"`.

```rust
pub struct TokenAuthenticator {
    manager: Arc<TokenManager>,
}

#[async_trait]
impl BearerAuthenticator for TokenAuthenticator {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>> {
        self.manager.validate(token).await
    }
}
```

## Internal Flow

### Issue

1. Generate access token: `Token::base64(config.token_length)`
2. Generate refresh token: `Token::base64(config.token_length)`
3. Hash both: `sha256_hex_str(&access_plain)`, `sha256_hex_str(&refresh_plain)`
4. Insert row into `personal_access_tokens` with hashes, guard, actor_id, expiry timestamps
5. Return `TokenPair` with plaintext tokens (never stored on server)

### Validate (on every authenticated request)

1. Hash incoming token: `sha256_hex_str(token)`
2. Query: `SELECT * FROM personal_access_tokens WHERE access_token_hash = $1 AND revoked_at IS NULL AND expires_at > NOW()`
3. If found: update `last_used_at`, construct `Actor { id: row.actor_id, guard: GuardId::owned(row.guard) }`
4. If not found: return `None` → 401

### Refresh

1. Hash incoming refresh token
2. Query: `SELECT * WHERE refresh_token_hash = $1 AND revoked_at IS NULL AND refresh_expires_at > NOW()`
3. If `rotate_refresh_tokens`: set `revoked_at = NOW()` on old row
4. Issue new token pair (same `issue` flow) for same actor_id + guard
5. Return new `TokenPair`

### Revoke All (Logout)

1. `UPDATE personal_access_tokens SET revoked_at = NOW() WHERE guard = $1 AND actor_id = $2 AND revoked_at IS NULL`

## Consumer DX

### Login endpoint (consumer writes this):

```rust
async fn login(
    State(app): State<AppContext>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<TokenPair>> {
    let user = User::query()
        .where_(User::EMAIL.eq(&body.email))
        .first(&app).await?
        .ok_or(Error::unauthorized("invalid credentials"))?;

    if !app.hash()?.check(&body.password, &user.password)? {
        return Err(Error::unauthorized("invalid credentials"));
    }

    let pair = app.tokens()?.issue::<User>(&user.id.to_string()).await?;
    Ok(Json(pair))
}
```

### Refresh endpoint:

```rust
async fn refresh(
    State(app): State<AppContext>,
    Json(body): Json<RefreshRequest>,
) -> Result<Json<TokenPair>> {
    Ok(Json(app.tokens()?.refresh(&body.refresh_token).await?))
}
```

### Logout endpoint:

```rust
async fn logout(
    State(app): State<AppContext>,
    Auth(user): Auth<User>,
) -> Result<StatusCode> {
    app.tokens()?.revoke_all::<User>(&user.id.to_string()).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

### Routes:

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route("/auth/login", post(login));
    r.route("/auth/refresh", post(refresh));
    r.route_with_options("/auth/logout", post(logout),
        HttpRouteOptions::new().guard(AuthGuard::Api),
    );
    r.route_with_options("/me", get(profile),
        HttpRouteOptions::new().guard(AuthGuard::Api),
    );
    Ok(())
}
```

### Protected handler (unchanged from before):

```rust
async fn profile(Auth(user): Auth<User>) -> Result<Json<User>> {
    Ok(Json(user))
}
```

---

# Session Authentication

**Status: Done — core implementation complete**

## Overview

Session-based authentication for web dashboards. Uses HTTP-only cookies with Redis-backed session storage and sliding expiry.

Equivalent to Laravel's session-based auth.

## Config

```toml
[auth.guards.web]
driver = "session"

[auth.sessions]
ttl_minutes = 120                   # session lifetime
cookie_name = "foundry_session"       # cookie name
cookie_secure = true                # Secure flag (HTTPS only)
cookie_path = "/"
sliding_expiry = true               # auto-extend on activity
```

## SessionManager

```rust
pub struct SessionManager {
    redis: Arc<RedisManager>,
    config: SessionConfig,
}
```

### Public API

```rust
impl SessionManager {
    pub async fn create<M: Authenticatable>(&self, actor_id: &str) -> Result<String>
    pub async fn validate(&self, session_id: &str) -> Result<Option<Actor>>
    pub async fn destroy(&self, session_id: &str) -> Result<()>
    pub async fn destroy_all<M: Authenticatable>(&self, actor_id: &str) -> Result<()>
    pub fn login_response(&self, session_id: String, body: impl IntoResponse) -> Response
    pub fn logout_response(&self, body: impl IntoResponse) -> Response
}
```

### Redis Storage

- Session data: `session:{id}` → JSON `{ actor_id, guard }` with TTL
- Session index: `session_index:{guard}:{actor_id}` → SET of session IDs (for "logout everywhere")

### Guard Driver Dispatch

The `AuthManager` and `http_auth_middleware` will be extended to support both bearer and session-based guard drivers:

- Bearer guards: extract from `Authorization` header (current behavior)
- Session guards: extract session ID from `Cookie` header, validate via `SessionManager`

### Consumer DX

```rust
async fn web_login(
    State(app): State<AppContext>,
    Json(body): Json<LoginRequest>,
) -> Result<Response> {
    let admin = AdminUser::query()
        .where_(AdminUser::EMAIL.eq(&body.email))
        .first(&app).await?
        .ok_or(Error::unauthorized("invalid credentials"))?;

    if !app.hash()?.check(&body.password, &admin.password)? {
        return Err(Error::unauthorized("invalid credentials"));
    }

    let sessions = app.sessions()?;
    let session_id = sessions.create::<AdminUser>(&admin.id.to_string()).await?;
    Ok(sessions.login_response(session_id, Json(json!({"message": "ok"}))))
}
```

---

# Framework Prerequisites

**Status: Done**

These are shared utilities needed by both token and session auth:

## SHA-256 Utility

Currently private in `src/email/ses.rs`. Extract to `src/support/sha256.rs`:

```rust
pub fn sha256_hex(data: &[u8]) -> String
pub fn sha256_hex_str(s: &str) -> String
```

## Cookie Support

Add `axum-extra` dependency for `CookieJar` extractor. Provide session cookie helpers:

```rust
pub struct SessionCookie;

impl SessionCookie {
    pub fn build(name: &str, value: &str, secure: bool) -> Cookie
    pub fn clear(name: &str) -> Cookie
}
```

## Guard Driver Config

Expand `AuthConfig` to declare which driver each guard uses:

```rust
pub struct AuthConfig {
    pub default_guard: GuardId,
    pub bearer_prefix: String,
    pub tokens: TokenConfig,
    pub sessions: SessionConfig,
    pub guards: HashMap<String, GuardDriverConfig>,
}

pub struct GuardDriverConfig {
    pub driver: String,  // "token" | "session" | "custom"
}
```

Guards with `driver = "token"` get a `TokenAuthenticator` auto-registered during bootstrap.
Guards with `driver = "session"` get a `FoundrySessionAuthenticator` auto-registered.
Guards with `driver = "custom"` or no config entry are registered by the consumer in `ServiceProvider::register`.

---

# Security Model

## Token Security

| Concern | Solution |
|---------|----------|
| Token storage (server) | SHA-256 hash in DB — plaintext never persisted |
| Token storage (mobile) | Client stores refresh token in OS secure storage (Keychain / EncryptedSharedPrefs) |
| Token theft (access) | Short-lived (15min default), damage window limited |
| Token theft (refresh) | Rotation on use — old token invalidated, reuse detectable |
| DB leak | Hashed tokens — cannot be used even if DB is compromised |
| Logout | `revoke_all` sets `revoked_at` on all tokens for actor |

## Session Security

| Concern | Solution |
|---------|----------|
| XSS | HTTP-only cookie — JavaScript cannot access session ID |
| MITM | Secure flag — cookie only sent over HTTPS |
| CSRF | SameSite=Lax — cookie not sent on cross-origin requests |
| Session hijacking | Random 256-bit session ID, Redis-backed validation |
| Logout | DEL from Redis — instant invalidation |
| Logout everywhere | Session index SET per actor — destroy all sessions at once |

## Multi-Guard Isolation

Each guard is a separate authentication domain:

1. **Route level**: `.guard(AuthGuard::Admin)` tells middleware which authenticator to use
2. **Authenticator level**: admin authenticator only recognizes admin tokens → user token = 401
3. **Extractor level**: `Auth<AdminUser>` validates `actor.guard == AdminUser::guard()` → mismatch = error

---

# Implementation Phases

## Phase 1: Foundation ✅ Done
- [x] Extract SHA-256 utility to `src/support/sha256.rs`
- [x] Add `axum-extra` to Cargo.toml
- [x] Create cookie helpers at `src/http/cookie.rs`

## Phase 2: Config ✅ Done
- [x] Expand `AuthConfig` with `TokenConfig`, `SessionConfig`, `guards` map
- [x] Add `app.tokens()` accessor to `AppContext`

## Phase 3: Token Subsystem ✅ Done
- [x] Create `personal_access_tokens` migration
- [x] Create `src/auth/token.rs` with `TokenManager`, `TokenPair`, `TokenAuthenticator`
- [x] Wire `TokenManager` into bootstrap (auto-create for `driver = "token"` guards)
- [x] Export from `src/lib.rs` and `src/prelude.rs`
- [x] DB-backed acceptance coverage for issue/validate/refresh/revoke

## Phase 4: Session Subsystem ✅ Done
- [x] Create `src/auth/session.rs` with `SessionManager`
- [x] Extend guard registry with `GuardAuthenticator` enum (Bearer vs Session)
- [x] `AuthManager.authenticate_headers` handles both bearer and cookie-based session guards
- [x] Wire `SessionManager` into bootstrap (auto-create for `driver = "session"` guards)
- [x] `app.sessions()` accessor on `AppContext`
- [x] Export `SessionManager` from `lib.rs` and `prelude.rs`

---

# Beyond Token + Session Status

- **Rate limiting per actor** — ✅ Done (per-user rate limiting in HTTP middleware)
- **Audit trail** — ✅ Done (optional actor in ModelHookContext via AppTransaction::set_actor())
- **Token scoping/abilities** — ✅ Done (issue_with_abilities, abilities parsed into Actor permissions on validate)
- **Token pruning** — ✅ Done (token:prune CLI command, TokenManager::prune())

---

# Assumptions and Defaults

- Main authentication method is bearer token (non-JWT by design choice)
- Guards are config-driven (`driver = "token"` or `"session"`) with code-override support
- Guards are registered at boot time and immutable at runtime
- Each guard maps to exactly one authenticatable model (enforced at registration)
- Actor is request-scoped (stored in request extensions), not on AppContext
- `resolve_from_actor` runs a DB query per extraction — acceptable for request handlers
- `Auth<M>` is the recommended DX for handlers
- WebSocket uses the same guard system through `WebSocketChannelOptions`
- Token plaintext is never stored server-side — only SHA-256 hashes
- Session IDs are opaque random strings, not JWTs — validation always hits Redis for instant revocability
- Mobile clients store refresh tokens in OS-level secure storage — not the framework's concern

---

# One-Line Goal

> A Foundry app should configure auth guards in TOML, register authenticatable models in a provider, and get login/refresh/logout/session with `app.tokens()` and `app.sessions()` — then use `Auth(user): Auth<User>` in any handler.
