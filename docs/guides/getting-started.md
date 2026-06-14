# Getting Started

Every Foundry app starts with `App::builder()`. Configure it, register your services, routes, and commands, then pick a process type to run.

---

## Minimal HTTP App

```rust
use foundry::prelude::*;

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_routes(|r| {
            r.route("/health", get(|| async { Json(json!({ "ok": true })) }));
            Ok(())
        })
        .run_http()
}
```

```toml
# config/server.toml
[server]
host = "127.0.0.1"
port = 3000
```

That's a running HTTP server.

---

## AppBuilder API

Every method is chainable and returns `Self`:

### Configuration

```rust
App::builder()
    .load_env()                             // load .env file
    .load_config_dir("config")              // merge config/*.toml in lexical order
```

`config:publish` creates grouped TOML files such as `00-app.toml`, `10-http.toml`, and
`40-runtime.toml`. Environment variables still override the merged config with the same
double-underscore names, for example `DATABASE__URL`.

### Registration

```rust
    .register_provider(AppServiceProvider)   // dependency injection
    .register_plugin(MyPlugin)               // plugin with dependencies
    .register_routes(routes)                 // HTTP routes
    .register_commands(commands)             // CLI commands
    .register_schedule(schedules)            // cron/interval tasks
    .register_websocket_routes(ws_routes)    // WebSocket channels
    .register_validation_rule(id, rule)      // custom validation rules
```

### HTTP-Specific

```rust
    .register_middleware(MiddlewareConfig::from(Compression))
    .middleware_group("api", vec![...])
    .enable_observability()                  // health checks + OpenAPI
    .serve_spa("frontend/dist")              // SPA fallback for client-side routing
```

### Execution (pick one)

```rust
    .run_http()?;           // HTTP server
    .run_cli()?;            // CLI commands
    .run_scheduler()?;      // cron/interval scheduler
    .run_worker()?;         // background job worker
    .run_websocket()?;      // WebSocket server
```

---

## Five Process Types

A Foundry app can run as any of five process types from the same codebase. Each serves a different purpose.

### HTTP Server

REST API + optional SPA frontend.

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .register_routes(routes)
    .register_middleware(MiddlewareConfig::from(Compression))
    .enable_observability()
    .run_http()?;
```

```toml
[server]
host = "127.0.0.1"
port = 3000
```

Binds to port, serves HTTP, graceful shutdown on Ctrl+C.

### CLI Commands

One-off tasks, admin operations, data imports.

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .register_commands(commands)
    .run_cli()?;
```

```rust
fn commands(reg: &mut CommandRegistry) -> Result<()> {
    reg.command(
        CommandId::new("import:users"),
        Command::new("import:users").about("Import users from CSV"),
        |inv| async move {
            let db = inv.app().database()?;
            // import logic...
            Ok(())
        },
    )?;
    Ok(())
}
```

```bash
cargo run -- import:users
```

Executes the command and exits. Framework commands (`db:migrate`, `config:publish`, etc.) are included automatically.

### Background Worker

Processes jobs from the queue continuously.

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .run_worker()?;
```

```toml
[runtime]
worker_threads = 0        # 0 = Tokio default for Foundry-owned sync runners
max_blocking_threads = 0  # 0 = Tokio default blocking pool cap

[jobs]
queue = "default"
poll_interval_ms = 100
max_concurrent_jobs = 0    # 0 = unlimited
shutdown_timeout_ms = 30000 # 0 = abort active jobs immediately on shutdown
```

Polls the job queue, executes jobs with retry on failure or panic, and runs until shutdown signal. On shutdown, active jobs drain until `shutdown_timeout_ms`; aborted jobs recover through lease expiry. Scale by running multiple worker processes.

### Scheduler

Cron and interval tasks. Only one instance runs per cluster (Redis-based leadership).

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .register_schedule(schedules)
    .run_scheduler()?;
```

```rust
fn schedules(s: &mut ScheduleRegistry) -> Result<()> {
    s.daily(ScheduleId::new("cleanup"), |inv| async move {
        let db = inv.app().database()?;
        db.raw_execute("DELETE FROM logs WHERE created_at < NOW() - INTERVAL '30 days'", &[]).await?;
        Ok(())
    })?;
    Ok(())
}
```

```toml
[scheduler]
tick_interval_ms = 1000
leader_lease_ttl_ms = 5000
shutdown_timeout_ms = 30000
```

### WebSocket Server

Real-time channels with presence and broadcasting.

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .register_websocket_routes(ws_routes)
    .run_websocket()?;
```

```toml
[websocket]
host = "127.0.0.1"
port = 3010
path = "/ws"
```

Runs on a separate port from HTTP. Clients connect via `ws://host:3010/ws`.

### When to Use Which

| Process | Runs | Scales? | Use for |
|---------|------|---------|---------|
| HTTP | Long-lived | Yes (stateless) | REST API, SPA |
| CLI | One-off | N/A | Admin commands, migrations, imports |
| Worker | Long-lived | Yes (multiple) | Background jobs (email, processing) |
| Scheduler | Long-lived | No (one leader) | Cron jobs, cleanup, reports |
| WebSocket | Long-lived | Yes (Redis-synced) | Real-time channels, chat, live updates |

---

## Service Providers

Custom services and framework registrations live in `ServiceProvider`:

```rust
struct AppServiceProvider;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        // Phase 1: register services, jobs, events, guards
        registrar.singleton(StripeClient::new(&config.api_key))?;
        registrar.register_job::<SendWelcomeEmail>()?;
        registrar.register_authenticatable::<User>()?;
        registrar.register_guard(Guard::User, TokenAuthenticator::new())?;
        registrar.listen_event::<OrderPlaced, _>(NotifyWarehouse)?;
        Ok(())
    }

    async fn boot(&self, app: &AppContext) -> Result<()> {
        // Phase 2: run after all services registered, before serving
        // Use for: migrations, cache warming, external API connections
        Ok(())
    }
}
```

**register()** — declare what exists. No async service calls. Order: plugin providers → app providers.

**boot()** — initialize with full context. Can resolve any service. Order: plugin providers → plugin boot() → app providers.

### What You Register

| Method | What it registers |
|--------|-------------------|
| `singleton::<T>(value)` | Shared service instance |
| `factory::<T, F>(closure)` | New instance per resolve |
| `register_job::<J>()` | Background job handler |
| `listen_event::<E, L>(listener)` | Domain event listener |
| `register_guard(id, authenticator)` | Auth guard |
| `register_policy(id, policy)` | Authorization policy |
| `register_authenticatable::<M>()` | Model that can authenticate |
| `register_readiness_check(id, check)` | Health probe |
| `register_storage_driver(name, factory)` | Custom storage backend |
| `register_email_driver(name, factory)` | Custom email driver |
| `register_notification_channel(id, channel)` | Notification delivery channel |
| `register_datatable::<D>()` | Server-side datatable |
| `register_job_middleware(middleware)` | Job lifecycle hooks |

---

## AppContext

Inside handlers, jobs, schedules, and commands — `AppContext` is your gateway to everything:

```rust
async fn handler(State(app): State<AppContext>) -> Result<impl IntoResponse> {
    let db = app.database()?;
    let cache = app.cache()?;
    let email = app.email()?;
    let jobs = app.jobs()?;
    let redis = app.redis()?;
    let storage = app.storage()?;
    let ws = app.websocket()?;
    let auth = app.auth()?;
    let events = app.events()?;
    let i18n = app.i18n()?;
    let hash = app.hash()?;
    let crypt = app.crypt()?;
    let lock = app.lock()?;
    let clock = app.clock();
    let config = app.config();

    // Custom services
    let stripe = app.resolve::<StripeClient>()?;

    // Transactions
    let mut tx = app.begin_transaction().await?;
    // ... writes ...
    tx.dispatch_after_commit(SendConfirmation { order_id });
    tx.commit().await?;

    Ok(Json(json!({ "ok": true })))
}
```

---

## Project Structure

Recommended layout for a real project:

```
my-app/
├── Cargo.toml
├── build.rs                        # database codegen (migrations/seeders)
├── config/
│   ├── app.toml                    # app name, environment, timezone, shutdown
│   ├── server.toml                 # HTTP host/port
│   ├── database.toml               # PostgreSQL connection
│   ├── redis.toml                  # Redis connection
│   └── auth.toml                   # guards, tokens, sessions
├── database/
│   ├── migrations/                 # Rust migration files
│   └── seeders/                    # Rust seeder files
├── locales/                        # i18n JSON files
│   ├── en/
│   └── ms/
├── templates/
│   └── emails/                     # email templates
├── frontend/                       # SPA frontend (optional)
│   └── dist/
├── src/
│   ├── main.rs                     # entry point
│   ├── app/
│   │   ├── mod.rs
│   │   ├── providers/              # ServiceProviders
│   │   ├── portals/                # HTTP routes
│   │   ├── commands/               # CLI commands
│   │   ├── schedules/              # scheduled tasks
│   │   ├── realtime/               # WebSocket channels
│   │   └── domain/                 # business logic
│   │       ├── models/             # Model structs
│   │       ├── jobs/               # Background jobs
│   │       ├── events/             # Domain events + listeners
│   │       ├── notifications/      # Notification definitions
│   │       └── enums/              # AppEnum types
│   └── bootstrap/
│       ├── mod.rs
│       ├── app.rs                  # shared AppBuilder setup
│       ├── http.rs                 # HTTP runtime builder
│       ├── cli.rs                  # CLI runtime builder
│       ├── scheduler.rs            # scheduler runtime builder
│       ├── worker.rs               # worker runtime builder
│       └── websocket.rs            # WebSocket runtime builder
└── tests/
```

### The Bootstrap Pattern

Keep `main.rs` clean by sharing one base builder and adding runtime-specific registrations in separate files. This is the same shape used by Foundry's consumer fixture.

**src/bootstrap/app.rs:**

```rust
use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(app::providers::AppServiceProvider)
}
```

**src/bootstrap/http.rs:**

```rust
use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder()
        .register_routes(app::portals::router)
        .register_middleware(MiddlewareConfig::from(Compression))
        .enable_observability()
}
```

**src/bootstrap/cli.rs:**

```rust
use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder().register_commands(app::commands::register)
}
```

**src/bootstrap/scheduler.rs:**

```rust
use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder().register_schedule(app::schedules::register)
}
```

**src/bootstrap/worker.rs:**

```rust
use foundry::prelude::*;

pub fn builder() -> AppBuilder {
    super::app::builder()
}
```

**src/bootstrap/websocket.rs:**

```rust
use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder().register_websocket_routes(app::realtime::register)
}
```

**src/bootstrap/mod.rs:**

```rust
pub mod app;
pub mod cli;
pub mod http;
pub mod scheduler;
pub mod websocket;
pub mod worker;
```

**src/main.rs:**

```rust
mod app;
mod bootstrap;

fn main() -> foundry::foundation::Result<()> {
    match std::env::var("PROCESS").unwrap_or_default().as_str() {
        "worker" => bootstrap::worker::builder().run_worker(),
        "scheduler" => bootstrap::scheduler::builder().run_scheduler(),
        "websocket" => bootstrap::websocket::builder().run_websocket(),
        "cli" => bootstrap::cli::builder().run_cli(),
        "http" | "" => bootstrap::http::builder().run_http(),
        _ => bootstrap::http::builder().run_http(),
    }
}
```

Deploy as separate processes:

```bash
PROCESS=http     cargo run          # API server
PROCESS=worker   cargo run          # job processor
PROCESS=scheduler cargo run         # cron runner (one instance)
PROCESS=websocket cargo run         # WebSocket server
PROCESS=cli cargo run -- db:migrate # CLI command
```

---

## Graceful Shutdown

On Ctrl+C or SIGTERM:

1. Stop accepting new connections/jobs
2. Finish in-flight requests and drain active jobs/schedules up to their configured shutdown timeouts
3. Call `plugin.shutdown()` for each plugin in reverse dependency order
4. Exit cleanly

No special code needed — the framework handles this automatically. Process-manager hard kills such as SIGKILL cannot be caught; unacked jobs recover through lease expiry.

---

## What's Next

| Guide | When you need |
|-------|--------------|
| [Database](database.md) | Models, queries, relations, migrations |
| [Auth & Guards](auth.md) | Token/session auth, permissions, policies |
| [Routes & Middleware](routes-and-middleware.md) | Route groups, CORS, CSRF, rate limiting |
| [Validation](validation.md) | Request validation rules |
| [Background Processing](background-processing.md) | Jobs, scheduler, events |
| [Email & Notifications](email-and-notifications.md) | Send emails, multi-channel notifications |
| [WebSocket](websocket.md) | Real-time channels, presence |
| [Storage & Imaging](storage-and-imaging.md) | File uploads, image processing |
| [Caching & Redis](caching-and-redis.md) | Cache abstraction, Redis client |
| [Datatable](datatable.md) | Server-side data tables |
| [Model Extensions](model-extensions.md) | Attachments, metadata, translations, countries, settings, enums |
| [i18n](i18n.md) | Internationalization |
| [Plugins](plugins.md) | Plugin development |
