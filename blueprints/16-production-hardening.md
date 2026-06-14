# Foundry Production Hardening Blueprint

> **Status:** ✅ Complete
> **Created:** 2026-04-12
> **Completed:** 2026-04-13
> **Purpose:** Critical production-readiness fixes — small scope, high impact.

---

# What Already Exists

- Request context middleware logs method/path/status/duration/request_id per request
- `ValidationErrors::into_response()` returns structured `{message, status, errors: [{field, code, message}]}`
- `Validated<T>` extractor converts `ValidationErrors` directly to response (bypasses `Error` type)
- Rate limit headers (`x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset`) already on both success and 429 responses
- `RouteRegistry` with named routes and URL generation
- `sha256_hex` / `sha256_hex_str` utilities
- `CryptManager` with AES-256-GCM encryption (key not retrievable after construction)
- `RuntimeBackend::set_if_absent(key, ttl)` for SETNX (but stores hardcoded `1`, not arbitrary values)
- `CommandRegistry` with clap-based CLI command registration
- DB pool with `min_connections`, `max_connections`, `acquire_timeout`

---

# Phase 1: Structured Error Responses

## Problem

`From<ValidationErrors> for Error` (error.rs:91-97) converts structured validation errors into a flat `Error::Http { message: errors.to_string() }`, losing the field-level error array. This only affects handlers using the `?` operator on validation results — the `Validated<T>` extractor already returns structured errors correctly. The `ErrorResponse` struct only has `{message, status}` — no `error_code` field for programmatic client handling.

## Solution

### 1.1 Add `error_code` and optional `errors` to `ErrorResponse`

**File:** `src/foundation/error.rs`

```rust
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub message: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<crate::validation::FieldError>>,
}
```

Note: `request_id` stays in the `x-request-id` response header only (already implemented). Not duplicated in body.

### 1.2 Add `Validation` variant to `Error` enum

Keep the structured field errors instead of flattening:

```rust
pub enum Error {
    Message(String),
    Http { status: u16, message: String, error_code: Option<String> },
    NotFound(String),
    Validation(crate::validation::ValidationErrors),
    Other(#[from] anyhow::Error),
}
```

`IntoResponse` for `Validation` delegates to `ValidationErrors::into_response()` to ensure a single consistent 422 format (same as the `Validated<T>` extractor path).

### 1.3 Constructors

```rust
impl Error {
    // Existing
    pub fn http(status: u16, message: impl Into<String>) -> Self {
        Self::Http { status, message: message.into(), error_code: None }
    }

    // New: with stable error code for programmatic client handling
    pub fn http_with_code(status: u16, message: impl Into<String>, code: impl Into<String>) -> Self {
        Self::Http { status, message: message.into(), error_code: Some(code.into()) }
    }
}
```

**Files:** `src/foundation/error.rs`

---

# Phase 2: DB Pool Lifecycle

## Problem

Missing `max_lifetime`, `idle_timeout` on pool connections. In production with PgBouncer or cloud Postgres, connections go stale after load balancer timeouts (typically 30 min). sqlx defaults are `None` for both, meaning connections live forever.

## Solution

**File:** `src/config/mod.rs`

Add to `DatabaseConfig`:
```rust
pub idle_timeout_seconds: u64,     // default: 600 (10 min)
pub max_lifetime_seconds: u64,     // default: 1800 (30 min)
```

**File:** `src/database/runtime.rs`

Pass to `PgPoolOptions` (both write and read replica pools):
```rust
let pool = PgPoolOptions::new()
    .min_connections(config.min_connections)
    .max_connections(config.max_connections)
    .acquire_timeout(Duration::from_millis(config.acquire_timeout_ms))
    .idle_timeout(Duration::from_secs(config.idle_timeout_seconds))
    .max_lifetime(Duration::from_secs(config.max_lifetime_seconds))
    .connect(&config.url)
    .await?;
```

---

# Phase 3: Distributed Locking

## Problem

No user-facing distributed lock API. Workers processing payments, generating reports, or rebuilding caches need mutual exclusion.

## Solution

### 3.1 New RuntimeBackend methods

**File:** `src/support/runtime.rs`

The existing `set_if_absent` stores hardcoded `1`. Lock acquisition needs to store an owner ID (UUID) and release needs to verify ownership. Add two new methods:

```rust
impl RuntimeBackend {
    /// SET key value NX EX ttl — returns true if set, false if key exists.
    pub async fn set_nx_value(&self, key: &str, value: &str, ttl_secs: u64) -> Result<bool> { ... }

    /// DEL key only if current value matches expected — returns true if deleted.
    /// Redis: Lua script for atomicity. Memory: Mutex check-and-delete.
    pub async fn del_if_value(&self, key: &str, expected: &str) -> Result<bool> { ... }
}
```

Redis `del_if_value` Lua script:
```lua
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
end
return 0
```

### 3.2 DistributedLock API

**File:** `src/support/lock.rs` (new)

```rust
pub struct DistributedLock {
    backend: Arc<RuntimeBackend>,
}

impl DistributedLock {
    /// Attempt to acquire a lock. Returns a guard that releases on drop.
    pub async fn acquire(&self, key: &str, ttl: Duration) -> Result<Option<LockGuard>> { ... }

    /// Block until the lock is acquired (with timeout).
    pub async fn block(&self, key: &str, ttl: Duration, wait_timeout: Duration) -> Result<LockGuard> { ... }
}

pub struct LockGuard { ... }
// Drop releases via tokio::spawn (fire-and-forget background task)
```

**Internal:**
- Owner ID: random UUID per acquisition (prevents releasing someone else's lock)
- `acquire`: calls `set_nx_value(key, owner, ttl)`. Returns `Some(guard)` if true, `None` if false.
- `block`: retries `acquire` with 100ms sleep intervals until success or timeout.
- `LockGuard::drop`: spawns `del_if_value(key, owner)` background task.

**DX:**
```rust
if let Some(guard) = app.lock()?.acquire("payment:123", Duration::from_secs(30)).await? {
    process_payment(123).await?;
    // guard auto-releases on drop
}

// Or blocking:
let guard = app.lock()?.block("report:daily", Duration::from_secs(60), Duration::from_secs(10)).await?;
```

**Files:** `src/support/lock.rs` (new), `src/support/runtime.rs`, `src/support/mod.rs`, `src/foundation/app.rs`, `src/lib.rs`, `src/prelude.rs`

---

# Phase 4: Maintenance Mode

## Problem

During deployments or migrations, need to return 503 to all requests while allowing specific bypass tokens through.

## Solution

**File:** `src/http/middleware.rs`

Add `MaintenanceMode` middleware variant:

```rust
#[derive(Clone, Debug)]
pub struct MaintenanceMode {
    bypass_secret: Option<String>,
}

impl MaintenanceMode {
    pub fn new() -> Self { Self { bypass_secret: None } }

    pub fn bypass_secret(mut self, secret: impl Into<String>) -> Self {
        self.bypass_secret = Some(secret.into());
        self
    }

    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::MaintenanceMode(self)
    }
}
```

**Internal:** Middleware checks `RuntimeBackend` for a `maintenance:active` key. If present and no valid bypass, returns 503:

```json
{ "message": "Service is undergoing maintenance", "status": 503 }
```

Bypass: `?bypass=SECRET` query param or `X-Maintenance-Bypass: SECRET` header.

**CLI commands:** Register `down` and `up` via `CommandRegistry` (using the existing clap `Command` pattern in `src/cli/mod.rs`):
- `foundry down [--secret=...]` — sets `maintenance:active` key in RuntimeBackend
- `foundry up` — deletes the key

**Files:** `src/http/middleware.rs`, `src/config/mod.rs`, `src/foundation/app.rs`, `src/cli/mod.rs`

---

# Phase 5: Signed Routes

## Problem

No framework-level signed URLs. Needed for email verification links, password reset links, unsubscribe links — any action that must be verifiable without authentication.

## Solution

### 5.1 HMAC-SHA256 utility

**File:** `src/support/hmac.rs` (new)

Use proper HMAC-SHA256 (not plain `sha256(url + key)` which is vulnerable to length-extension attacks). Requires `hmac` crate dependency.

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub fn hmac_sha256_hex(key: &[u8], message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    hex_encode(&mac.finalize().into_bytes())
}
```

### 5.2 Signing key config

**File:** `src/config/mod.rs`

Add a dedicated signing key to `AppConfig` (separate from the encryption key — signing and encryption keys should be independent):

```rust
pub struct AppConfig {
    pub timezone: Timezone,
    pub signing_key: String,  // base64-encoded, required for signed routes
}
```

### 5.3 Signed route methods

**File:** `src/http/routes.rs` (extend existing `RouteRegistry`)

```rust
impl RouteRegistry {
    /// Generate a signed URL with expiry.
    pub fn signed_url(
        &self,
        name: &str,
        params: &[(&str, &str)],
        signing_key: &[u8],
        expires_at: DateTime,
    ) -> Result<String> {
        let mut url = self.url(name, params)?;
        let expiry = expires_at.as_chrono().timestamp();
        let sep = if url.contains('?') { "&" } else { "?" };
        url = format!("{url}{sep}expires={expiry}");
        let signature = crate::support::hmac::hmac_sha256_hex(signing_key, &url);
        Ok(format!("{url}&signature={signature}"))
    }

    /// Verify a signed URL. Returns error if invalid or expired.
    pub fn verify_signature(url: &str, signing_key: &[u8]) -> Result<()> {
        // 1. Extract and remove `&signature=...` from URL
        // 2. Recompute HMAC over the remaining URL
        // 3. Constant-time compare
        // 4. Check `expires` param against current time
        ...
    }
}
```

**AppContext shortcut:**
```rust
pub fn signed_route_url(&self, name: &str, params: &[(&str, &str)], expires_at: DateTime) -> Result<String> {
    let registry = self.resolve::<RouteRegistry>()?;
    let signing_key = self.config().app()?.signing_key_bytes()?;
    registry.signed_url(name, params, &signing_key, expires_at)
}
```

**Files:** `src/support/hmac.rs` (new), `src/http/routes.rs`, `src/config/mod.rs`, `src/foundation/app.rs`, `Cargo.toml` (add `hmac` crate)

---

# Implementation Order

| Phase | Item | Scope | Key Files | Status |
|-------|------|-------|-----------|--------|
| 1 | Structured error responses | `Error` enum + `ErrorResponse` | `src/foundation/error.rs` | ✅ Done |
| 2 | DB pool lifecycle | 2 config fields + 2 pool options | `src/config/mod.rs`, `src/database/runtime.rs` | ✅ Done |
| 3 | Distributed locking | New module + 2 RuntimeBackend methods | `src/support/lock.rs`, `src/support/runtime.rs`, `src/foundation/app.rs` | ✅ Done |
| 4 | Maintenance mode | Middleware + CLI commands | `src/http/middleware.rs`, `src/http/mod.rs` | ✅ Done |
| 5 | Signed routes | HMAC utility + RouteRegistry extension | `src/support/hmac.rs`, `src/http/routes.rs`, `src/config/mod.rs` | ✅ Done |

---

# Assumptions

- `request_id` stays in response header only (already implemented) — not duplicated in error body
- Rate limit headers already exist on success responses — no work needed
- Request access logging already exists in `request_context_middleware` — logs method/path/status/duration/request_id
- `Error::Validation`'s `IntoResponse` delegates to `ValidationErrors::into_response()` for format consistency with the `Validated<T>` extractor
- Distributed lock requires two new `RuntimeBackend` methods (`set_nx_value`, `del_if_value`)
- Signed routes use a dedicated `signing_key` config, not the encryption key (separation of concerns)
- Signed routes use proper HMAC-SHA256 (not plain SHA-256 concatenation)
- Maintenance mode flag stored in `RuntimeBackend` (Redis key for multi-instance, in-memory for single)
- CLI `down`/`up` commands use existing `CommandRegistry` infrastructure
