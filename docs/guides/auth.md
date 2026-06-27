# Auth & Guards Guide

Foundry provides dual-mode auth: **token-based** for APIs/mobile and **session-based** for web. Guards, permissions, and policies control access at different layers.

---

## Setup

### Step 1: Define Your Guards

Guards are named auth boundaries. Define them as a `FoundryId` enum — the
generated `GuardId` strings link to your TOML config keys:

```rust
#[derive(Clone, Copy, FoundryId)]
#[foundry(id = GuardId, rename_all = "snake_case")]
enum Guard {
    User,
    Admin,
}
```

Use `make:ids` to generate a starter `ids.rs` module with typed guard,
permission, and route IDs. Use `make:guard --name ApiGuard` to generate a
backend-owned guard id constant, `register(...)` helper, and
`BearerAuthenticator` implementation shell. The generated id removes a trailing
`Guard` suffix, so `ApiGuard` registers `GuardId::new("api")`. It returns
`Ok(None)` until you implement `authenticate(...)` and add the matching
`[auth.guards.api]` config entry.

### Step 2: Configure the Auth Driver

The config maps each guard name to its authentication mechanism:

```toml
# config/auth.toml
[auth]
default_guard = "user"

[auth.guards.user]       # ← matches GuardId::new("user")
driver = "token"         # authenticates via Authorization: Bearer <token>

[auth.guards.admin]      # ← matches GuardId::new("admin")
driver = "session"       # authenticates via session cookie
```

The link between code and config:

```
Code:  Guard::User  →  GuardId::new("user")
                                     │
Config:              [auth.guards.user]
                                  │
                        driver = "token"
```

### Step 3: Define Permissions and Policies

```rust
#[derive(Clone, Copy, AppEnum)]
#[foundry(id_type = PermissionId)]
enum Permission {
    #[foundry(key = "posts:read")]
    PostsRead,
    #[foundry(key = "posts:write")]
    PostsWrite,
    #[foundry(key = "posts:delete")]
    PostsDelete,
    #[foundry(key = "users:manage")]
    UsersManage,
}

#[derive(Clone, Copy)]
enum Policy {
    CanEditPost,
}

impl From<Policy> for PolicyId {
    fn from(v: Policy) -> Self {
        match v {
            Policy::CanEditPost => PolicyId::new("can_edit_post"),
        }
    }
}
```

### Step 4: Implement Authenticatable on Your Models

Each model declares which guard it authenticates through:

```rust
#[derive(Model)]
#[foundry(table = "users")]
pub struct User {
    pub id: ModelId<Self>,
    pub email: String,
    pub name: String,
    pub password_hash: String,
}

#[async_trait]
impl Authenticatable for User {
    fn guard() -> GuardId {
        Guard::User.into()
    }

    async fn resolve_from_actor<E: QueryExecutor>(
        actor: &Actor,
        executor: &E,
    ) -> Result<Option<Self>> {
        User::model_query()
            .where_col(User::ID, &actor.id)
            .first(executor)
            .await
    }
}

// Enable token issuance on User instances
impl HasToken for User {}
```

If your model follows the common `id: ModelId<Self>` pattern, you can usually omit `resolve_from_actor()` entirely and rely on Foundry's default primary-key lookup:

```rust
#[async_trait]
impl Authenticatable for User {
    fn guard() -> GuardId {
        Guard::User.into()
    }
}
```

The default resolver supports UUID, text, and integer primary keys. Override it only when you need custom eager loading, tenant scoping, active-status checks, or a non-standard lookup.

If you have a separate admin model:

```rust
#[derive(Model)]
#[foundry(table = "admins")]
pub struct Admin {
    pub id: ModelId<Self>,
    pub email: String,
}

#[async_trait]
impl Authenticatable for Admin {
    fn guard() -> GuardId {
        Guard::Admin.into()
    }

    async fn resolve_from_actor<E: QueryExecutor>(
        actor: &Actor,
        executor: &E,
    ) -> Result<Option<Self>> {
        Admin::model_query()
            .where_col(Admin::ID, &actor.id)
            .first(executor)
            .await
    }
}
```

### Step 5: Register in ServiceProvider

```rust
struct AppServiceProvider;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_authenticatable::<User>()?;
        registrar.register_authenticatable::<Admin>()?;
        registrar.register_policy(Policy::CanEditPost, CanEditPostPolicy)?;
        Ok(())
    }
}
```

Use `make:policy --name CanEditPost` to generate the policy id constant,
`register(...)` helper, and `Policy` implementation shell from one backend-owned
name. The generated policy denies by default until you implement
`evaluate(...)`.

`types:export` emits `AuthManifest.ts` from these guard, policy, and
authenticatable registrations. Frontend auth-state code can import
`AuthGuardIds`, `AuthPolicyIds`, `DefaultAuthGuard`, `authGuards()`,
`authDefaultGuardName()`, `authConfiguredDefaultGuardName()`,
`authDefaultGuardManifestEntry()`, `authGuardsByKind()`,
`authAuthenticatableGuards()`, `authGuardKind()`, and
`authGuardHasAuthenticatable()`. Runtime guard, policy, token-guard, and
guard-kind strings can be normalized with `authGuardNameOrNull()`,
`authPolicyNameOrNull()`, `authTokenGuardNameOrNull()`, and
`authGuardKindOrNull()` instead of copying guard or policy names,
default-guard lookups, or guard filters by hand. Regenerate frontend types after
adding or renaming guards, policies, or authenticatable models.
Generated auth selector helpers return cloned guard and policy metadata, so
admin UIs can add local display state without mutating the backend-owned
manifest.

### Step 6: Define Routes

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    // Public
    r.route("/posts", get(list_posts));

    // Requires User guard
    r.route_with_options("/posts", post(create_post),
        HttpRouteOptions::new()
            .guard(Guard::User)
            .permission(Permission::PostsWrite));

    // Requires Admin guard
    r.route_with_options("/admin/users", get(admin_users),
        HttpRouteOptions::new()
            .guard(Guard::Admin)
            .permission(Permission::UsersManage));

    // Auth endpoints (public)
    r.route("/auth/login", post(login));
    r.route("/auth/refresh", post(refresh));

    Ok(())
}
```

---

## Extractors

Use these in handler signatures to access the authenticated user:

### `Auth<M>` — Authenticated model (recommended)

Authenticates AND loads the model from the database in one step:

```rust
async fn profile(Auth(user): Auth<User>) -> impl IntoResponse {
    Json(json!({ "id": user.id, "email": user.email }))
}
```

Returns 401 if unauthenticated. Returns 404 if model not found in database.

### `CurrentActor` — Raw actor (when you don't need the model)

`types:export` emits the `Actor` TypeScript type automatically for raw actor
responses such as `/me` or auth-state endpoints. `roles`, `permissions`, and
`claims` are optional in TypeScript because Actor deserialization accepts older
payloads where those fields are omitted.

```rust
async fn whoami(CurrentActor(actor): CurrentActor) -> impl IntoResponse {
    Json(json!({
        "id": actor.id,
        "guard": actor.guard.to_string(),
        "permissions": actor.permissions.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
    }))
}
```

### `OptionalActor` — Public route with optional auth

```rust
async fn homepage(OptionalActor(maybe_actor): OptionalActor) -> impl IntoResponse {
    match maybe_actor.as_ref() {
        Some(actor) => Json(json!({ "greeting": format!("Hello, {}", actor.id) })),
        None => Json(json!({ "greeting": "Hello, guest" })),
    }
}
```

**When to use which:**

| Extractor | Auth required? | DB query? | Best for |
|-----------|---------------|-----------|----------|
| `Auth<User>` | Yes | Yes | Most endpoints — you need the user model |
| `CurrentActor` | Yes | No | When you only need ID/roles/permissions |
| `OptionalActor` | No | No | Public pages with optional personalization |

---

## Token Auth (APIs)

### Issuing Tokens

```rust
async fn login(
    State(app): State<AppContext>,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse> {
    let db = app.database()?;
    let user = User::model_query()
        .where_col(User::EMAIL, &body.email)
        .first(&*db).await?
        .ok_or_else(|| Error::not_found("invalid credentials"))?;

    let hash = app.hash()?;
    let password = body.password.clone();
    let password_hash = user.password_hash.clone();
    if !run_blocking("password check", move || hash.check(&password, &password_hash)).await? {
        return Err(Error::http(401, "invalid credentials"));
    }

    // Basic token
    let tokens = user.create_token(&app).await?;

    // Or: named token (for device identification)
    // let tokens = user.create_token_named(&app, "mobile-app").await?;

    // Or: scoped token (abilities become permissions on the Actor)
    // let tokens = user.create_token_with_abilities(&app, "ci-deploy", vec![
    //     "posts:read".into(),
    // ]).await?;

    Ok(Json(tokens))
}
```

Response:

```json
{
  "access_token": "foundry_abc123...",
  "refresh_token": "foundry_xyz789...",
  "expires_in": 900,
  "token_type": "Bearer"
}
```

Client sends: `Authorization: Bearer foundry_abc123...`

### Refreshing Tokens

```rust
async fn refresh(
    State(app): State<AppContext>,
    JsonValidated(body): JsonValidated<RefreshTokenRequest>,
) -> Result<impl IntoResponse> {
    let tokens = app.tokens()?;
    let new_pair = tokens.refresh(&body.refresh_token).await?;
    Ok(Json(TokenResponse::new(new_pair)))
}
```

Refreshing now preserves the original token name and scoped abilities automatically. Invalid refresh tokens also return a standardized auth error code (`invalid_refresh_token`) instead of a raw framework string.

`TokenResponse` also works well as a WebSocket payload for token-auth portals:

```rust
context.publish("auth.tokens", TokenResponse::new(tokens)).await?;
```

If a portal only needs a short-lived WebSocket auth token string, Foundry now also provides a tiny typed wrapper:

```rust
let ws_token = "ws_abc123";
context.publish("auth.ws_token", WsTokenResponse::new(ws_token)).await?;
```

### Revoking Tokens

```rust
// Revoke one token
app.tokens()?.revoke(&access_token).await?;

// Revoke all tokens for a user
app.tokens()?.revoke_all::<User>(&user_id).await?;

// Or from the model instance
user.revoke_all_tokens(&app).await?;

// Manual cleanup is still available; workers also prune automatically by default.
app.tokens()?.prune(30).await?;
```

### Token Config

```toml
[auth.tokens]
access_token_ttl_minutes = 15       # short-lived access token
refresh_token_ttl_days = 30         # long-lived refresh token
token_length = 32                   # random bytes in token
rotate_refresh_tokens = true        # issue new refresh token on refresh
prune_retention_days = 30           # auto-prune expired/revoked rows older than N days
prune_interval_ms = 3600000         # worker prune interval
prune_batch_size = 1000             # max rows deleted per pass

# Optional per-guard TTL overrides. Missing values inherit [auth.tokens].
[auth.tokens.guards.admin]
access_token_ttl_minutes = 43200    # 30 days
refresh_token_ttl_days = 30

[auth.tokens.guards.user]
access_token_ttl_minutes = 4320     # 3 days
refresh_token_ttl_days = 3
```

Set `prune_retention_days = 0` if an app-owned schedule should remain the only
token cleanup owner. Existing calls to `token:prune` and `app.tokens()?.prune(...)`
continue to work.

## Login Lockout

Use `LoginThrottle` in your login handler to enforce per-identifier failure tracking. This is
different from request rate limiting: failed logins increment the counter, successful logins reset
it.

```rust
use foundry::auth::lockout::LoginThrottle;

async fn login(
    State(app): State<AppContext>,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse> {
    let db = app.database()?;
    let throttle = LoginThrottle::new(&app)?;

    throttle.before_attempt(&body.email).await?;

    let user = User::model_query()
        .where_col(User::EMAIL, &body.email)
        .first(&*db).await?
        .ok_or_else(|| Error::http_with_code(401, "Invalid credentials", "invalid_credentials"))?;

    let hash = app.hash()?;
    let password = body.password.clone();
    let password_hash = user.password_hash.clone();
    if !run_blocking("password check", move || hash.check(&password, &password_hash)).await? {
        throttle.record_failure(&body.email).await?;
        return Err(Error::http_with_code(
            401,
            "Invalid credentials",
            "invalid_credentials",
        ));
    }

    throttle.record_success(&body.email).await?;
    Ok(Json(TokenResponse::new(user.create_token(&app).await?)))
}
```

When the threshold is exceeded, `before_attempt()` returns `LockoutError::LockedOut`, which maps to
an HTTP 429 response with the stable error code `login_locked_out`. Foundry also emits
`LoginLockedOutEvent` so you can notify security or write custom audit entries.

```toml
[auth.lockout]
enabled = true
max_failures = 5
lockout_minutes = 15
window_minutes = 15
```

---

## Auth Error Responses

Unauthorized and forbidden auth failures now return stable machine-friendly codes alongside the status:

```json
{
  "message": "Authentication credentials are required.",
  "status": 401,
  "error_code": "missing_auth_credentials",
  "message_key": "auth.missing_auth_credentials"
}
```

This keeps server responses consistent while giving clients a translation-friendly key for UI copy.
The generic `Error` path now preserves both `error_code` and `message_key` for
auth-originated failures too.
`AuthError::response_body()` returns the same backend-owned `ErrorResponse`
contract for custom guards or WebSocket auth surfaces that need the typed body.
`types:export` also emits `AuthErrorCode`, `AuthErrorCodeValues`,
`AuthErrorCodeKeys`, `getAuthErrorCodeValues()`,
`getAuthErrorCodeOptions()`, `getAuthErrorCodeMeta()`,
`getAuthErrorCodeKeys()`, and `isAuthErrorCode()` so clients can safely
narrow known auth failures while leaving app-specific
`ErrorResponse.error_code` values as strings.
Generated auth error-code values, options, metadata, and keys are frozen at
runtime, while AppEnum selector helpers return fresh values for local display
state.

---

## Session Auth (Web)

### Login / Logout

```rust
async fn web_login(
    State(app): State<AppContext>,
    Json(body): Json<LoginRequest>,
) -> Result<Response> {
    // ... verify credentials ...

    let sessions = app.sessions()?;
    let session_id = sessions.create::<Admin>(&admin.id.to_string()).await?;

    // Sets the session cookie and wraps the response body
    sessions.login_response(session_id, Json(json!({ "ok": true })))
}

async fn web_logout(
    State(app): State<AppContext>,
    CurrentActor(actor): CurrentActor,
) -> Result<Response> {
    let sessions = app.sessions()?;
    sessions.destroy_all::<Admin>(&actor.id).await?;

    // Clears the session cookie
    sessions.logout_response(Json(json!({ "ok": true })))
}
```

### Remember Me

```rust
let session_id = sessions.create_with_remember::<Admin>(&admin_id, remember_me).await?;
let response = sessions.login_response_with_remember(
    session_id,
    remember_me,
    Json(json!({ "ok": true })),
)?;
// remember_me = true  → uses remember_ttl_days (default: 30 days)
// remember_me = false → uses ttl_minutes (default: 120 min)
```

Session data is stored in Redis under Foundry's Redis namespace. Session IDs are
opaque URL-safe tokens; malformed cookie values are rejected before Redis lookup.
When `sliding_expiry` is enabled, normal sessions extend by `ttl_minutes` and
remember-me sessions extend by `remember_ttl_days`, so activity does not shorten
long-lived remembered sessions. Foundry also keeps the per-actor Redis session
index bounded with a TTL, so naturally expired sessions do not leave permanent
index keys behind. `login_response(...)` keeps a browser session cookie for
source compatibility; use `login_response_with_remember(...)` when the cookie
should persist with `Max-Age`.

### Session Config

```toml
[auth.sessions]
ttl_minutes = 120                   # session duration
cookie_name = "foundry_session"       # cookie name
cookie_secure = true                # HTTPS only
cookie_path = "/"                   # Set-Cookie Path used for login/logout
cookie_same_site = "lax"            # lax | strict | none; none requires secure cookies
cookie_domain = ""                  # optional Set-Cookie Domain
sliding_expiry = true               # extend TTL on activity
remember_ttl_days = 30              # "remember me" duration
```

`types:export` emits these browser-relevant session settings through
`AuthManifest.ts` as `AuthRuntimeManifest.sessions` plus helpers such as
`AuthSessionCookieName` and `authSessionCookieName()`. It does not export the
raw cookie domain value; use the generated metadata for frontend auth-state
tooling instead of copying session constants from TOML.
Runtime manifest export uses the same backend-effective values for trimmed
bearer prefixes, normalized cookie SameSite values, lockout clamps, MFA pending
token TTL clamps, recovery-code count clamps, and blank-MFA-issuer fallback to
`app.name`. Direct auth runtime descriptors must keep exported strings trimmed,
cookie settings valid, TTLs positive, and numeric values within JavaScript's
safe integer range.
Generated auth runtime selectors return cloned token/session/lockout/MFA
metadata, including required-role arrays, so auth-state tooling can annotate
those results locally.

## Multi-Factor Authentication (TOTP)

Foundry ships a first-party TOTP baseline with built-in handlers for enroll, confirm, verify,
disable, and recovery-code rotation. Publish the framework migrations before turning it on:
`types:export` emits `MfaCodeRequest`, `MfaEnrollChallenge`, `MfaRecoveryCodesRequest`, and `MfaRecoveryCodesResponse` automatically, so MFA setup and verification screens can import the backend-owned request and response contracts. `AuthManifest.ts` also includes `AuthRuntimeManifest.mfa` and `AuthMfaPendingTokenTtlMinutes` for the configured issuer, pending-token TTL, recovery-code count, and required-role map.
Generated MFA selector helpers clone required-role maps and role arrays before
returning them.

```bash
cargo run -- migrate:publish
```

Register the built-in handlers on the routes you want to expose:

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route_with_options(
        "/auth/mfa/enroll",
        post(foundry::auth::mfa::routes::enroll),
        HttpRouteOptions::new().guard(Guard::Admin),
    );
    r.route_with_options(
        "/auth/mfa/confirm",
        post(foundry::auth::mfa::routes::confirm),
        HttpRouteOptions::new().guard(Guard::Admin),
    );
    r.route_with_options(
        "/auth/mfa/verify",
        post(foundry::auth::mfa::routes::verify),
        HttpRouteOptions::new()
            .guard(Guard::Admin)
            .allow_mfa_pending_token(),
    );
    r.route_with_options(
        "/auth/mfa/disable",
        post(foundry::auth::mfa::routes::disable),
        HttpRouteOptions::new().guard(Guard::Admin),
    );
    r.route_with_options(
        "/auth/mfa/recovery-codes",
        post(foundry::auth::mfa::routes::recovery),
        HttpRouteOptions::new().guard(Guard::Admin),
    );
    Ok(())
}
```

For login flows, issue a short-lived pending token when the actor's roles require MFA:

```rust
use foundry::auth::mfa::MfaManager;

async fn admin_login(
    State(app): State<AppContext>,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse> {
    let admin = authenticate_admin(&app, &body).await?;
    let admin_roles = load_admin_roles(&app, &admin).await?;

    let actor = Actor::new(admin.id.to_string(), Guard::Admin)
        .with_roles(admin_roles);
    let mfa = MfaManager::new(&app)?;

    let tokens = if mfa.requires_mfa(&actor) {
        mfa.issue_pending_token(&actor, "admin-login").await?
    } else {
        mfa.issue_full_token(&actor, "admin-login").await?
    };

    Ok(Json(TokenResponse::new(tokens)))
}
```

Pending tokens carry the reserved `auth:mfa_pending` ability. Guarded routes reject them by
default; only routes marked with `allow_mfa_pending_token()` can accept them. The `/auth/mfa/verify`
handler exchanges a pending token for a normal full-access token pair.
Generated route metadata exposes those exceptions as `allowsMfaPendingToken`,
with helpers such as `routesAllowingMfaPendingToken()` for MFA route guards and
flow diagnostics.

```toml
[auth.mfa]
enabled = true
issuer = "foundry"
pending_token_ttl_minutes = 10
recovery_codes = 8

[auth.mfa.required_roles]
admin = ["developer", "super_admin"]
```

TOTP codes must be six ASCII digits. Recovery codes use Foundry's generated `xxxxx-yyyyy` shape,
are one-time use, and are consumed with an owner-checked database update so concurrent retries
cannot spend the same code twice. Repeated MFA failures reuse the built-in lockout backend under a
separate internal key.

---

## Permissions (Route-Level)

Permissions are checked automatically on guarded routes. The Actor must have ALL listed permissions:

```rust
// Requires auth + posts:write permission
r.route_with_options("/posts", post(create_post),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .permission(Permission::PostsWrite));

// Requires auth + BOTH permissions
r.route_with_options("/admin/reports", get(admin_reports),
    HttpRouteOptions::new()
        .guard(Guard::Admin)
        .permissions([Permission::UsersManage, Permission::PostsRead]));
```

**Where do Actor permissions come from?**

| Source | When |
|--------|------|
| Token abilities | `create_token_with_abilities(...)` — abilities become permissions |
| Manual on Actor | `Actor::new(...).with_permissions([...])` in your authenticator |
| Your database | Load from a roles/permissions table in `resolve_from_actor()` or your `BearerAuthenticator` |

If the Actor lacks the required permission, the framework returns **403 Forbidden** automatically — no handler code needed.

---

## Policies (Business-Logic)

Policies answer "can this actor do this specific thing?" — checked manually in handlers.
Start one with `make:policy --name CanEditPost`, then replace the generated
`Ok(false)` body with your business rule.

```rust
struct CanEditPostPolicy;

#[async_trait]
impl Policy for CanEditPostPolicy {
    async fn evaluate(&self, actor: &Actor, app: &AppContext) -> Result<bool> {
        // Check ownership or admin role
        if actor.has_role(RoleId::new("admin")) {
            return Ok(true);
        }

        let db = app.database()?;
        let owns_post = Post::model_query()
            .where_col(Post::AUTHOR_ID, &actor.id)
            .exists(&*db).await?;

        Ok(owns_post)
    }
}
```

Use in a handler:

```rust
async fn update_post(
    State(app): State<AppContext>,
    CurrentActor(actor): CurrentActor,
) -> Result<impl IntoResponse> {
    let can_edit = app.authorizer()?
        .allows_policy(&actor, Policy::CanEditPost).await?;

    if !can_edit {
        return Err(Error::http(403, "you cannot edit this post"));
    }

    // proceed...
}
```

**Permissions vs Policies:**

| | Permissions | Policies |
|--|------------|---------|
| Checked at | Route level (automatic) | Handler level (explicit) |
| Logic | Simple: does the Actor have this string? | Complex: async, can query DB |
| Fails with | 403 (automatic) | Your choice |
| Example | "Can access this endpoint?" | "Can edit THIS specific post?" |

---

## Password Reset

```rust
// 1. User requests reset
async fn forgot_password(State(app): State<AppContext>, Json(body): Json<ForgotRequest>) -> Result<impl IntoResponse> {
    let token = app.password_resets()?.create_token::<User>(&body.email).await?;
    // Send email with token (e.g. https://app.com/reset?email=...&token=...)
    Ok(Json(json!({ "message": "check your email" })))
}

// 2. User submits new password with token
async fn reset_password(State(app): State<AppContext>, Json(body): Json<ResetRequest>) -> Result<impl IntoResponse> {
    app.password_resets()?.validate_token::<User>(&body.email, &body.token).await?;
    // Token is single-use — deleted after validation. Update password now.
    Ok(Json(json!({ "message": "password updated" })))
}
```

Foundry stores password reset tokens in `password_reset_tokens`. Tokens are single
use and expire after `[auth.password_resets].expiry_minutes`. Workers prune
expired rows automatically; set `expiry_minutes = 0` to disable expiry and
automatic pruning.

## Email Verification

Same pattern:

```rust
let token = app.email_verification()?.create_token::<User>(&user.email).await?;
// Send verification email...

app.email_verification()?.validate_token::<User>(&email, &token).await?;
// Mark email as verified in your database
```

Email verification uses the same table with a `verify:` guard prefix. It has its
own expiry/prune controls under `[auth.email_verification]`.

---

## Custom Guard

For auth systems beyond token/session (JWT, OAuth, API keys), implement `BearerAuthenticator`:

```rust
struct JwtAuthenticator {
    secret: String,
}

#[async_trait]
impl BearerAuthenticator for JwtAuthenticator {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>> {
        let claims = decode_jwt(token, &self.secret)?;
        Ok(Some(
            Actor::new(claims.sub, Guard::User)
                .with_roles(claims.roles.iter().map(RoleId::owned))
                .with_permissions(claims.permissions.iter().map(PermissionId::owned))
                .with_claims(json!({ "org_id": claims.org_id }))
        ))
    }
}
```

Register it — takes precedence over config-driven auto-registration:

```rust
registrar.register_guard(Guard::User, JwtAuthenticator { secret: "...".into() })?;
```

---

## Static Auth (Testing)

For tests or prototyping without a database:

```rust
registrar.register_guard(Guard::User,
    StaticBearerAuthenticator::new()
        .token("admin-token",
            Actor::new("admin-1", Guard::User)
                .with_roles([RoleId::new("admin")])
                .with_permissions([Permission::PostsRead, Permission::PostsWrite, Permission::PostsDelete]))
        .token("reader-token",
            Actor::new("reader-1", Guard::User)
                .with_permissions([Permission::PostsRead]))
)?;
```

No database, no Redis, no config file. Works immediately.

---

## Guard Registration Precedence

During bootstrap, guards resolve in this order:

1. **Manual** (`register_guard()` in ServiceProvider or Plugin) — always wins
2. **Config-driven** (`[auth.guards.user] driver = "token"`) — only if not already registered manually
3. **Custom driver** (`driver = "custom"`) — never auto-registered, requires manual registration

This lets you use config for standard setups and override specific guards when you need custom logic.

---

## Full Config Reference

```toml
[auth]
default_guard = "user"              # used when route doesn't specify a guard
bearer_prefix = "Bearer"            # Authorization header prefix

[auth.tokens]
access_token_ttl_minutes = 15
refresh_token_ttl_days = 30
token_length = 32
rotate_refresh_tokens = true
prune_retention_days = 30
prune_interval_ms = 3600000
prune_batch_size = 1000

# [auth.tokens.guards.admin]
# access_token_ttl_minutes = 43200
# refresh_token_ttl_days = 30

[auth.sessions]
ttl_minutes = 120
cookie_name = "foundry_session"
cookie_secure = true
cookie_path = "/"
cookie_same_site = "lax"
cookie_domain = ""
sliding_expiry = true
remember_ttl_days = 30

[auth.password_resets]
expiry_minutes = 60
prune_interval_ms = 3600000
prune_batch_size = 1000

[auth.email_verification]
expiry_minutes = 1440
prune_interval_ms = 3600000
prune_batch_size = 1000

[auth.lockout]
enabled = true
max_failures = 5
lockout_minutes = 15
window_minutes = 15

[auth.mfa]
enabled = true
issuer = "foundry"
pending_token_ttl_minutes = 10
recovery_codes = 8

[auth.mfa.required_roles]
admin = ["developer", "super_admin"]

[auth.guards.user]
driver = "token"

[auth.guards.admin]
driver = "session"
```

The generated `AuthManifest.ts` mirrors the frontend-safe parts of this config:
default guard, bearer prefix, token/session TTLs, password-reset and
email-verification expiry, lockout policy, MFA settings, and per-guard token TTL
overrides. It intentionally omits token length, pruning intervals, pruning batch
sizes, and the raw session cookie domain.
