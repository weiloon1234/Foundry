<p align="center">
  <h1 align="center">Foundry</h1>
  <p align="center">A strongly-typed Rust backend framework built for thin apps and thick infrastructure.</p>
</p>

<p align="center">
  <a href="https://crates.io/crates/foundry"><img src="https://img.shields.io/crates/v/foundry.svg" alt="crates.io"></a>
  <a href="https://docs.rs/foundry"><img src="https://docs.rs/foundry/badge.svg" alt="docs.rs"></a>
  <img src="https://img.shields.io/badge/rust-1.94%2B-orange.svg" alt="MSRV">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

---

Foundry is a modular Rust backend framework built on **Axum**, **Tokio**, and **SQLx**. Your app code focuses on bootstrap, registration, and domain logic. Foundry owns the runtime, orchestration, infrastructure wiring, and cross-cutting concerns.

## Features

- **5 Runtime Kernels** &mdash; HTTP, CLI, Scheduler, Worker, WebSocket &mdash; each an independent async runtime
- **AST-First Database** &mdash; typed models, relations, projections, eager loading, cursor pagination, streaming, upsert, window functions
- **Auth System** &mdash; bearer tokens, sessions, guards, policies, roles, permissions, password reset, email verification
- **Validation** &mdash; 38+ built-in rules, custom rules, request validation extractor, file validation
- **Background Jobs** &mdash; leased at-least-once delivery with batching, chaining, rate limiting
- **Email** &mdash; multi-driver: SMTP, Mailgun, Postmark, Resend, SES, Log
- **Storage** &mdash; local + S3, multipart uploads, image processing pipeline
- **WebSocket** &mdash; channel-based with presence, typed events, replay
- **Plugin System** &mdash; compile-time registry with dependency resolution, direct registration of any framework feature, shutdown lifecycle, assets, scaffolds
- **Observability** &mdash; structured logging, readiness/liveness probes, runtime diagnostics, OpenTelemetry
- **Typed Everything** &mdash; `ModelId<M>`, `GuardId`, `JobId`, `ChannelId`, etc. &mdash; zero raw-string IDs

## Quick Start

Add Foundry to your `Cargo.toml`:

```toml
[dependencies]
foundry = "0.1"
```

Create a minimal HTTP server:

```rust
use foundry::prelude::*;

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_routes(routes)
        .run_http()
}

fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route("/health", get(health));
    Ok(())
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}
```

## Architecture

```
Your App                         Foundry Framework
--------                         ---------------
main.rs
  App::builder()
    .register_provider(...) ---> +----------------------+
    .register_routes(...)   ---> | AppBuilder           |
    .register_commands(...) ---> | - ServiceRegistrar   |
    .register_schedule(...) ---> | - HttpRegistrar      |
    .run_http()             ---> | - CommandRegistry    |
                                 | - ScheduleRegistry   |
                                 | - HttpKernel::serve  |
                                 +----------+-----------+
                                            |
                                            v
                                 +----------------------+
                                 | AppContext           |
                                 | - Database           |
                                 | - Redis              |
                                 | - Auth               |
                                 | - Storage            |
                                 | - Email              |
                                 | - Cache              |
                                 | - Jobs               |
                                 | - WebSocket          |
                                 | - Events             |
                                 | - I18n               |
                                 +----------------------+
```

## Runtime Kernels

Foundry provides 5 independent async runtimes. Each is started from the same `AppBuilder`:

```rust
// HTTP server
App::builder().register_routes(routes).run_http()?;

// CLI commands
App::builder().register_commands(commands).run_cli()?;

// Background job worker
App::builder().run_worker()?;

// Cron + interval scheduler (Redis-safe leadership)
App::builder().register_schedule(schedules).run_scheduler()?;

// WebSocket server
App::builder().register_websocket_routes(ws_routes).run_websocket()?;
```

## Database

Foundry ships an AST-first query system. Queries are built as expression trees, then compiled to SQL &mdash; not string concatenation.

### Models

```rust
#[derive(Clone, Copy, AppEnum)]
enum UserStatus {
    Pending,     // DB: "pending" (TEXT)
    Active,      // DB: "active"
    Suspended,   // DB: "suspended"
}

#[derive(Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<Self>,
    name: String,
    email: String,
    status: UserStatus,  // stored as TEXT, auto serde + OpenAPI + validation
    created_at: DateTime,
}

// Query
let user = User::model_query()
    .where_col(User::EMAIL, "alice@example.com")
    .first(&db)
    .await?;

// Create
User::model_create()
    .set(User::NAME, "Alice")
    .set(User::EMAIL, "alice@example.com")
    .set(User::STATUS, UserStatus::Pending)
    .execute(&db)
    .await?;
```

### Relations

```rust
let posts = has_many(User::ID, Post::AUTHOR_ID, |u| u.id, |u, posts| u.posts = Loaded(posts));

let users = User::model_query()
    .with(posts)
    .all(&db)
    .await?;
```

### Projections, CTEs, Window Functions

```rust
let report = ProjectionQuery::<MonthlySales>::new()
    .group_by(MonthlySales::MONTH)
    .having(MonthlySales::TOTAL, ComparisonOp::Gte, 1000)
    .all(&db)
    .await?;
```

## Auth

```rust
// Guard + policy registration
registrar.register_guard(AuthGuard::Api, StaticBearerAuthenticator::new()
    .token("secret-token", Actor::new("user-1", AuthGuard::Api)));
registrar.register_policy(PolicyKey::IsAdmin, AdminPolicy);

// Route with auth
r.route_with_options("/admin", get(admin_handler),
    HttpRouteOptions::new()
        .guard(AuthGuard::Api)
        .permission(Ability::AdminAccess));

// Extract authenticated actor in handler
async fn admin_handler(CurrentActor(actor): CurrentActor) -> impl IntoResponse {
    Json(serde_json::json!({ "user": actor.id }))
}
```

## Validation

38+ built-in rules with async database checks. Use `#[derive(Validate)]` to generate validation from attributes, or implement `RequestValidator` manually for full control:

```rust
#[derive(Deserialize, ApiSchema, Validate)]
#[validate(
    messages(email(unique = "This email is already registered")),
    attributes(email = "email address")
)]
struct CreateUser {
    #[validate(required, email, unique("users", "email"))]
    email: String,

    #[validate(required, min_length(8))]
    password: String,

    #[validate(required, confirmed)]
    password_confirmation: String,

    #[validate(required, app_enum)]
    status: UserStatus,  // AppEnum auto-validates + auto-generates OpenAPI schema
}

// Use in handler — auto-validates, returns 422 on failure
async fn create_user(Validated(payload): Validated<CreateUser>) -> impl IntoResponse {
    // payload is validated — status is a valid UserStatus, email is unique
}
```

## Background Jobs

```rust
#[derive(Debug, Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: String }

#[async_trait]
impl Job for SendWelcomeEmail {
    const ID: JobId = JobId::new("send_welcome_email");

    async fn handle(&self, ctx: JobContext) -> Result<()> {
        let email = ctx.app().email()?;
        // send email...
        Ok(())
    }
}

// Dispatch
app.jobs()?.dispatch(SendWelcomeEmail { user_id: "123".into() })?;

// Dispatch with delay
app.jobs()?.dispatch_later(job, DateTime::now().add_seconds(60).timestamp_millis())?;

// Batch
app.jobs()?.batch("onboard")
    .add(SendWelcomeEmail { .. })?
    .add(CreateDefaultSettings { .. })?
    .on_complete(NotifyAdmin { .. })?
    .dispatch()?;
```

## Middleware

```rust
App::builder()
    .register_middleware(MiddlewareConfig::from(
        Cors::default().allow_any_origin().credential(false)
    ))
    .register_middleware(MiddlewareConfig::from(Compression))
    .register_middleware(MiddlewareConfig::from(
        SecurityHeaders::default().hsts(31536000).csp("default-src 'self'")
    ))
    .register_middleware(MiddlewareConfig::from(
        RateLimit::per_minute(60).by_ip()
    ))
    .run_http()?;
```

## WebSocket

```rust
const CHAT: ChannelId = ChannelId::new("chat");
const MESSAGE: ChannelEventId = ChannelEventId::new("message");

struct ChatHandler;

#[async_trait]
impl ChannelHandler for ChatHandler {
    async fn handle(&self, ctx: WebSocketContext, payload: Value) -> Result<()> {
        ctx.publish(MESSAGE, payload).await
    }
}

fn ws_routes(r: &mut WebSocketRegistrar) -> Result<()> {
    r.channel_with_options(CHAT, ChatHandler,
        WebSocketChannelOptions::new()
            .presence(true)
            .guard(AuthGuard::Api))?;
    Ok(())
}
```

## Plugin System

Plugins can register any framework feature directly &mdash; routes, guards, jobs, middleware, event listeners, and more. No ServiceProvider wrapper needed.

```rust
struct AnalyticsPlugin;

impl Plugin for AnalyticsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("analytics", Version::new(1, 0, 0), VersionReq::parse(">=0.1").unwrap())
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar.register_routes(analytics::routes);
        registrar.register_guard(AnalyticsGuard::Api, AnalyticsAuthenticator);
        registrar.register_job::<AnalyticsFlushJob>();
        registrar.listen_event::<PageView, _>(TrackPageView);
        registrar.register_middleware(MiddlewareConfig::from(AnalyticsHeaders));
        Ok(())
    }

    async fn shutdown(&self, app: &AppContext) -> Result<()> {
        // Flush pending analytics on graceful shutdown
        Ok(())
    }
}

// Register at bootstrap — plugins are loaded in dependency order
App::builder()
    .register_plugin(AnalyticsPlugin)
    .run_http()?;
```

Plugins support dependency resolution (topological sort with cycle detection), SemVer version constraints, config defaults, asset distribution, and scaffold templating. Use `plugin:list` to inspect registered plugins and their contributions.

## Full Example: Model to API

A complete flow showing how enums, models, validation, OpenAPI, and routes work together with minimal boilerplate:

```rust
// ── Enums (one derive gives you DB + serde + OpenAPI + validation) ──

#[derive(Clone, Copy, AppEnum)]
enum UserStatus {
    Pending,       // DB: TEXT "pending",  OpenAPI: {"type":"string","enum":["pending","active","suspended"]}
    Active,
    Suspended,
}

#[derive(Clone, Copy, AppEnum)]
enum Priority {
    Low = 1,       // DB: INT4 1,  OpenAPI: {"type":"integer","enum":[1,2,3]}
    Medium = 2,
    High = 3,
}

// ── Model ──

#[derive(Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<Self>,
    email: String,
    name: String,
    status: UserStatus,
    created_at: DateTime,
}

// ── Request DTO (validation + OpenAPI from one struct) ──

#[derive(Deserialize, ApiSchema, Validate)]
#[validate(messages(email(unique = "Already registered")))]
struct CreateUserRequest {
    #[validate(required, email, unique("users", "email"))]
    email: String,

    #[validate(required, min_length(2))]
    name: String,

    #[validate(required, min_length(8))]
    password: String,

    #[validate(required, confirmed)]
    password_confirmation: String,

    #[validate(required, app_enum)]
    status: UserStatus,   // auto-validated + auto-documented in OpenAPI
}

// ── Response DTO ──

#[derive(Serialize, ApiSchema)]
struct UserResponse {
    id: ModelId<User>,
    email: String,
    name: String,
    status: UserStatus,   // OpenAPI schema auto-resolved from AppEnum
}

// ── Route with OpenAPI documentation ──

fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.api_version(1, |r| {
        r.scope("/users", |users| {
            users
                .name_prefix("users")
                .guard(AuthGuard::Api)
                .tag("users");

            users.post("", "store", create_user, |route| {
                route.summary("Create user");
                route.request::<CreateUserRequest>();
                route.response::<UserResponse>(201);
            });

            Ok(())
        })?;

        Ok(())
    })?;
    Ok(())
}

// ── Handler (validated, type-safe) ──

async fn create_user(
    Validated(req): Validated<CreateUserRequest>,
    State(app): State<AppContext>,
) -> Result<impl IntoResponse> {
    let db = app.database()?;
    let user = User::model_create()
        .set(User::EMAIL, &req.email)
        .set(User::NAME, &req.name)
        .set(User::STATUS, req.status)
        .execute(&*db).await?;

    Ok((StatusCode::CREATED, Json(UserResponse {
        id: user.id, email: user.email,
        name: user.name, status: user.status,
    })))
}
```

**What each derive provides &mdash; zero manual wiring:**

| Derive | Gives you |
|--------|-----------|
| `AppEnum` | DB column type, serde serialization, OpenAPI schema, validation rule, optional typed ID conversion |
| `Model` | Typed columns, query builders, create/update/delete, lifecycle hooks |
| `ApiSchema` | JSON Schema for OpenAPI (auto-resolves nested AppEnum fields) |
| `Validate` | Request validation rules, custom messages, `Validated<T>` extractor |

## Configuration

TOML-based with environment variable overlay:

```bash
# Generate a sample config
cargo run -- config:publish

# Generate .env.example with all overridable variables
cargo run -- env:publish

# Generate signing + encryption keys
cargo run -- key:generate

# Publish framework migrations
cargo run -- migrate:publish

# Publish framework seeders
cargo run -- seed:publish
```

`config:publish` now emits the framework-owned `[auth.lockout]` and `[auth.mfa]` sections.
`env:publish` emits the matching `AUTH__LOCKOUT__*`, `AUTH__MFA__*`, and
`AUTH__MFA__REQUIRED_ROLES__<GUARD>` overrides. Built-in audit logging is code-driven: mark the
admin route tree with `audit_area("admin")`, and unmarked routes will not produce audit rows.
There is no global audit config section anymore. Error reporters are registered in code with
`AppBuilder::register_error_reporter*()`, so they do not appear in the generated config yet.

Environment variables override config using double-underscore notation:

```bash
DATABASE__URL=postgres://... SERVER__PORT=8080 cargo run
```

Database config supports primary/read endpoints (`DATABASE__URL` and `DATABASE__READ_URL`),
per-pool overrides (`DATABASE__WRITE_POOL__MAX_CONNECTIONS`, `DATABASE__READ_POOL__MAX_CONNECTIONS`),
and lazy pools (`DATABASE__CONNECT_LAZY=true`) for serverless Postgres or provider poolers.

## Redis

```rust
async fn remember_login(app: &AppContext, user_id: &str) -> Result<()> {
    let redis = app.redis()?;
    let key = redis.key(format!("logins:{user_id}"));
    let mut conn = redis.connection().await?;
    conn.set_ex(&key, "1", 3600).await?;
    Ok(())
}
```

All keys are automatically namespaced. For cross-app access:

```rust
let foreign_key = redis.key_in_namespace("analytics:prod", "daily:users");
```

Redis config accepts both `redis://` and TLS `rediss://` endpoints.

## Service Providers

```rust
struct AppServiceProvider;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.singleton::<MyService>(MyService::new())?;
        registrar.register_guard(AuthGuard::Api, my_authenticator)?;
        registrar.register_job::<SendWelcomeEmail>()?;
        registrar.listen_event::<UserCreated, _>(OnUserCreated)?;
        Ok(())
    }
}
```

## Built-in CLI Commands

| Command | Description |
|---------|-------------|
| **Setup** | |
| `config:publish` | Publish sample configuration, including auth lockout, MFA, and audit sections |
| `env:publish` | Generate `.env.example` with all supported env overrides, including lockout, MFA, and audit |
| `key:generate` | Generate 32-byte signing and encryption keys |
| `migrate:publish` | Publish framework migration files, including audit log and MFA tables |
| `seed:publish` | Publish framework seeder files |
| `about` | Display framework version and environment info |
| **Database** | |
| `db:migrate` | Run pending migrations; accepts `--lock-timeout-ms <MS>` |
| `db:migrate:status` | Show migration status; accepts `--json` |
| `db:rollback` | Rollback the last migration batch; accepts `--lock-timeout-ms <MS>` |
| `db:seed` | Run database seeders |
| `seed:countries` | Seed 250 built-in country records |
| **Scaffolding** | |
| `make:migration` | Create a new migration file |
| `make:seeder` | Create a new seeder file |
| `make:model` | Create a new model file; accepts `--path <DIR>` |
| `make:job` | Create a new job file; accepts `--path <DIR>` |
| `make:command` | Create a new command file; accepts `--path <DIR>` |
| **Runtime** | |
| `doctor` | Run runtime health checks; accepts `--deploy`, `--json`, and `--strict` |
| `down` | Put the application into maintenance mode |
| `up` | Bring the application out of maintenance mode |
| `routes:list` | List all registered routes |
| `token:prune` | Prune expired personal access tokens |
| **Plugins** | |
| `plugin:list` | List registered plugins |
| `plugin:install-assets` | Install plugin assets |
| `plugin:scaffold` | Run a plugin scaffold |
| **Documentation** | |
| `docs:api` | Generate API surface docs |

## Examples

| Example | Description |
|---------|-------------|
| [blueprint_http](examples/blueprint_http.rs) | Minimal HTTP + CLI + scheduler |
| [blueprint_typed](examples/blueprint_typed.rs) | Typed IDs, auth, events, jobs, WebSocket |
| [phase2_websocket](examples/phase2_websocket.rs) | WebSocket channels |
| [phase25_auth](examples/phase25_auth.rs) | Auth guards and policies |
| [phase3_observability](examples/phase3_observability.rs) | Diagnostics and probes |
| [phase3_redis](examples/phase3_redis.rs) | Redis with namespacing |
| [phase3_database_generic](examples/phase3_database_generic.rs) | Generic query builder |
| [phase3_database_model](examples/phase3_database_model.rs) | Typed model queries |
| [phase3_database_relations](examples/phase3_database_relations.rs) | Relations and eager loading |
| [phase3_database_projection](examples/phase3_database_projection.rs) | Projections, CTEs, UNION |
| [phase3_database_many_to_many](examples/phase3_database_many_to_many.rs) | Many-to-many with pivots |
| [phase4_database_lifecycle](examples/phase4_database_lifecycle.rs) | Migrations and seeders |
| [phase3_plugin](examples/phase3_plugin.rs) | Plugin system |

## Requirements

- **Rust 1.94+**
- **PostgreSQL** (primary database target)
- **Redis** (optional &mdash; required for distributed scheduler, jobs, cache, locks)

## Development

```bash
# Full verification (same as CI)
make verify

# Run tests
make test

# Postgres acceptance tests
FOUNDRY_TEST_POSTGRES_URL=postgres://... make test-postgres

# Generate API surface docs
make api-docs

# Pre-release
make verify-release
```

## Documentation

| Resource | Description |
|----------|-------------|
| [API Surface](docs/api/index.md) | Auto-generated public API reference (per module) |
| [Public API Contract](docs/api/public-api-contract.md) | Import layers and compatibility rules for consumer apps |
| [API Reference](docs/api-reference.md) | Hand-curated API reference with context |
| [Getting Started](docs/guides/getting-started.md) | AppBuilder, 5 process types, project structure |
| [Recipes](docs/guides/recipes.md) | Production readiness, CRUD, queued email, uploads, datatables, plugins |
| [CLI Commands](docs/guides/cli-commands.md) | Define commands with arguments and flags |
| [Database Guide](docs/guides/database.md) | Models, relations, queries, projections, migrations |
| [Auth Guide](docs/guides/auth.md) | Token/session auth, guards, permissions, policies |
| [Routes & Middleware](docs/guides/routes-and-middleware.md) | Routing, middleware stack, CORS, CSRF, rate limiting |
| [Email & Notifications](docs/guides/email-and-notifications.md) | Multi-driver email, multi-channel notifications |
| [Caching & Redis](docs/guides/caching-and-redis.md) | Cache abstraction, namespaced Redis client |
| [Storage & Imaging](docs/guides/storage-and-imaging.md) | File uploads, local + S3, image processing |
| [Background Processing](docs/guides/background-processing.md) | Jobs, scheduler, domain events |
| [WebSocket](docs/guides/websocket.md) | Channels, rooms, presence, broadcasting |
| [Model Extensions](docs/guides/model-extensions.md) | AppEnum, Attachments, Metadata, Translations, Countries |
| [i18n](docs/guides/i18n.md) | Translation catalogs, locale resolution |
| [Plugin Guide](docs/guides/plugins.md) | 5 real-world plugin use cases with full code |
| [Datatable Guide](docs/guides/datatable.md) | Server-side datatables: filtering, sorting, export |
| [Validation Guide](docs/guides/validation.md) | 38+ rules, custom rules, request validation |
| [CONTRIBUTING](CONTRIBUTING.md) | Contributor workflow and expectations |
| [CHANGELOG](CHANGELOG.md) | Release history |
| [Release Checklist](docs/release-checklist.md) | Release procedure |

## License

MIT
