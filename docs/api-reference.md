# Foundry API Reference

> 1:1 mirror of `src/` — every public struct, enum, trait, function, type alias, and constant.

---

## Table of Contents

- [foundation/](#foundation)
- [kernel/](#kernel)
- [config/](#config)
- [support/](#support)
- [database/](#database)
  - [database/ast](#databaseast)
  - [database/model](#databasemodel)
  - [database/query](#databasequery)
  - [database/relation](#databaserelation)
  - [database/projection](#databaseprojection)
  - [database/aggregate](#databaseaggregate)
  - [database/collection_ext](#databasecollection_ext)
  - [database/extensions](#databaseextensions)
  - [database/runtime](#databaseruntime)
  - [database/compiler](#databasecompiler)
  - [database/lifecycle](#databaselifecycle)
- [auth/](#auth)
  - [auth/token](#authtoken)
  - [auth/session](#authsession)
  - [auth/password_reset](#authpassword_reset)
  - [auth/email_verification](#authemail_verification)
- [http/](#http)
  - [http/middleware](#httpmiddleware)
  - [http/cookie](#httpcookie)
  - [http/resource](#httpresource)
  - [http/routes](#httproutes)
- [websocket/](#websocket)
- [validation/](#validation)
- [email/](#email)
- [storage/](#storage)
- [jobs/](#jobs)
- [scheduler/](#scheduler)
- [events/](#events)
- [notifications/](#notifications)
- [cache/](#cache)
- [redis/](#redis)
- [logging/](#logging)
- [plugin/](#plugin)
- [datatable/](#datatable)
- [i18n/](#i18n)
- [translations/](#translations)
- [cli/](#cli)
- [testing/](#testing)
- [metadata/](#metadata)
- [openapi/](#openapi)
- [app_enum/](#app_enum)
- [attachments/](#attachments)
- [countries/](#countries)
- [imaging/](#imaging)

---

## foundation/

Core bootstrapping: app builder, context, DI container, error handling.

### Structs

| Name | Summary |
|------|---------|
| `App` | Entry point — exposes `builder() -> AppBuilder` |
| `AppBuilder` | Fluent builder for configuring and launching the app |
| `AppContext` | Central DI container — access to all framework services |
| `AppTransaction` | Active database transaction with after-commit callbacks |
| `Container` | Dependency injection container |
| `ServiceRegistrar` | Registers services, jobs, events, guards, policies during bootstrap |
| `ErrorResponse` | JSON error response body |

### Enums

| Name | Variants |
|------|----------|
| `Error` | `Message(String)`, `Http { status, message, error_code, message_key }`, `Validation(ValidationErrors)`, `NotFound(String)`, `Other(anyhow::Error)` |

### Traits

```rust
trait ServiceProvider: Send + Sync + 'static {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()>;
    async fn boot(&self, app: &AppContext) -> Result<()>; // default no-op
}
```

### Type Aliases

```rust
type Result<T> = std::result::Result<T, Error>;
```

### Error — constructors

```rust
Error::message(message: impl Into<String>) -> Self           // 500
Error::http(status: u16, message: impl Into<String>) -> Self  // custom status
Error::http_with_code(status, message, code) -> Self           // custom + error_code
Error::http_with_metadata(status, message, error_code, message_key) -> Self
Error::not_found(message: impl Into<String>) -> Self           // 404
Error::other<E: Into<anyhow::Error>>(error: E) -> Self         // 500
```

### AppBuilder — methods

```rust
fn new() -> Self
fn load_env(self) -> Self
fn load_config_dir(self, path: impl Into<PathBuf>) -> Self
fn serve_spa(self, dir: impl Into<PathBuf>) -> Self

// Registration
fn register_plugin<P: Plugin>(self, plugin: P) -> Self
fn register_plugins<I, P>(self, plugins: I) -> Self
fn register_provider<P: ServiceProvider>(self, provider: P) -> Self
fn register_routes<F>(self, registrar: F) -> Self
fn register_commands<F>(self, registrar: F) -> Self
fn register_schedule<F>(self, registrar: F) -> Self
fn register_websocket_routes<F>(self, registrar: F) -> Self
fn register_validation_rule<I, R>(self, id: I, rule: R) -> Self
fn register_middleware(self, config: MiddlewareConfig) -> Self
fn middleware_group(self, name: &str, middlewares: Vec<MiddlewareConfig>) -> Self
fn enable_observability(self) -> Self
fn enable_observability_with(self, options: ObservabilityOptions) -> Self

// Run (sync + async variants)
fn run_http(self) -> Result<()>
async fn run_http_async(self) -> Result<()>
fn run_cli(self) -> Result<()>
async fn run_cli_async(self) -> Result<()>
fn run_scheduler(self) -> Result<()>
async fn run_scheduler_async(self) -> Result<()>
fn run_worker(self) -> Result<()>
async fn run_worker_async(self) -> Result<()>
fn run_websocket(self) -> Result<()>
async fn run_websocket_async(self) -> Result<()>

// Build kernels directly
async fn build_http_kernel(self) -> Result<HttpKernel>
async fn build_cli_kernel(self) -> Result<CliKernel>
async fn build_scheduler_kernel(self) -> Result<SchedulerKernel>
async fn build_worker_kernel(self) -> Result<WorkerKernel>
async fn build_websocket_kernel(self) -> Result<WebSocketKernel>
```

### AppContext — methods

```rust
// Core
fn container(&self) -> &Container
fn config(&self) -> &ConfigRepository
fn timezone(&self) -> Result<Timezone>
fn clock(&self) -> Clock
fn rules(&self) -> &RuleRegistry
fn resolve<T: Send + Sync + 'static>(&self) -> Result<Arc<T>>

// Service accessors
fn events(&self) -> Result<Arc<EventBus>>
fn auth(&self) -> Result<Arc<AuthManager>>
fn authorizer(&self) -> Result<Arc<Authorizer>>
fn jobs(&self) -> Result<Arc<JobDispatcher>>
fn websocket(&self) -> Result<Arc<WebSocketPublisher>>
fn database(&self) -> Result<Arc<DatabaseManager>>
fn redis(&self) -> Result<Arc<RedisManager>>
fn storage(&self) -> Result<Arc<StorageManager>>
fn email(&self) -> Result<Arc<EmailManager>>
fn hash(&self) -> Result<Arc<HashManager>>
fn crypt(&self) -> Result<Arc<CryptManager>>
fn diagnostics(&self) -> Result<Arc<RuntimeDiagnostics>>
fn i18n(&self) -> Result<Arc<I18nManager>>
fn plugins(&self) -> Result<Arc<PluginRegistry>>
fn datatables(&self) -> Result<Arc<DatatableRegistry>>
fn authenticatables(&self) -> Result<Arc<AuthenticatableRegistry>>
fn tokens(&self) -> Result<Arc<TokenManager>>
fn sessions(&self) -> Result<Arc<SessionManager>>
fn password_resets(&self) -> Result<Arc<PasswordResetManager>>
fn email_verification(&self) -> Result<Arc<EmailVerificationManager>>
fn cache(&self) -> Result<Arc<CacheManager>>
fn lock(&self) -> Result<Arc<DistributedLock>>

// Transactions
async fn begin_transaction(&self) -> Result<AppTransaction>
async fn with_model_batching<F, T>(&self, future: F) -> T

// Notifications
async fn notify(notifiable: &dyn Notifiable, notification: &dyn Notification) -> Result<()>
async fn notify_queued(notifiable: &dyn Notifiable, notification: &dyn Notification) -> Result<()>

// URL generation
fn route_url<I: Into<RouteId>>(name: I, params: &[(&str, &str)]) -> Result<String>
fn signed_route_url<I: Into<RouteId>>(name: I, params: &[(&str, &str)], expires_at: DateTime) -> Result<String>
fn verify_signed_url(url: &str) -> Result<()>

// Plugin lifecycle
async fn shutdown_plugins(&self) -> Result<()>
```

### AppTransaction — methods

```rust
fn app(&self) -> &AppContext
fn transaction(&self) -> &DatabaseTransaction
fn set_actor(&mut self, actor: Actor)
fn actor(&self) -> Option<&Actor>
fn dispatch_after_commit<J: Job>(&self, job: J)
fn notify_after_commit(&self, notifiable: &dyn Notifiable, notification: &dyn Notification)
fn after_commit<F, Fut>(&self, callback: F)
async fn commit(self) -> Result<()>
async fn rollback(self) -> Result<()>
```

### Container — methods

```rust
fn new() -> Self
fn singleton<T: Send + Sync + 'static>(value: T) -> Result<()>
fn singleton_arc<T: Send + Sync + 'static>(value: Arc<T>) -> Result<()>
fn factory<T, F>(factory: F) -> Result<()>
fn factory_arc<T, F>(factory: F) -> Result<()>
fn resolve<T: Send + Sync + 'static>() -> Result<Arc<T>>
fn contains<T: 'static>() -> bool
```

### ServiceRegistrar — methods

```rust
fn container(&self) -> &Container
fn config(&self) -> &ConfigRepository
fn singleton<T>(value: T) -> Result<()>
fn singleton_arc<T>(value: Arc<T>) -> Result<()>
fn factory<T, F>(factory: F) -> Result<()>
fn resolve<T>() -> Result<Arc<T>>
fn listen_event<E: Event, L: EventListener<E>>(listener: L) -> Result<()>
fn register_job<J: Job>() -> Result<()>
fn register_job_middleware<M: JobMiddleware>(middleware: M) -> Result<()>
fn register_guard<I, G>(id: I, guard: G) -> Result<()>
fn register_policy<I, P>(id: I, policy: P) -> Result<()>
fn register_authenticatable<M: Authenticatable>() -> Result<()>
fn register_readiness_check<I, C>(id: I, check: C) -> Result<()>
fn register_storage_driver(name: &str, factory: StorageDriverFactory) -> Result<()>
fn register_email_driver(name: &str, factory: EmailDriverFactory) -> Result<()>
fn register_notification_channel<I, N>(id: I, channel: N) -> Result<()>
fn register_datatable<D: Datatable>() -> Result<()>
```

---

## kernel/

5 independent async runtimes.

### Structs

| Name | Summary |
|------|---------|
| `HttpKernel` | Axum HTTP server |
| `BoundHttpServer` | HTTP server bound to a socket, ready to serve |
| `CliKernel` | Clap CLI dispatcher |
| `SchedulerKernel` | Cron + interval task executor |
| `WorkerKernel` | Background job processor |
| `WebSocketKernel` | WebSocket channel server |
| `BoundWebSocketServer` | WebSocket server bound to a socket |

### HttpKernel

```rust
fn new(app, routes, middlewares, observability, spa_dir) -> Self
fn app(&self) -> &AppContext
fn build_router(&self) -> Result<Router>
async fn bind(self) -> Result<BoundHttpServer>
async fn serve(self) -> Result<()>
```

### CliKernel

```rust
fn new(app, registrars) -> Self
fn app(&self) -> &AppContext
fn build_registry(&self) -> Result<CommandRegistry>
async fn run(self) -> Result<()>
async fn run_with_args<I, T>(self, args: I) -> Result<()>
```

### SchedulerKernel

```rust
fn new(app, registry) -> Result<Self>
fn app(&self) -> &AppContext
async fn tick(&self) -> Result<Vec<ScheduleId>>
async fn tick_at(&self, now: DateTime) -> Result<Vec<ScheduleId>>
async fn run_once(&self) -> Result<Vec<ScheduleId>>
async fn run_once_at(&self, now: DateTime) -> Result<Vec<ScheduleId>>
```

### WorkerKernel

```rust
fn new(app) -> Result<Self>
fn app(&self) -> &AppContext
async fn run(self) -> Result<()>
async fn run_once(&self) -> Result<bool>
```

### WebSocketKernel

```rust
fn new(app, routes) -> Self
fn app(&self) -> &AppContext
async fn bind(self) -> Result<BoundWebSocketServer>
async fn serve(self) -> Result<()>
```

---

## config/

TOML-based configuration with environment overlay.

### Structs

| Name | Summary |
|------|---------|
| `ConfigRepository` | Loads and queries TOML config |
| `AppConfig` | `name`, `environment`, `timezone`, `signing_key`, `background_shutdown_timeout_ms` |
| `ServerConfig` | `host`, `port` |
| `DatabaseConfig` | `url`, `read_url`, `schema`, migration lock timeout, connection pool settings, SQL observability retention |
| `DatabaseModelConfig` | `timestamps_default`, `soft_deletes_default` |
| `RedisConfig` | `url`, `namespace` |
| `WebSocketConfig` | `host`, `port`, `path`, heartbeat, rate limits, origin allow-list, outbound buffer, history buffer/TTL |
| `JobsConfig` | `queue`, `max_retries`, `polling`, `concurrency`, `shutdown_timeout_ms`, `job_history` retention |
| `SchedulerConfig` | `tick_interval_ms`, `leader_lease_ttl_ms`, `shutdown_timeout_ms` |
| `AuthConfig` | `guards`, `tokens`, `sessions`, credential lifecycle, `bearer_prefix` |
| `TokenConfig` | TTLs, rotation, length, pruning, per-guard TTL overrides |
| `TokenGuardConfig` | optional per-guard token TTL overrides |
| `SessionConfig` | TTL, cookie settings, sliding expiry |
| `PasswordResetConfig` | expiry and worker pruning for password reset tokens |
| `EmailVerificationConfig` | expiry and worker pruning for email verification tokens |
| `GuardDriverConfig` | Individual guard driver config |
| `LoggingConfig` | `level`, `format`, `directory`, `retention` |
| `I18nConfig` | `locales`, `resource_path` |
| `ObservabilityConfig` | route/capture switches, sample retention, tracing, OTLP |
| `RuntimeConfig` | Tokio worker/blocking thread sizing for Foundry-owned sync runners |
| `HashingConfig` | `driver`, memory/time costs, parallelism |
| `CryptConfig` | `key` |
| `CacheConfig` | `driver`, `error_mode`, key bounds, TTL, memory size, `remember()` stampede controls |

### Enums

| Name | Variants |
|------|----------|
| `Environment` | `Development`, `Production`, `Testing` |
| `GuardDriver` | `Token`, `Session`, `Custom` |
| `CacheDriver` | `Redis`, `Memory` |

### ConfigRepository — methods

```rust
fn empty() -> Self
fn from_dir(path: impl AsRef<Path>) -> Result<Self>
fn with_env_overlay_only() -> Result<Self>
fn root(&self) -> Arc<Value>
fn value(&self, path: &str) -> Option<Value>
fn string(&self, path: &str) -> Option<String>
fn section<T: DeserializeOwned>(&self, section: &str) -> Result<T>

// Typed section accessors
fn app(&self) -> Result<AppConfig>
fn server(&self) -> Result<ServerConfig>
fn database(&self) -> Result<DatabaseConfig>
fn redis(&self) -> Result<RedisConfig>
fn websocket(&self) -> Result<WebSocketConfig>
fn jobs(&self) -> Result<JobsConfig>
fn runtime(&self) -> Result<RuntimeConfig>
fn auth(&self) -> Result<AuthConfig>
fn scheduler(&self) -> Result<SchedulerConfig>
fn logging(&self) -> Result<LoggingConfig>
fn i18n(&self) -> Result<I18nConfig>
fn observability(&self) -> Result<ObservabilityConfig>
fn storage(&self) -> Result<StorageConfig>
fn email(&self) -> Result<EmailConfig>
fn hashing(&self) -> Result<HashingConfig>
fn cache(&self) -> Result<CacheConfig>
fn crypt(&self) -> Result<CryptConfig>
```

### Environment — methods

```rust
fn from_label(label: impl Into<String>) -> Self
fn as_str(&self) -> &str
fn is_production(&self) -> bool
fn is_production_like(&self) -> bool
fn is_development(&self) -> bool
fn is_staging(&self) -> bool
fn is_testing(&self) -> bool
```

`Environment` accepts `development`, `production`, `staging`, `testing`, and custom labels. Use `is_production_like` for checks where staging should follow production behavior.

`RuntimeConfig.worker_threads` and `max_blocking_threads` default to `0`, which keeps Tokio defaults. Nonzero values apply only to Foundry-created sync runners such as `run_http`, `run_worker`, `run_scheduler`, `run_websocket`, and `run_cli`; async runners keep using the caller-owned runtime.

### Constants

```rust
const CONFIG_PUBLISH_COMMAND: CommandId;
const KEY_GENERATE_COMMAND: CommandId;
const MIGRATE_PUBLISH_COMMAND: CommandId;
const SEED_COMMAND: CommandId;
const ABOUT_COMMAND: CommandId;
```

### Functions

```rust
fn sample_config() -> String  // generates sample TOML
```

---

## support/

Typed IDs, crypto, datetime, collections, utilities.

### Typed Identifiers

All created via `TypeId::new("literal")` — zero-cost, const-constructible:

| Type | Purpose |
|------|---------|
| `ModelId<M>` | UUIDv7 per-model, type-parameterized |
| `GuardId` | Auth guard |
| `PolicyId` | Authorization policy |
| `PermissionId` | Permission |
| `RoleId` | Role |
| `ValidationRuleId` | Validation rule |
| `CommandId` | CLI command |
| `ScheduleId` | Scheduled task |
| `ChannelId` | WebSocket channel |
| `ChannelEventId` | WebSocket event |
| `JobId` | Background job |
| `QueueId` | Job queue |
| `EventId` | Domain event |
| `NotificationChannelId` | Notification channel |
| `PluginId` | Plugin |
| `PluginAssetId` | Plugin asset |
| `PluginScaffoldId` | Plugin scaffold |
| `MigrationId` | Migration |
| `SeederId` | Seeder |
| `ProbeId` | Health probe |

### DateTime / Clock

```rust
// DateTime (UTC)
DateTime::now() -> Self
DateTime::parse(value: &str) -> Result<Self>
DateTime::parse_in_timezone(value: &str, timezone: &Timezone) -> Result<Self>
fn format(&self) -> String
fn format_in(&self, timezone: &Timezone) -> String
fn date_in(&self, timezone: &Timezone) -> Date
fn local_datetime_in(&self, timezone: &Timezone) -> LocalDateTime
fn add_seconds(self, secs: i64) -> Self
fn sub_seconds(self, secs: i64) -> Self
fn add_days(self, days: i64) -> Self
fn sub_days(self, days: i64) -> Self
fn timestamp_millis(&self) -> i64
fn timestamp_micros(&self) -> i64

// LocalDateTime (naive)
LocalDateTime::parse(value: &str) -> Result<Self>
fn format(&self) -> String
fn in_timezone(&self, tz: &Timezone) -> Result<DateTime>
fn date(&self) -> Date
fn time(&self) -> Time
fn add_seconds / sub_seconds / add_days / sub_days

// Date
Date::parse(value: &str) -> Result<Self>
fn format(&self) -> String

// Time
Time::parse(value: &str) -> Result<Self>
fn format(&self) -> String

// Timezone
Timezone::utc() -> Self
Timezone::parse(value: &str) -> Result<Self>
fn as_str(&self) -> String

// Clock
Clock::new(timezone: Timezone) -> Self
fn now(&self) -> DateTime
fn today(&self) -> Date
fn timezone(&self) -> &Timezone
```

### Collection\<T\>

```rust
fn new() -> Self
fn from_vec(items: Vec<T>) -> Self
fn into_vec(self) -> Vec<T>
fn as_slice(&self) -> &[T]
fn len(&self) -> usize
fn is_empty(&self) -> bool
fn iter(&self) -> Iter<T>
fn first(&self) -> Option<&T>
fn last(&self) -> Option<&T>
fn get(&self, index: usize) -> Option<&T>

// Transforms
fn map<U>(self, f) -> Collection<U>
fn map_into<U>(self, f) -> Collection<U>
fn filter(self, f) -> Collection<T>
fn reject(self, f) -> Collection<T>
fn flat_map<U>(self, f) -> Collection<U>
fn find(&self, f) -> Option<&T>
fn first_where(self, f) -> Option<T>
fn any(&self, f) -> bool
fn all(&self, f) -> bool
fn count_where(&self, f) -> usize
fn pluck<U>(self, f) -> Collection<U>
fn key_by<K>(self, f) -> HashMap<K, T>
fn group_by<K>(self, f) -> HashMap<K, Collection<T>>
fn unique_by<K>(self, f) -> Collection<T>
fn partition_by(self, f) -> (Collection<T>, Collection<T>)
fn chunk(self, size: usize) -> Collection<Collection<T>>
fn sort_by(&mut self, f)
fn sort_by_key<K>(&mut self, f)
fn reverse(&mut self)
fn sum_by<U>(self, f) -> U
fn min_by<U>(self, f) -> Option<U>
fn max_by<U>(self, f) -> Option<U>
fn take(self, n: usize) -> Collection<T>
fn skip(self, n: usize) -> Collection<T>
fn for_each(self, f)
fn tap(self, f) -> Collection<T>
fn pipe(self, f) -> Collection<T>
```

### CryptManager

```rust
fn from_config(config: &CryptConfig) -> Result<Self>
fn encrypt(&self, plaintext: &[u8]) -> Result<String>
fn decrypt(&self, encoded: &str) -> Result<Vec<u8>>
fn encrypt_string(&self, plaintext: &str) -> Result<String>
fn decrypt_string(&self, encoded: &str) -> Result<String>
```

### HashManager

```rust
fn from_config(config: &HashingConfig) -> Result<Self>
fn hash(&self, password: &str) -> Result<String>
fn check(&self, password: &str, hash: &str) -> Result<bool>
fn random_string(length: usize) -> Result<String>  // static
```

`HashManager::hash()` and `HashManager::check()` stay synchronous for compatibility. In async handlers or model mutators, wrap password hashing/checking with `run_blocking` so Argon2 work does not occupy Tokio worker threads.

### Token

```rust
fn generate(length: usize) -> Result<String>  // static
fn bytes(length: usize) -> Result<Vec<u8>>     // static
fn hex(bytes: usize) -> Result<String>         // static
fn base64(bytes: usize) -> Result<String>      // static
```

### DistributedLock / LockGuard

```rust
// DistributedLock
async fn acquire(&self, key: &str, ttl: Duration) -> Result<Option<LockGuard>>
async fn block(&self, key: &str, ttl: Duration, timeout: Duration) -> Result<LockGuard>

// LockGuard
async fn extend(&self, ttl: Duration) -> Result<bool>
fn start_heartbeat(&self, ttl: Duration, interval: Duration) -> LockHeartbeat
async fn release(self) -> Result<bool>

struct LockHeartbeat
```

### Utility Functions

```rust
fn sanitize_html(input: &str, allowed_tags: &[&str]) -> String
fn strip_tags(input: &str) -> String
fn sha256_hex(data: &[u8]) -> String
fn sha256_hex_str(s: &str) -> String
fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool
fn boxed<F, T>(future: F) -> BoxFuture<T>
async fn run_blocking<T, F>(label: impl Into<String>, work: F) -> Result<T>
```

### Type Aliases

```rust
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
```

---

## database/

AST-first query system with typed models, relations, projections.

### database/ast

#### Enums

| Name | Variants |
|------|----------|
| `DbType` | `Int16`, `Int32`, `Int64`, `Bool`, `Float32`, `Float64`, `Numeric`, `Text`, `Json`, `Uuid`, `TimestampTz`, `Timestamp`, `Date`, `Time`, `Bytea`, + array variants for each |
| `DbValue` | Same variants as `DbType` — each wraps the actual value |
| `Expr` | `Column`, `Excluded`, `Value`, `Aggregate`, `Function`, `Unary`, `Binary`, `Subquery`, `Window`, `Case`, `JsonPath`, `Raw` |
| `Condition` | `Comparison`, `InList`, `JsonPredicate`, `And`, `Or`, `Not`, `IsNull`, `IsNotNull`, `Exists`, `Raw` |
| `ComparisonOp` | `Eq`, `NotEq`, `Gt`, `Gte`, `Lt`, `Lte`, `Like`, `NotLike` |
| `AggregateFn` | `Count`, `Sum`, `Avg`, `Min`, `Max` |
| `OrderDirection` | `Asc`, `Desc` |
| `JoinKind` | `Inner`, `Left`, `Right`, `Full`, `Cross` |
| `BinaryOperator` | `Add`, `Subtract`, `Multiply`, `Divide`, `Concat`, `Custom` |
| `UnaryOperator` | `Not`, `Negate` |
| `FromItem` | `Table`, `Subquery` |
| `JsonPathSegment` | `Key`, `Index` |
| `JsonPathMode` | `Json`, `Text` |
| `JsonPredicateOp` | `Contains`, `ContainedBy`, `HasKey`, `HasAnyKeys`, `HasAllKeys` |
| `JsonPredicateValue` | `Json`, `Key`, `Keys` |
| `WindowFrameUnits` | `Rows`, `Range` |
| `WindowFrameBound` | `UnboundedPreceding`, `Preceding`, `CurrentRow`, `Following`, `UnboundedFollowing` |
| `LockStrength` | `Update`, `NoKeyUpdate`, `Share`, `KeyShare` |
| `LockBehavior` | `Wait`, `NoWait`, `SkipLocked` |
| `RelationKind` | `HasMany`, `HasOne`, `BelongsTo`, `ManyToMany` |
| `OnConflictTarget` | `Columns`, `Constraint` |
| `OnConflictAction` | `DoNothing`, `DoUpdate` |
| `InsertSource` | `Values`, `Select` |
| `CteMaterialization` | `Materialized`, `NotMaterialized` |
| `SetOperator` | `Union`, `UnionAll` |
| `QueryBody` | `Select`, `Insert`, `Update`, `Delete`, `SetOperation` |

#### Structs

| Name | Summary |
|------|---------|
| `Numeric` | Newtype for numeric values |
| `TableRef` | Table reference with optional alias |
| `ColumnRef` | Column reference with optional table, alias, db_type |
| `AggregateExpr` | Aggregate function expression |
| `AggregateNode` | Named aggregate expression |
| `CaseWhen` | CASE condition-result pair |
| `CaseExpr` | Full CASE expression |
| `JsonPathExpr` | JSON path navigation |
| `FunctionCall` | SQL function call |
| `UnaryExpr` | Unary operator expression |
| `BinaryExpr` | Binary operator expression |
| `WindowFrame` | Window frame spec |
| `WindowSpec` | Window function spec |
| `WindowExpr` | Window function |
| `OrderBy` | Expression + direction |
| `SelectItem` | Select list item |
| `JoinNode` | Join specification |
| `LockClause` | Row lock spec |
| `PivotNode` | Pivot table reference |
| `RelationNode` | Relation metadata |
| `SelectNode` | Full SELECT |
| `OnConflictUpdate` | UPSERT update clause |
| `OnConflictNode` | ON CONFLICT clause |
| `InsertNode` | INSERT statement |
| `UpdateNode` | UPDATE statement |
| `DeleteNode` | DELETE statement |
| `CteNode` | Common Table Expression |
| `SetOperationNode` | UNION/UNION ALL |
| `QueryAst` | Complete query AST |

---

### database/model

#### Traits

```rust
trait ToDbValue {
    fn to_db_value(self) -> DbValue;
}

trait FromDbValue: Sized {
    fn from_db_value(value: &DbValue) -> Result<Self>;
}

trait IntoColumnValue<T> {
    fn into_column_value(self) -> T;
}

trait IntoFieldValue<T> {
    fn into_field_value(self, db_type: DbType) -> DbValue;
}

trait ModelLifecycle<M>: Send + Sync + 'static {
    async fn creating(context: &ModelHookContext<'_>, draft: &CreateDraft<M>) -> Result<()>;
    async fn created(context: &ModelHookContext<'_>, model: &M) -> Result<()>;
    async fn updating(context: &ModelHookContext<'_>, draft: &UpdateDraft<M>) -> Result<()>;
    async fn updated(context: &ModelHookContext<'_>, model: &M) -> Result<()>;
    async fn deleting(context: &ModelHookContext<'_>, model: &M) -> Result<()>;
    async fn deleted(context: &ModelHookContext<'_>, model: &M) -> Result<()>;
}

trait ModelWriteExecutor {
    fn app_context(&self) -> &AppContext;
    fn active_transaction(&self) -> Option<&DatabaseTransaction>;
    fn actor(&self) -> Option<&Actor>;
}

trait Model: Sized + Send + Sync + 'static {
    type Lifecycle: ModelLifecycle<Self>;
    fn table_meta() -> &'static TableMeta<Self>;
    fn model_query() -> ModelQuery<Self>;
    fn model_create() -> CreateModel<Self>;
    fn model_create_many() -> CreateManyModel<Self>;
    fn model_update() -> UpdateModel<Self>;
    fn model_delete() -> DeleteModel<Self>;
    fn model_force_delete() -> DeleteModel<Self>;
    fn model_restore() -> RestoreModel<Self>;
}

trait PersistedModel {
    fn persisted_condition(&self) -> Condition;
}

trait ModelInstanceWriteExt: PersistedModel + Model {
    fn update(&self) -> UpdateModel<Self>;
    fn delete(&self) -> DeleteModel<Self>;
    fn force_delete(&self) -> DeleteModel<Self>;
    fn restore(&self) -> RestoreModel<Self>;
}
```

#### Structs

| Name | Summary |
|------|---------|
| `ColumnInfo` | Field name, db_type, optional write_mutator |
| `Column<M, T>` | Typed column reference |
| `TableMeta<M>` | Table metadata: name, columns, primary key, behavior, hydrate fn |
| `ModelHookContext<'a>` | Context passed to lifecycle hooks |
| `CreateDraft<M>` | Accumulated values for model creation |
| `UpdateDraft<M>` | Accumulated value changes for update |
| `NoModelLifecycle` | No-op lifecycle implementation |

#### Traits

```rust
trait AfterCommitSink: Send + Sync {
    fn supports_after_commit(&self) -> bool; // default false
    fn defer_after_commit(&self, callback: AfterCommitCallback); // default drops callback
}
```

#### Enums

| Name | Variants |
|------|----------|
| `ModelFeatureSetting` | `Default`, `Enabled`, `Disabled` |
| `ModelPrimaryKeyStrategy` | `UuidV7`, `Manual` |
| `Loaded<T>` | `Unloaded`, `Loaded(T)` |

#### Type Aliases

```rust
type ModelFieldWriteMutatorFuture<'a> = Pin<Box<dyn Future<Output = Result<DbValue>> + Send + 'a>>;
type ModelFieldWriteMutator = for<'a> fn(&'a ModelHookContext<'a>, DbValue) -> ModelFieldWriteMutatorFuture<'a>;
type AfterCommitCallback =
    Box<dyn FnOnce(AppContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;
```

#### Model Events (auto-dispatched)

```rust
struct ModelCreatingEvent { /* ... */ }
struct ModelCreatedEvent  { /* ... */ }
struct ModelUpdatingEvent { /* ... */ }
struct ModelUpdatedEvent  { /* ... */ }
struct ModelDeletingEvent { /* ... */ }
struct ModelDeletedEvent  { /* ... */ }
```

`ModelCreatingEvent`, `ModelUpdatingEvent`, and `ModelDeletingEvent` are dispatched inside the
active model write transaction before the mutation is committed. Listener failures from these
pre-commit events abort the write and roll back framework-owned transactions.

`ModelCreatedEvent`, `ModelUpdatedEvent`, and `ModelDeletedEvent` are deferred until the active
transaction commits successfully. That makes post-write listeners safe for dependent writes that
need to see the committed row, including FK-backed records and queued onboarding work. If a
post-commit listener fails, Foundry logs the failure and leaves the already committed write intact.

---

### database/query

#### Structs

| Name | Summary |
|------|---------|
| `Query` | Raw query builder with fluent API |
| `ModelQuery<M>` | Typed model query builder |
| `CreateModel<M>` | Single model insertion |
| `CreateManyModel<M>` | Batch model insertion |
| `CreateRow<M>` | Raw row insertion |
| `UpdateModel<M>` | Model update |
| `DeleteModel<M>` | Model deletion |
| `ProjectionQuery<P>` | Projection query builder |
| `Pagination` | `page`, `per_page` |
| `Paginated<T>` | Collection + pagination metadata |
| `PaginatedResponse<T>` | JSON response with data, meta, links |
| `PaginationMeta` | `current_page`, `per_page`, `total`, `last_page` |
| `PaginationLinks` | `next`, `prev` URLs |
| `CursorPagination` | Cursor-based pagination config |
| `CursorPaginated<T>` | Cursor-paginated collection |
| `CursorMeta` | Cursor pagination metadata |
| `CursorInfo` | Cursor + direction |
| `CaseBuilder` | CASE expression builder |
| `WindowBuilder` | Window spec builder |
| `JsonExprBuilder` | JSON path builder |
| `Cte` | CTE builder |

#### Type Aliases

```rust
type RestoreModel<M> = UpdateModel<M>;
```

#### ModelQuery — retrieval helpers

```rust
async fn all<E>(&self, executor: &E) -> Result<Collection<M>>
async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
async fn first<E>(&self, executor: &E) -> Result<Option<M>>
async fn first_or_fail<E>(&self, executor: &E) -> Result<M>
async fn find<E, K>(&self, executor: &E, key: K) -> Result<Option<M>>
async fn find_or_fail<E, K>(&self, executor: &E, key: K) -> Result<M>
async fn find_many<E, I, K>(&self, executor: &E, keys: I) -> Result<Collection<M>>
async fn exists<E>(&self, executor: &E) -> Result<bool>
async fn doesnt_exist<E>(&self, executor: &E) -> Result<bool>
async fn value<E, T>(&self, executor: &E, column: Column<M, T>) -> Result<Option<T>>
async fn chunk<E, F, Fut>(&self, executor: &E, size: u64, handler: F) -> Result<()>
async fn chunk_by_id<E, T, F, Fut>(&self, executor: &E, column: Column<M, T>, size: u64, handler: F) -> Result<()>
async fn each_by_id<E, T, F, Fut>(&self, executor: &E, column: Column<M, T>, size: u64, handler: F) -> Result<()>
```

#### ModelQuery — relation and extension loading

```rust
fn with<To>(self, relation: RelationDef<M, To>) -> Self
fn with_many_to_many<To, Pivot>(self, relation: ManyToManyDef<M, To, Pivot>) -> Self
fn with_aggregate<Value>(self, aggregate: RelationAggregateDef<M, Value>) -> Self
fn with_attachments(self, collection: impl Into<String>) -> Self
fn with_translated_field(self, field: impl Into<String>) -> Self
fn with_translations_for(self, locale: impl Into<String>) -> Self
fn with_all_translations(self) -> Self
```

---

### database/relation

#### Traits

```rust
trait RelationLoader<From>: Send + Sync {
    fn node() -> RelationNode;
    async fn load(models: &mut [From], executor: &dyn QueryExecutor) -> Result<()>;
    async fn load_missing(models: &mut [From], executor: &dyn QueryExecutor) -> Result<()>;
}
```

#### Structs

| Name | Summary |
|------|---------|
| `RelationDef<From, To>` | One-to-many or one-to-one relation definition |
| `ManyToManyDef<From, To, Pivot>` | Many-to-many with pivot table |
| `RelationAggregateDef<From, Value>` | Aggregation over related records |

#### Functions

```rust
fn has_many<From, To, Key>() -> RelationDef<From, To>
fn has_one<From, To, Key>() -> RelationDef<From, To>
fn belongs_to<From, To, Key>() -> RelationDef<From, To>
fn many_to_many<From, To, Pivot, LocalKey, TargetKey>() -> ManyToManyDef<From, To, Pivot>
```

#### Nested eager loading

```rust
// RelationDef<From, To>
fn with<Child>(self, child: RelationDef<To, Child>) -> Self
fn with_many_to_many<Child, Pivot>(self, child: ManyToManyDef<To, Child, Pivot>) -> Self
fn with_attachments(self, collection: impl Into<String>) -> Self
fn with_translated_field(self, field: impl Into<String>) -> Self
fn with_translations_for(self, locale: impl Into<String>) -> Self
fn with_all_translations(self) -> Self

// ManyToManyDef<From, To, Pivot>
fn with<Child>(self, child: RelationDef<To, Child>) -> Self
fn with_many_to_many<Child, ChildPivot>(self, child: ManyToManyDef<To, Child, ChildPivot>) -> Self
fn with_attachments(self, collection: impl Into<String>) -> Self
fn with_translated_field(self, field: impl Into<String>) -> Self
fn with_translations_for(self, locale: impl Into<String>) -> Self
fn with_all_translations(self) -> Self
```

#### Type Aliases

```rust
type AnyRelation<M> = Arc<dyn RelationLoader<M>>;
```

---

### database/projection

#### Traits

```rust
trait Projection: Sized + Send + Sync + 'static {
    fn projection_meta() -> &'static ProjectionMeta<Self>;
    fn from_record(record: &DbRecord) -> Result<Self>;
    fn source() -> FromItem;
}
```

#### Structs

| Name | Summary |
|------|---------|
| `ProjectionFieldInfo` | Field alias, source column, db_type |
| `ProjectionField<P, T>` | Typed projection field |
| `ProjectionMeta<P>` | Projection metadata + hydrate fn |

---

### database/aggregate

```rust
struct AggregateProjection<T>; // aggregate result with type info
```

---

### database/collection_ext

#### Traits

```rust
trait IntoLoadableRelation<M> {
    fn into_relation(self) -> AnyRelation<M>;
}

trait ModelCollectionExt<T> {
    fn model_keys(&self) -> Vec<String>;
    async fn load<R>(&mut self, relation: R, executor: &dyn QueryExecutor) -> Result<()>;
    async fn load_missing<R>(&mut self, relation: R, executor: &dyn QueryExecutor) -> Result<()>;
}
```

---

### database/extensions

Task-local model extension cache used by eager and lazy batch loading for attachments and
translations.

#### Functions

```rust
async fn scope_model_extensions<F, T>(future: F) -> T
where
    F: Future<Output = T>;
```

HTTP requests are scoped automatically. CLI jobs, workers, and tests can use
`AppContext::with_model_batching(...)` or `scope_model_extensions(...)` to enable explicit
extension eager loading and lazy batch safety outside HTTP.

---

### database/runtime

#### Structs

| Name | Summary |
|------|---------|
| `DatabaseManager` | Connection pool manager |
| `DatabaseTransaction` | Active transaction |
| `DbRecord` | Key-value row from database |
| `SlowQueryEntry` | `sql`, `duration_ms`, `label`, `recorded_at` |
| `QueryExecutionOptions` | `timeout`, `label`, `use_write_pool` |

#### Traits

```rust
trait QueryExecutor: Send + Sync {
    async fn raw_query_with(&self, sql: &str, binds: &[DbValue], options: QueryExecutionOptions) -> Result<Vec<DbRecord>>;
    async fn raw_execute_with(&self, sql: &str, binds: &[DbValue], options: QueryExecutionOptions) -> Result<u64>;
    fn stream_records<'a>(&'a self, sql: &'a str, binds: &'a [DbValue]) -> DbRecordStream<'a>;
    async fn raw_query(&self, sql: &str, binds: &[DbValue]) -> Result<Vec<DbRecord>>;
    async fn raw_execute(&self, sql: &str, binds: &[DbValue]) -> Result<u64>;
    async fn query_records_with(&self, ast: &QueryAst, options: QueryExecutionOptions) -> Result<Vec<DbRecord>>;
    async fn query_records(&self, ast: &QueryAst) -> Result<Vec<DbRecord>>;
    async fn execute_compiled_with(&self, compiled: &CompiledSql, options: QueryExecutionOptions) -> Result<u64>;
    async fn execute_compiled(&self, compiled: &CompiledSql) -> Result<u64>;
}
```

#### Type Aliases

```rust
type DbRecordStream<'a> = BoxStream<'a, Result<DbRecord>>;
```

#### Functions

```rust
fn recent_slow_queries() -> Vec<SlowQueryEntry>
```

---

### database/compiler

```rust
struct CompiledSql { sql: String, bindings: Vec<DbValue> }
struct PostgresCompiler;

impl PostgresCompiler {
    fn compile(ast: &QueryAst) -> Result<CompiledSql>
}
```

---

### database/lifecycle

#### Traits

```rust
trait MigrationFile: Send + Sync {
    async fn up(&self, context: &MigrationContext) -> Result<()>;
    async fn down(&self, context: &MigrationContext) -> Result<()>;
}

trait SeederFile: Send + Sync {
    async fn seed(&self, context: &SeederContext) -> Result<()>;
}
```

#### Structs

```rust
struct MigrationContext<'a>; // database context for migrations
struct SeederContext<'a>;    // database context for seeders
```

---

## auth/

Bearer + session auth, policies, guards, token management.

### Enums

| Name | Variants |
|------|----------|
| `AccessScope` | `Public`, `Guarded(GuardedAccess)` |
| `AuthError` | `Unauthorized(String)`, `Forbidden(String)`, `Internal(String)` |

### Structs

| Name | Summary |
|------|---------|
| `GuardedAccess` | Access control with guard + permissions |
| `Actor` | Authenticated user: id, guard, roles, permissions, claims |
| `AuthManager` | Authenticates requests via bearer or session |
| `Authorizer` | Enforces permissions and policies |
| `StaticBearerAuthenticator` | In-memory token lookup |
| `CurrentActor(Actor)` | Axum extractor — requires auth |
| `OptionalActor(Option<Actor>)` | Axum extractor — optional auth |
| `AuthenticatedModel<M>` | Axum extractor — resolves model from actor |
| `AuthenticatableRegistry` | Type-erased model resolver |

### Traits

```rust
trait BearerAuthenticator: Send + Sync + 'static {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>>;
}

trait Policy: Send + Sync + 'static {
    async fn evaluate(&self, actor: &Actor, app: &AppContext) -> Result<bool>;
}

trait Authenticatable: Model + Send + Sync + 'static {
    fn guard() -> GuardId;
    async fn resolve_from_actor<E: QueryExecutor>(actor: &Actor, executor: &E) -> Result<Option<Self>>;
}
```

### Type Aliases

```rust
type Auth<M> = AuthenticatedModel<M>;
```

### Actor — methods

```rust
fn new<I, G>(id: I, guard: G) -> Self
fn with_guard<I>(self, guard: I) -> Self
fn with_roles<I, R>(self, roles: I) -> Self
fn with_permissions<I, P>(self, permissions: I) -> Self
fn with_claims(self, claims: Value) -> Self
fn has_role<I>(&self, role: I) -> bool
fn has_permission<I>(&self, permission: I) -> bool
async fn resolve<M: Authenticatable>(&self, app: &AppContext) -> Result<Option<M>>
```

### AuthManager — methods

```rust
fn default_guard(&self) -> &GuardId
async fn authenticate_headers(&self, headers: &HeaderMap, guard: Option<&GuardId>) -> Result<Actor, AuthError>
async fn authenticate_token(&self, token: &str, guard: Option<&GuardId>) -> Result<Actor, AuthError>
fn extract_token(&self, headers: &HeaderMap) -> Result<String, AuthError>
```

### Authorizer — methods

```rust
fn allows_permission(&self, actor: &Actor, permission: &PermissionId) -> bool
fn allows_permissions(&self, actor: &Actor, permissions: &BTreeSet<PermissionId>) -> bool
async fn authorize_permissions(&self, actor: &Actor, permissions: &BTreeSet<PermissionId>) -> Result<(), AuthError>
async fn allows_policy<I>(&self, actor: &Actor, policy: I) -> Result<bool>
```

---

### auth/token

```rust
struct TokenPair { access_token, refresh_token, expires_in, token_type }
struct TokenManager;
struct TokenAuthenticator;

trait HasToken: Authenticatable {
    async fn create_token(&self, app: &AppContext) -> Result<TokenPair>;
    async fn create_token_named(&self, app: &AppContext, name: &str) -> Result<TokenPair>;
    async fn create_token_with_abilities(&self, app: &AppContext, name: &str, abilities: Vec<String>) -> Result<TokenPair>;
    async fn revoke_all_tokens(&self, app: &AppContext) -> Result<u64>;
    fn token_actor_id(&self) -> String;
}
```

**TokenManager — methods:**

```rust
async fn issue<M: Authenticatable>(&self, actor_id: &str) -> Result<TokenPair>
async fn issue_named<M: Authenticatable>(&self, actor_id: &str, name: &str) -> Result<TokenPair>
async fn issue_with_abilities<M: Authenticatable>(&self, actor_id: &str, name: &str, abilities: Vec<String>) -> Result<TokenPair>
async fn validate(&self, access_token: &str) -> Result<Option<Actor>>
async fn touch(&self, access_token: &str) -> Result<()>
async fn refresh(&self, refresh_token: &str) -> Result<TokenPair>
async fn revoke(&self, access_token: &str) -> Result<()>
async fn revoke_all<M: Authenticatable>(&self, actor_id: &str) -> Result<u64>
async fn prune(&self, older_than_days: u64) -> Result<u64>
```

---

### auth/session

```rust
struct SessionManager;
```

**Methods:**

```rust
fn config(&self) -> &SessionConfig
async fn create<M: Authenticatable>(&self, actor_id: &str) -> Result<String>
async fn create_with_remember<M: Authenticatable>(&self, actor_id: &str, remember: bool) -> Result<String>
async fn validate(&self, session_id: &str) -> Result<Option<Actor>>
async fn destroy(&self, session_id: &str) -> Result<()>
async fn destroy_all<M: Authenticatable>(&self, actor_id: &str) -> Result<()>
fn login_response(&self, session_id: String, body: impl IntoResponse) -> Result<Response>
fn login_response_with_remember(&self, session_id: String, remember: bool, body: impl IntoResponse) -> Result<Response>
fn logout_response(&self, body: impl IntoResponse) -> Result<Response>
```

Session cookies validate configured name/path/domain/SameSite values. `SameSite=None`
requires secure cookies. `login_response_with_remember` adds a persistent `Max-Age`
only when `remember` is true; `login_response` remains a browser-session cookie.

---

### auth/password_reset

```rust
struct PasswordResetManager;

async fn create_token<M: Authenticatable>(&self, email: &str) -> Result<String>
async fn validate_token<M: Authenticatable>(&self, email: &str, token: &str) -> Result<()>
async fn prune_expired(&self) -> Result<u64>
```

---

### auth/email_verification

```rust
struct EmailVerificationManager;

async fn create_token<M: Authenticatable>(&self, email: &str) -> Result<String>
async fn validate_token<M: Authenticatable>(&self, email: &str, token: &str) -> Result<()>
async fn prune_expired(&self) -> Result<u64>
```

---

## http/

Routes, middleware, cookies, resources, SPA.

### Structs

| Name | Summary |
|------|---------|
| `HttpRegistrar` | Route registration builder |
| `HttpRouteOptions` | Per-route config: access, middleware, rate limit, docs |
| `RouteManifestEntry` | Named route metadata exported to TypeScript |
| `RouteManifestResponse` | Route response schema metadata exported to TypeScript |

### Type Aliases

```rust
type RouteRegistrar = Arc<dyn Fn(&mut HttpRegistrar) -> Result<()> + Send + Sync>;
type HttpRouter = Router<AppContext>;
```

### HttpRegistrar — methods

```rust
fn new() -> Self
fn route(&mut self, path: &str, method_router: MethodRouter<AppContext>) -> &mut Self
fn route_with_options(&mut self, path: &str, method_router: MethodRouter<AppContext>, options: HttpRouteOptions) -> &mut Self
fn route_named<I: Into<RouteId>>(&mut self, name: I, path: &str, method_router: MethodRouter<AppContext>) -> &mut Self
fn route_named_with_options<I: Into<RouteId>>(&mut self, name: I, path: &str, method_router: MethodRouter<AppContext>, options: HttpRouteOptions) -> &mut Self
fn scope(&mut self, path: &str, f: impl FnOnce(&mut HttpScope<'_>) -> Result<()>) -> Result<&mut Self>
fn nest(&mut self, path: &str, router: HttpRouter) -> &mut Self
fn merge(&mut self, router: HttpRouter) -> &mut Self
fn group(&mut self, prefix: &str, f: impl FnOnce(&mut HttpRegistrar) -> Result<()>) -> Result<&mut Self>
fn group_with_options(&mut self, prefix: &str, options: HttpRouteOptions, f: impl FnOnce(&mut HttpRegistrar) -> Result<()>) -> Result<&mut Self>
fn resource(&mut self, name: &str, path: &str, routes: HttpResourceRoutes) -> &mut Self
fn resource_with_options(&mut self, name: &str, path: &str, routes: HttpResourceRoutes, options: HttpRouteOptions) -> &mut Self
fn api_version(&mut self, version: u32, f: impl FnOnce(&mut HttpRegistrar) -> Result<()>) -> Result<&mut Self>
fn collect_route_manifest(&self) -> Result<Vec<RouteManifestEntry>>
fn into_router(self, app: AppContext) -> Router
fn into_router_with_middlewares(self, app: AppContext, middlewares: Vec<MiddlewareConfig>) -> Router
```

### HttpScope — methods

```rust
fn scope(&mut self, path: &str, f: impl FnOnce(&mut HttpScope<'_>) -> Result<()>) -> Result<&mut Self>
fn name_prefix(&mut self, prefix: &str) -> &mut Self
fn public(&mut self) -> &mut Self
fn guard<I>(&mut self, guard: I) -> &mut Self
fn permission<I>(&mut self, permission: I) -> &mut Self
fn permissions<I, P>(&mut self, permissions: I) -> &mut Self
fn middleware(&mut self, config: MiddlewareConfig) -> &mut Self
fn middleware_group(&mut self, name: impl Into<String>) -> &mut Self
fn rate_limit(&mut self, rate_limit: RateLimit) -> &mut Self
fn tag(&mut self, tag: &str) -> &mut Self
fn summary(&mut self, summary: &str) -> &mut Self
fn description(&mut self, description: &str) -> &mut Self
fn deprecated(&mut self) -> &mut Self
fn get<H, T>(&mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder)) -> &mut Self
fn post<H, T>(&mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder)) -> &mut Self
fn put<H, T>(&mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder)) -> &mut Self
fn patch<H, T>(&mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder)) -> &mut Self
fn delete<H, T>(&mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder)) -> &mut Self
```

### HttpRouteOptions — methods

```rust
fn new() -> Self
fn guard<I>(self, guard: I) -> Self
fn permission<I>(self, permission: I) -> Self
fn permissions<I, P>(self, permissions: I) -> Self
fn middleware(self, config: MiddlewareConfig) -> Self
fn middleware_group(self, name: impl Into<String>) -> Self
fn rate_limit(self, rate_limit: RateLimit) -> Self
fn document(self, doc: RouteDoc) -> Self
```

### HttpRouteBuilder — methods

```rust
fn public(&mut self) -> &mut Self
fn guard<I>(&mut self, guard: I) -> &mut Self
fn permission<I>(&mut self, permission: I) -> &mut Self
fn permissions<I, P>(&mut self, permissions: I) -> &mut Self
fn middleware(&mut self, config: MiddlewareConfig) -> &mut Self
fn middleware_group(&mut self, name: impl Into<String>) -> &mut Self
fn rate_limit(&mut self, rate_limit: RateLimit) -> &mut Self
fn tag(&mut self, tag: &str) -> &mut Self
fn summary(&mut self, summary: &str) -> &mut Self
fn description(&mut self, description: &str) -> &mut Self
fn request<T: ApiSchema>(&mut self) -> &mut Self
fn response<T: ApiSchema>(&mut self, status: u16) -> &mut Self
fn deprecated(&mut self) -> &mut Self
```

---

### http/middleware

#### Enums

| Name | Variants |
|------|----------|
| `MiddlewareConfig` | (enum of all middleware types) |
| `RateLimitWindow` | `PerSecond(u32)`, `PerMinute(u32)`, `PerHour(u32)` |
| `RateLimitBy` | `Ip`, `Actor`, `ActorOrIp` |

#### Structs

| Name | Summary |
|------|---------|
| `RealIp(IpAddr)` | Real IP extractor |
| `Cors` | CORS config builder |
| `SecurityHeaders` | Security headers builder |
| `Csrf` | CSRF protection |
| `CsrfToken(String)` | CSRF token wrapper |
| `RateLimit` | Rate limiting config |
| `MaxBodySize(usize)` | Body size limiter |
| `RequestTimeout(Duration)` | Request timeout |
| `Compression` | Brotli/Gzip compression |
| `MaintenanceMode` | Maintenance mode |
| `TrustedProxy` | Proxy trust config |
| `ETag` | HTTP ETag support |
| `MiddlewareGroups` | Named middleware group registry |

#### Download helpers

```rust
enum ContentDispositionType { Attachment, Inline }
fn attachment_content_disposition(filename: impl AsRef<str>) -> HeaderValue
fn inline_content_disposition(filename: impl AsRef<str>) -> HeaderValue
fn content_disposition_header(disposition: ContentDispositionType, filename: impl AsRef<str>) -> HeaderValue
fn content_disposition_value(disposition: ContentDispositionType, filename: &str) -> String
```

Download helpers sanitize path-like/control-character filenames, emit a safe ASCII
`filename`, and include RFC 5987 `filename*` for Unicode clients.

**Csrf — methods:**

```rust
fn new() -> Self
fn from_config(config: &HttpCsrfConfig) -> Result<Self>
fn cookie_name(self, name: &str) -> Self
fn header_name(self, name: HeaderName) -> Self
fn secure(self, secure: bool) -> Self
fn path(self, path: &str) -> Self
fn same_site(self, same_site: &str) -> Self
fn exclude(self, path: &str) -> Self
fn exclude_paths<'a, I>(self, paths: I) -> Self where I: IntoIterator<Item = &'a str>
fn build(self) -> MiddlewareConfig
```

CSRF exclusions are segment-aware: `/api` excludes `/api` and `/api/...`, not `/apiary`.

**Cors — methods:**

```rust
fn origin(self, origin: &str) -> Self
fn origins(self, origins: Vec<&str>) -> Self
fn allow_any_origin(self) -> Self
fn credential(self, allow: bool) -> Self
fn allowed_methods(self, methods: impl Into<String>) -> Self
fn allowed_headers(self, headers: impl Into<String>) -> Self
fn exposed_headers(self, headers: impl Into<String>) -> Self
fn max_age(self, secs: u64) -> Self
```

**SecurityHeaders — methods:**

```rust
fn hsts(self, max_age_secs: u32) -> Self
fn csp(self, policy: &str) -> Self
fn frame_options(self, value: &str) -> Self
fn x_content_type_options(self) -> Self
fn referrer_policy(self, policy: &str) -> Self
fn permissions_policy(self, policy: &str) -> Self
```

**RateLimit — methods:**

```rust
fn per_second(max: u32) -> Self
fn per_minute(max: u32) -> Self
fn per_hour(max: u32) -> Self
fn by_ip(self) -> Self
fn by_actor(self) -> Self
fn by_actor_or_ip(self) -> Self
```

---

### http/cookie

```rust
fn extract_cookie_value(headers: &HeaderMap, name: &str) -> Option<String>

struct SessionCookie;
fn build<'a>(name: &'a str, value: &'a str, secure: bool) -> Cookie<'a>
fn clear(name: &str) -> Cookie<'_>

// Re-exports
pub use axum_extra::extract::cookie::{Cookie, SameSite};
pub use axum_extra::extract::CookieJar;
```

---

### http/resource

```rust
trait ApiResource<T> {
    fn transform(item: &T) -> Value;
    fn make(item: &T) -> Value;
    fn collection(items: &[T]) -> Vec<Value>;
    fn paginated(paginated: &Paginated<T>, base_url: &str) -> Value;
}
```

---

### http/routes

```rust
struct RouteRegistry;

fn new() -> Self
fn register(&mut self, name: impl Into<RouteId>, pattern: impl Into<String>)
fn url<I>(&self, name: I, params: &[(&str, &str)]) -> Result<String>
fn has<I>(&self, name: I) -> bool
fn iter(&self) -> impl Iterator<Item = (&RouteId, &String)>
fn signed_url(&self, name: impl Into<RouteId>, params: &[(&str, &str)], signing_key: &[u8], expires_at: DateTime) -> Result<String>
fn verify_signature(url: &str, signing_key: &[u8]) -> Result<()>  // static
```

Signed URL verification rejects duplicate `expires` or `signature` parameters,
invalid signature shape, expired URLs, and query parameters appended after the
signature.

---

## websocket/

Channel-based typed WebSocket with presence.

### Constants

```rust
const SYSTEM_CHANNEL: ChannelId;
const ERROR_EVENT: ChannelEventId;
const SUBSCRIBED_EVENT: ChannelEventId;
const UNSUBSCRIBED_EVENT: ChannelEventId;
const PRESENCE_JOIN_EVENT: ChannelEventId;
const PRESENCE_LEAVE_EVENT: ChannelEventId;
const ACK_EVENT: ChannelEventId;
```

### Enums

| Name | Variants |
|------|----------|
| `ClientAction` | `Subscribe`, `Unsubscribe`, `Message`, `ClientEvent` |

Wire values serialize as `snake_case` (`subscribe`, `unsubscribe`, `message`, `client_event`). PascalCase values are accepted only as compatibility aliases.

### Structs

| Name | Summary |
|------|---------|
| `PresenceInfo` | `actor_id`, `channel`, `joined_at` |
| `ClientMessage` | `action`, `channel`, `room`, `payload`, `event`, `ack_id` |
| `ServerMessage` | `channel`, `event`, `room`, `payload` |
| `WebSocketContext` | Connection context: app, connection_id, actor, channel, room |
| `WebSocketChannelOptions` | Channel config: access, presence, handlers |
| `WebSocketPublisher` | Publishes messages and manages subscriptions |
| `WebSocketRegistrar` | Channel registration builder |

### Traits

```rust
trait ChannelHandler: Send + Sync + 'static {
    async fn handle(&self, context: WebSocketContext, payload: Value) -> Result<()>;
}
```

### WebSocketContext — methods

```rust
fn app(&self) -> &AppContext
fn connection_id(&self) -> u64
fn actor(&self) -> Option<&Actor>
async fn resolve_actor<M: Authenticatable>(&self) -> Result<Option<M>>
fn channel(&self) -> &ChannelId
fn room(&self) -> Option<&str>
async fn publish<I>(&self, event: I, payload: impl Serialize) -> Result<()>
async fn presence_members(&self) -> Result<Vec<PresenceInfo>>
async fn presence_count(&self) -> Result<usize>
```

### WebSocketPublisher — methods

```rust
async fn publish<C, E>(&self, channel: C, event: E, room: Option<&str>, payload: impl Serialize) -> Result<()>
async fn publish_message(&self, message: ServerMessage) -> Result<()>
async fn disconnect_user(&self, actor_id: &str) -> Result<()>
```

### WebSocketChannelOptions — builder

```rust
fn new() -> Self
fn presence(self, enabled: bool) -> Self
fn guard<I>(self, guard: I) -> Self
fn permission<I>(self, permission: I) -> Self
fn permissions<I, P>(self, permissions: I) -> Self
fn authorize<F, Fut>(self, f: F) -> Self
fn allow_client_events(self, enabled: bool) -> Self
fn on_join<F, Fut>(self, f: F) -> Self
fn on_leave<F, Fut>(self, f: F) -> Self
fn replay(self, count: u32) -> Self
```

### WebSocketRegistrar — methods

```rust
fn new() -> Self
fn channel<I, H>(&mut self, id: I, handler: H) -> Result<&mut Self>
fn channel_with_options<I, H>(&mut self, id: I, handler: H, options: WebSocketChannelOptions) -> Result<&mut Self>
```

### Type Aliases

```rust
type WebSocketRouteRegistrar = Arc<dyn Fn(&mut WebSocketRegistrar) -> Result<()> + Send + Sync>;
type LifecycleCallback = Arc<dyn Fn(WebSocketContext) -> BoxFuture<Result<()>> + Send + Sync>;
type AuthorizeCallback = Arc<dyn Fn(WebSocketContext, ChannelId, Option<String>) -> BoxFuture<Result<()>> + Send + Sync>;
```

Protocol guarantees:

- `message` and `client_event` frames require an active matching channel/room subscription.
- Channel-wide publishes reach every subscriber on that channel; room publishes reach only exact room subscribers.
- `on_leave` and `presence:leave` run for unsubscribe, socket close, heartbeat timeout, and force disconnect.

---

## validation/

30+ rules, custom rules, request validation extractor.

### Structs

| Name | Summary |
|------|---------|
| `RuleContext` | App context + field name |
| `RuleRegistry` | Registry of custom validation rules |
| `ValidationError` | `code`, `message` |
| `FieldError` | `field`, `code`, `message` |
| `ValidationErrors` | Collection of field errors |
| `FieldValidator<'a>` | Validates a single string field |
| `EachValidator<'a, T>` | Validates multiple string items |
| `Validator` | Main validation orchestrator |
| `Validated<T>` | Axum extractor — auto-validates request body |

### Traits

```rust
trait ValidationRule: Send + Sync + 'static {
    async fn validate(&self, context: &RuleContext, value: &str) -> Result<Option<String>>;
}

trait RequestValidator: Send + Sync {
    async fn validate(&self, validator: &mut Validator) -> Result<()>;
    fn messages(&self) -> HashMap<String, String> { HashMap::new() }  // default
    fn attributes(&self) -> HashMap<String, String> { HashMap::new() } // default
}

trait FromMultipart: Sized {
    async fn from_multipart(multipart: Multipart) -> Result<Self>;
}
```

### File Validation Functions

```rust
async fn is_image(file: &UploadedFile) -> Result<bool>
fn check_max_size(file: &UploadedFile, max_kb: u64) -> bool
async fn get_image_dimensions(file: &UploadedFile) -> Result<(u32, u32)>
async fn check_allowed_mimes(file: &UploadedFile, allowed: &[String]) -> Result<bool>
fn check_allowed_extensions(file: &UploadedFile, allowed: &[String]) -> bool
```

---

## email/

Multi-driver email with templates and queueing.

### Structs

| Name | Summary |
|------|---------|
| `EmailAddress` | Address + optional name |
| `EmailMessage` | Fluent email builder |
| `EmailManager` | Multi-mailer manager |
| `EmailMailer` | Single mailer instance |
| `TemplateRenderer` | Template file renderer |
| `RenderedTemplate` | `html`, `text` |
| `OutboundEmail` | Resolved email ready to send |
| `LogEmailDriver` | Dev driver — logs to stdout |
| `SmtpEmailDriver` | SMTP driver |
| `MailgunEmailDriver` | Mailgun API driver |
| `PostmarkEmailDriver` | Postmark API driver |
| `ResendEmailDriver` | Resend API driver |
| `SesEmailDriver` | AWS SES driver |

### Enums

| Name | Variants |
|------|----------|
| `EmailAttachment` | `Path { path, name, content_type }`, `Storage { disk, path, name, content_type }` |
| `SmtpEncryption` | `StartTls`, `Tls`, `None` |

Built-in HTTP mailers (`MailgunEmailDriver`, `PostmarkEmailDriver`,
`ResendEmailDriver`, and `SesEmailDriver`) use `timeout_secs = 30` by default.
`EmailConfig.max_attachment_bytes` and `max_total_attachment_bytes` bound
resolved attachment payloads before provider delivery; `0` disables each cap.
The built-in SES driver uses the SES SendEmail API and rejects attachments
clearly instead of silently dropping them.
Provider error bodies are truncated and obvious secret fields are redacted before
delivery errors are returned or logged.

### Traits

```rust
trait EmailDriver: Send + Sync + 'static {
    async fn send(&self, message: &OutboundEmail) -> Result<()>;
}
```

### Type Aliases

```rust
type EmailDriverFactory = Arc<dyn Fn(&ConfigRepository, &toml::Table) -> Result<Arc<dyn EmailDriver>> + Send + Sync>;
```

### EmailMessage — builder

```rust
fn new(subject: impl Into<String>) -> Self
fn from(self, addr: impl Into<EmailAddress>) -> Self
fn to(self, addr: impl Into<EmailAddress>) -> Self
fn cc(self, addr: impl Into<EmailAddress>) -> Self
fn bcc(self, addr: impl Into<EmailAddress>) -> Self
fn reply_to(self, addr: impl Into<EmailAddress>) -> Self
fn text_body(self, body: impl Into<String>) -> Self
fn html_body(self, body: impl Into<String>) -> Self
fn template(self, template_name: &str, template_path: &str, variables: Value) -> Result<Self>
fn header(self, key: impl Into<String>, value: impl Into<String>) -> Self
fn attach(self, attachment: EmailAttachment) -> Self
```

### EmailManager — methods

```rust
fn from_config(config, custom_drivers, app) -> Result<Self>
fn mailer(&self, name: &str) -> Result<EmailMailer>
fn default_mailer(&self) -> Result<EmailMailer>
fn default_mailer_name(&self) -> &str
fn from_address(&self) -> &EmailFromConfig
fn configured_mailers(&self) -> Vec<String>
fn descriptors(&self) -> Vec<EmailMailerDescriptor>
```

### EmailMailer — methods

```rust
fn send(&self, message: EmailMessage) -> Result<()>
fn queue(&self, message: EmailMessage) -> Result<()>
fn queue_later(&self, message: EmailMessage, run_at_millis: i64) -> Result<()>
```

---

## storage/

Local + S3 file storage with multipart uploads.

### Structs

| Name | Summary |
|------|---------|
| `StorageManager` | Multi-disk manager |
| `StorageDisk` | Single disk instance |
| `LocalStorageAdapter` | Local filesystem adapter |
| `S3StorageAdapter` | S3-compatible adapter |
| `StorageObject` | Listed object metadata: `path`, `size`, `modified_at` |
| `StoredFile` | `disk`, `path`, `name`, `size`, `content_type`, `url` |
| `UploadedFile` | `field_name`, `original_name`, `content_type`, `size`, `temp_path` |
| `MultipartForm` | Parsed multipart form |
| `UploadLimits` | Storage-level multipart upload caps |
| `UploadCounters` | Request-local upload byte/file counters |

### Enums

| Name | Variants |
|------|----------|
| `StorageVisibility` | `Private`, `Public` |

### Traits

```rust
trait StorageAdapter: Send + Sync + 'static {
    async fn put_bytes(&self, path: &str, bytes: &[u8]) -> Result<()>;
    async fn put_file(&self, path: &str, temp_path: &Path, content_type: Option<&str>) -> Result<()>;
    async fn get(&self, path: &str) -> Result<Vec<u8>>;
    async fn delete(&self, path: &str) -> Result<()>;
    async fn exists(&self, path: &str) -> Result<bool>;
    async fn copy(&self, from: &str, to: &str) -> Result<()>;
    async fn move_to(&self, from: &str, to: &str) -> Result<()>;
    async fn url(&self, path: &str) -> Result<String>;
    async fn temporary_url(&self, path: &str, expires_at: DateTime) -> Result<String>;
    async fn list_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<StorageObject>>;
}
```

### Type Aliases

```rust
type StorageDriverFactory = Arc<dyn Fn(&ConfigRepository, &toml::Table) -> BoxFuture<Result<Arc<dyn StorageAdapter>>> + Send + Sync>;
```

### StorageManager — methods

```rust
fn from_config(config, custom_drivers) -> Result<Self>
fn default_disk(&self) -> Result<StorageDisk>
fn disk(&self, name: &str) -> Result<StorageDisk>
fn default_disk_name(&self) -> &str
fn configured_disks(&self) -> Vec<String>
// Also delegates: put, put_bytes, put_file, get, delete, exists, copy, move_to, url, temporary_url, list_prefix
```

### UploadedFile — methods

```rust
fn from_multipart_field(field_name, field, counters) -> Result<Option<UploadedFile>>
fn generate_storage_name(&self) -> String
fn original_extension(&self) -> Option<String>
fn sanitize_name(name: &str) -> String
fn normalize_name(name: &str) -> String
fn store(&self, app: &AppContext, dir: &str) -> Result<StoredFile>
fn store_on(&self, app: &AppContext, disk_name: &str, dir: &str) -> Result<StoredFile>
fn store_as(&self, app: &AppContext, dir: &str, name: &str) -> Result<StoredFile>
fn store_as_on(&self, app: &AppContext, disk_name: &str, dir: &str, name: &str) -> Result<StoredFile>
```

### Upload helpers

```rust
async fn cleanup_uploaded_files(files: impl IntoIterator<Item = &UploadedFile>)
async fn remove_uploaded_temp_file(file: &UploadedFile) -> bool
async fn prune_stale_upload_temp_files(retention_seconds: u64, batch_size: u64) -> Result<u64>
```

### MultipartForm — methods

```rust
fn file(&self, name: &str) -> Result<&UploadedFile>
fn files(&self, name: &str) -> &[UploadedFile]
fn text(&self, name: &str) -> Option<&str>
```

Multipart extraction honors `[storage]` upload caps and returns Foundry JSON `413` errors for oversized uploads or too many uploaded files. Foundry sanitizes uploaded filenames before metadata/storage-name use and removes Foundry-owned temp files on extraction failure. Foundry worker maintenance prunes stale successful `foundry-upload-*` temp files according to storage retention settings. Storage paths are logical relative keys; Foundry rejects absolute paths, relative segments, empty segments, backslashes, drive prefixes, and control characters before disk access.

Attachment image processing also honors `[storage]` decode safety limits for input bytes, width, height, and total pixels. Foundry worker maintenance audits old objects under `storage.attachment_orphan_prefix`; deletion is off by default and requires `storage.attachment_orphan_delete_enabled = true`.

---

## jobs/

Background job queue with leased at-least-once delivery.

### Traits

```rust
trait Job: Serialize + DeserializeOwned + Send + Sync + Debug {
    const ID: JobId;
    const QUEUE: Option<QueueId> = None;
    async fn handle(&self, context: JobContext) -> Result<()>;
    fn max_retries(&self) -> Option<u32> { None }
    fn backoff(&self, attempt: u32) -> Duration { /* exponential */ }
    fn timeout(&self) -> Option<Duration> { None }
    fn rate_limit(&self) -> Option<(u32, Duration)> { None }
    fn unique_for(&self) -> Option<Duration> { None }
    fn unique_key(&self) -> Option<String> { None }
}

trait JobMiddleware: Send + Sync + 'static {
    async fn before(&self, ...) -> Result<()>;
    async fn after(&self, ...) -> Result<()>;
    async fn failed(&self, ...) -> Result<()>;
    async fn on_dead_lettered(&self, ...) -> Result<()>;
}
```

### Structs

| Name | Summary |
|------|---------|
| `JobContext` | `app`, `queue`, `attempt` |
| `JobDeadLetterContext` | Dead-letter payload, attempts, error, and app context |
| `JobDispatcher` | Dispatch jobs to queue |
| `JobBatchBuilder` | Build job batches |
| `JobChainBuilder` | Build job chains |
| `Worker` | Job processor |

### JobContext — methods

```rust
fn app(&self) -> &AppContext
fn queue(&self) -> &QueueId
fn attempt(&self) -> u32
```

### JobDispatcher — methods

```rust
fn dispatch<J: Job>(&self, job: J) -> Result<()>
fn dispatch_later<J: Job>(&self, job: J, run_at_millis: i64) -> Result<()>
fn batch(&self, name: &str) -> JobBatchBuilder
fn chain(&self) -> JobChainBuilder
```

### JobBatchBuilder — methods

```rust
fn add<J: Job>(self, job: J) -> Result<Self>
fn on_complete<J: Job>(self, job: J) -> Result<Self>
fn dispatch(self) -> Result<String>
```

### JobChainBuilder — methods

```rust
fn add<J: Job>(self, job: J) -> Result<Self>
fn dispatch(self) -> Result<()>
```

### Functions

```rust
fn spawn_worker(app: AppContext) -> Result<JoinHandle<()>>
```

Workers stop claiming new jobs on shutdown and drain active jobs for `jobs.shutdown_timeout_ms`.
If the timeout elapses, or the value is `0`, active jobs are aborted without ack, retry, or
dead-letter finalization. Their lease expires and the existing requeue flow makes them runnable
again on another worker or restart.

Workers spawned with `spawn_worker(app)` are managed by the app lifecycle and remain capped by
`app.background_shutdown_timeout_ms`. Job handler panics are handled as normal job failures and use
the existing retry/dead-letter flow.

---

## scheduler/

Cron + interval scheduling with Redis-safe leadership.

### Enums

| Name | Variants |
|------|----------|
| `ScheduleKind` | `Cron { expression: Box<CronExpression> }`, `Interval { every: Duration }` |

### Structs

| Name | Summary |
|------|---------|
| `CronExpression` | Parsed cron expression |
| `ScheduleInvocation` | Context passed to schedule handlers |
| `ScheduleOptions` | Per-task options |
| `ScheduledTask` | Registered task |
| `ScheduleRegistry` | Task registry |

### Type Aliases

```rust
type ScheduleRegistrar = Arc<dyn Fn(&mut ScheduleRegistry) -> Result<()> + Send + Sync>;
```

### CronExpression — constructors

```rust
fn parse(value: impl Into<String>) -> Result<Self>
fn every_minute() -> Result<Self>
fn every_five_minutes() -> Result<Self>
fn every_ten_minutes() -> Result<Self>
fn every_fifteen_minutes() -> Result<Self>
fn every_thirty_minutes() -> Result<Self>
fn hourly() -> Result<Self>
fn daily() -> Result<Self>
fn daily_at(time: &str) -> Result<Self>
fn weekly() -> Result<Self>
fn monthly() -> Result<Self>
fn as_str(&self) -> &str
```

### ScheduleOptions — builder

```rust
fn new() -> Self
fn without_overlapping(self) -> Self
fn environments(self, envs: &[&str]) -> Self
fn before<F, Fut>(self, hook: F) -> Self
fn after<F, Fut>(self, hook: F) -> Self
fn on_failure<F, Fut>(self, hook: F) -> Self
```

### ScheduleRegistry — methods

```rust
fn new() -> Self
fn cron<I, F, Fut>(&mut self, id: I, expression: CronExpression, job: F) -> Result<&mut Self>
fn cron_with_options<I, F, Fut>(&mut self, id: I, expr: CronExpression, options: ScheduleOptions, job: F) -> Result<&mut Self>
fn interval<I, F, Fut>(&mut self, id: I, every: Duration, job: F) -> Result<&mut Self>
fn interval_with_options<I, F, Fut>(&mut self, id: I, every: Duration, options: ScheduleOptions, job: F) -> Result<&mut Self>
fn every_minute<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
fn every_five_minutes<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
fn hourly<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
fn daily<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
fn daily_at<I, F, Fut>(&mut self, id: I, time: &str, job: F) -> Result<&mut Self>
fn weekly<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
```

Schedule handler panics are handled as schedule failures and route through `ScheduleOptions::on_failure`.
Hook panics are logged and isolated, `without_overlapping` locks are released on failure or panic,
and active schedules drain for `scheduler.shutdown_timeout_ms` during shutdown.

---

## events/

Domain event bus with listeners.

### Traits

```rust
trait Event: Clone + Serialize + Send + Sync + 'static {
    const ID: EventId;
}

trait EventListener<E: Event>: Send + Sync + 'static {
    async fn handle(&self, context: &EventContext, event: &E) -> Result<()>;
}
```

### Structs

| Name | Summary |
|------|---------|
| `EventContext` | `app: AppContext` plus optional actor/request origin metadata |
| `EventOrigin` | Optional actor, IP, user-agent, and request ID payload for listeners |
| `EventBus` | Dispatches events to registered listeners |

### Functions

```rust
fn dispatch_with_origin<E>(event: E, origin: Option<EventOrigin>) -> Result<()>      // dispatch with origin metadata
fn dispatch_job<E, J, F>(mapper: F) -> JobDispatchListener<E, J, F>         // event → job dispatch
fn publish_websocket<E, F>(mapper: F) -> WebSocketPublishListener<E, F>      // event → WS broadcast
```

---

## notifications/

Multi-channel async notifications.

### Constants

```rust
const NOTIFICATION_BROADCAST_CHANNEL: ChannelId;
const NOTIFICATION_BROADCAST_EVENT: ChannelEventId;
const NOTIFY_EMAIL: NotificationChannelId;
const NOTIFY_DATABASE: NotificationChannelId;
const NOTIFY_BROADCAST: NotificationChannelId;
```

### Traits

```rust
trait Notification: Send + Sync {
    fn notification_type(&self) -> &str;
    fn via(&self) -> Vec<NotificationChannelId>;
    fn to_email(&self, notifiable: &dyn Notifiable) -> Option<EmailMessage> { None }
    fn to_database(&self) -> Option<Value> { None }
    fn to_broadcast(&self) -> Option<Value> { None }
    fn to_channel(&self, channel: &str, notifiable: &dyn Notifiable) -> Option<Value> { None }
}

trait Notifiable: Send + Sync {
    fn notification_id(&self) -> String;
    fn route_notification_for(&self, channel: &str) -> Option<String>;
}

trait NotificationChannel: Send + Sync + 'static {
    async fn send(&self, app: &AppContext, notifiable: &dyn Notifiable, notification: &dyn Notification) -> Result<()>;
}
```

### Structs

| Name | Summary |
|------|---------|
| `NotificationChannelRegistry` | Channel registry |
| `EmailNotificationChannel` | Email delivery |
| `DatabaseNotificationChannel` | Database storage |
| `BroadcastNotificationChannel` | WebSocket broadcast |
| `SendNotificationJob` | Queued notification job |

### Functions

```rust
async fn notify(app: &AppContext, notifiable: &dyn Notifiable, notification: &dyn Notification) -> Result<()>
async fn notify_queued(app: &AppContext, notifiable: &dyn Notifiable, notification: &dyn Notification) -> Result<()>
fn build_notification_job(notifiable: &dyn Notifiable, notification: &dyn Notification) -> SendNotificationJob
```

---

## cache/

In-memory and Redis-backed caching.

### Traits

```rust
trait CacheStore: Send + Sync + 'static {
    async fn get_raw(&self, key: &str) -> Result<Option<String>>;
    async fn put_raw(&self, key: &str, value: &str, ttl: Duration) -> Result<()>;
    async fn forget(&self, key: &str) -> Result<bool>;
    async fn flush(&self) -> Result<()>;
}
```

### Structs

| Name | Summary |
|------|---------|
| `CacheManager` | Main cache interface |
| `MemoryCacheStore` | In-memory with max entries |
| `RedisCacheStore` | Redis-backed with prefix |

### CacheManager — methods

```rust
fn new(store: Arc<dyn CacheStore>) -> Self
async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>
async fn put<T: Serialize>(&self, key: &str, value: &T, ttl: Duration) -> Result<()>
async fn remember<T, F, Fut>(&self, key: &str, ttl: Duration, f: F) -> Result<T>
async fn forget(&self, key: &str) -> Result<bool>
async fn flush(&self) -> Result<()>
```

Cache keys are validated before store access. `remember()` uses local
single-flight by default and can opt into a distributed runtime lock via cache
config.

---

## redis/

Namespaced Redis wrapper.

### Structs

| Name | Summary |
|------|---------|
| `RedisManager` | Connection + namespace manager |
| `RedisConnection` | Multiplexed connection wrapper |
| `RedisKey` | Namespaced key |
| `RedisChannel` | Namespaced pub/sub channel |

### Constants

```rust
const FRAMEWORK_BOOTSTRAP_PROBE: ProbeId;
const RUNTIME_BACKEND_PROBE: ProbeId;
const REDIS_PING_PROBE: ProbeId;
```

### RedisManager — methods

```rust
fn from_config(config: &ConfigRepository) -> Result<Self>
fn namespace(&self) -> &str
fn key(&self, suffix: impl AsRef<str>) -> RedisKey
fn key_in_namespace(&self, namespace: impl AsRef<str>, suffix: impl AsRef<str>) -> RedisKey
fn channel(&self, suffix: impl AsRef<str>) -> RedisChannel
fn channel_in_namespace(&self, namespace: impl AsRef<str>, suffix: impl AsRef<str>) -> RedisChannel
fn connection(&self) -> Result<RedisConnection>
```

### RedisConnection — methods

```rust
async fn get<T: FromRedisValue>(&mut self, key: &RedisKey) -> Result<T>
async fn get_optional<T: FromRedisValue>(&mut self, key: &RedisKey) -> Result<Option<T>>
async fn set<V: ToRedisArgs>(&mut self, key: &RedisKey, value: V) -> Result<()>
async fn set_ex<V: ToRedisArgs>(&mut self, key: &RedisKey, value: V, seconds: u64) -> Result<()>
async fn del(&mut self, key: &RedisKey) -> Result<usize>
async fn del_many(&mut self, keys: &[&RedisKey]) -> Result<usize>
async fn exists(&mut self, key: &RedisKey) -> Result<bool>
async fn expire(&mut self, key: &RedisKey, seconds: u64) -> Result<bool>
async fn incr(&mut self, key: &RedisKey) -> Result<i64>
async fn publish<V: ToRedisArgs>(&mut self, channel: &RedisChannel, value: V) -> Result<usize>
async fn hget<T, F>(&mut self, key: &RedisKey, field: F) -> Result<T>
async fn hset<F, V>(&mut self, key: &RedisKey, field: F, value: V) -> Result<usize>
async fn sadd<V: ToRedisArgs>(&mut self, key: &RedisKey, value: V) -> Result<usize>
async fn srem<V: ToRedisArgs>(&mut self, key: &RedisKey, value: V) -> Result<usize>
async fn smembers<T: FromRedisValue>(&mut self, key: &RedisKey) -> Result<Vec<T>>
```

---

## logging/

Structured logging, observability, health probes.

### Enums

| Name | Variants |
|------|----------|
| `LogFormat` | `Json`, `Text` |
| `LogLevel` | `Trace`, `Debug`, `Info`, `Warn`, `Error` |
| `HttpOutcomeClass` | `Informational`, `Success`, `Redirection`, `ClientError`, `ServerError` |
| `AuthOutcome` | `Success`, `Unauthorized`, `Forbidden`, `Error` |
| `JobOutcome` | `Enqueued`, `Leased`, `Started`, `Succeeded`, `Retried`, `ExpiredLeaseRequeued`, `DeadLettered` |
| `WebSocketConnectionState` | `Opened`, `Closed` |
| `RuntimeBackendKind` | `Redis`, `Memory` |
| `SchedulerLeadershipState` | `Acquired`, `Lost` |
| `ProbeState` | `Healthy`, `Unhealthy` |

### Structs

| Name | Summary |
|------|---------|
| `RequestId(String)` | Request ID wrapper |
| `RuntimeDiagnostics` | Metrics + health manager |
| `RuntimeSnapshot` | Full runtime metrics snapshot |
| `HttpRuntimeSnapshot` | HTTP metrics |
| `HttpDurationHistogramSnapshot` | HTTP latency histogram |
| `HttpDurationBucketSnapshot` | HTTP latency bucket |
| `AuthRuntimeSnapshot` | Auth metrics |
| `WebSocketRuntimeSnapshot` | WS metrics |
| `SchedulerRuntimeSnapshot` | Scheduler metrics |
| `JobRuntimeSnapshot` | Job metrics |
| `ProbeResult` | `id`, `state`, `message` |
| `LivenessReport` | `state` |
| `ReadinessReport` | `state`, `probes: Vec<ProbeResult>` |
| `ObservabilityOptions` | Guard + permission config for observability routes |

### Traits

```rust
trait ReadinessCheck: Send + Sync + 'static {
    async fn run(&self, app: &AppContext) -> Result<ProbeResult>;
}
```

### Constants

```rust
const REQUEST_ID_HEADER: &str = "x-request-id";
```

### RuntimeDiagnostics — methods

```rust
fn backend_kind(&self) -> RuntimeBackendKind
fn mark_bootstrap_complete(&self)
fn bootstrap_complete(&self) -> bool
fn liveness(&self) -> LivenessReport
fn snapshot(&self) -> RuntimeSnapshot
fn readiness_descriptors(&self) -> Vec<ReadinessProbeDescriptor>
async fn run_readiness_checks(&self, app: &AppContext) -> Result<ReadinessReport>

// Recording
fn record_http_response(&self, status: StatusCode)
fn record_http_response_with_duration(&self, status: StatusCode, duration_ms: u64)
fn record_auth_outcome(&self, outcome: AuthOutcome)
fn record_websocket_connection(&self, state: WebSocketConnectionState)
fn record_websocket_subscription_opened(&self)
fn record_websocket_subscription_closed(&self)
fn record_websocket_inbound_message(&self)
fn record_websocket_outbound_message(&self)
fn record_scheduler_tick(&self)
fn record_schedule_executed(&self)
fn record_scheduler_leadership(&self, state: SchedulerLeadershipState)
fn set_scheduler_leader_active(&self, active: bool)
fn record_job_outcome(&self, outcome: JobOutcome)
```

`/_foundry/runtime` returns the structured `RuntimeSnapshot`. `/_foundry/http/stats` additively returns
bounded HTTP route rankings, recent slow requests, and recent error samples for admin dashboards.
`/_foundry/metrics` exposes runtime counter families in Prometheus text format. Foundry does not store
Prometheus samples; scrape retention belongs to Prometheus or your metrics backend.
Generated OpenAPI documents the core JSON observability endpoints (`/_foundry/health`,
`/_foundry/ready`, `/_foundry/runtime`) with backend-owned response schemas.

HTTP runtime counters include observability endpoint traffic, while `/_foundry/http/stats` rankings
retain application routes only so dashboard polling does not crowd out useful samples.

`/_foundry/sql` preserves the existing `slow_queries` array and additively returns slow-query stats,
top-slowest ranking, and potential HTTP N+1 suspects grouped by repeated SQL fingerprint. SQL
literals and comments are redacted by default before logs or dashboard retention.

`ObservabilityConfig.enabled = false` skips `/_foundry/*` route registration. `capture_enabled = false`
keeps routes available but stops passive runtime capture; existing endpoint responses remain
available with empty or current live data. Runtime counters, HTTP samples, SQL slow queries, N+1
suspects, and WebSocket channel counters are process-local and reset on restart. `job_history` is
the persistent DB-backed observability store for job stats and failed jobs, pruned by workers using
`JobsConfig.history_retention_days`.

### ObservabilityOptions — builder

```rust
fn new() -> Self
fn guard<I>(self, guard: I) -> Self
fn permission<I>(self, permission: I) -> Self
fn permissions<I, P>(self, permissions: I) -> Self
fn access(&self) -> &AccessScope
```

### Functions

```rust
fn init(config: &ConfigRepository) -> Result<()>
```

---

## plugin/

Compile-time plugin registry with dependency validation.

### Enums

| Name | Variants |
|------|----------|
| `PluginAssetKind` | `Config`, `Migration`, `Static` |

### Structs

| Name | Summary |
|------|---------|
| `PluginManifest` | Plugin metadata: id, version, foundry_version, dependencies, assets, scaffolds |
| `PluginDependency` | Plugin ID + semver requirement |
| `PluginAsset` | Deliverable file asset |
| `PluginScaffold` | Code generation template |
| `PluginScaffoldVar` | Template variable with optional default |
| `PluginRegistrar` | Plugin registration interface |
| `PluginRegistry` | Installed plugin registry |
| `PluginContributions` | Per-plugin registration summary (route_count, command_count, etc.) |
| `PluginInstallOptions` | Asset installation options |
| `PluginScaffoldOptions` | Scaffold rendering options |

### Traits

```rust
trait Plugin: Send + Sync + 'static {
    fn manifest(&self) -> PluginManifest;
    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()>;
    async fn boot(&self, app: &AppContext) -> Result<()> { Ok(()) }   // default
    async fn shutdown(&self, app: &AppContext) -> Result<()> { Ok(()) } // default — called in reverse dep order
}
```

### PluginRegistrar — methods

```rust
// Core registration (closures)
fn new() -> Self
fn register_provider<P: ServiceProvider>(&mut self, provider: P) -> &mut Self
fn register_routes<F>(&mut self, registrar: F) -> &mut Self
fn register_commands<F>(&mut self, registrar: F) -> &mut Self
fn register_schedule<F>(&mut self, registrar: F) -> &mut Self
fn register_websocket_routes<F>(&mut self, registrar: F) -> &mut Self
fn register_validation_rule<I, R>(&mut self, id: I, rule: R) -> &mut Self
fn config_defaults(&mut self, defaults: Value) -> &mut Self
fn register_assets<I>(&mut self, assets: I) -> Result<&mut Self>
fn register_scaffolds<I>(&mut self, scaffolds: I) -> Result<&mut Self>

// Direct registration (no ServiceProvider wrapper needed)
fn register_guard<I: Into<GuardId>, G: BearerAuthenticator>(&mut self, id: I, guard: G) -> &mut Self
fn register_policy<I: Into<PolicyId>, P: Policy>(&mut self, id: I, policy: P) -> &mut Self
fn register_authenticatable<M: Authenticatable>(&mut self) -> &mut Self
fn listen_event<E: Event, L: EventListener<E>>(&mut self, listener: L) -> &mut Self
fn register_job<J: Job>(&mut self) -> &mut Self
fn register_job_middleware<M: JobMiddleware>(&mut self, middleware: M) -> &mut Self
fn register_notification_channel<I: Into<NotificationChannelId>, N: NotificationChannel>(&mut self, id: I, channel: N) -> &mut Self
fn register_datatable<D: Datatable>(&mut self) -> &mut Self
fn register_readiness_check<I: Into<ProbeId>, C: ReadinessCheck>(&mut self, id: I, check: C) -> &mut Self
fn register_storage_driver(&mut self, name: impl Into<String>, factory: StorageDriverFactory) -> &mut Self
fn register_email_driver(&mut self, name: impl Into<String>, factory: EmailDriverFactory) -> &mut Self
fn register_middleware(&mut self, config: MiddlewareConfig) -> &mut Self
```

### PluginRegistry — methods

```rust
fn new(plugins: Vec<PluginManifest>, contributions: HashMap<PluginId, PluginContributions>) -> Self
fn plugins(&self) -> &[PluginManifest]
fn plugin(&self, id: &PluginId) -> Option<&PluginManifest>
fn contributions(&self, id: &PluginId) -> Option<&PluginContributions>
fn install_assets(&self, options: &PluginInstallOptions) -> Result<Vec<PathBuf>>
fn render_scaffold(&self, options: &PluginScaffoldOptions) -> Result<Vec<PathBuf>>
fn is_empty(&self) -> bool
```

### PluginContributions — fields

```rust
pub struct PluginContributions {
    pub route_count: usize,
    pub command_count: usize,
    pub schedule_count: usize,
    pub websocket_route_count: usize,
    pub validation_rule_count: usize,
    pub provider_count: usize,
    pub middleware_count: usize,
    pub registrar_action_count: usize,
    pub asset_count: usize,
    pub scaffold_count: usize,
}
```

---

## datatable/

Server-side filtering, sorting, pagination, export.

### Enums

| Name | Variants |
|------|----------|
| `DatatableFilterOp` | `Eq`, `NotEq`, `Like`, `Gt`, `Gte`, `Lt`, `Lte`, `In`, `Date`, `DateFrom`, `DateTo`, `Datetime`, `DatetimeFrom`, `DatetimeTo`, `Has`, `HasLike`, `LikeAny` |
| `DatatableFilterValue` | `Text(String)`, `Bool(bool)`, `Number(i64)`, `Values(Vec<String>)` |
| `DatatableFilterKind` | `Text`, `Number`, `Select`, `Checkbox`, `Date`, `DateTime` |
| `DatatableFilterValueKind` | `Text`, `Boolean`, `Integer`, `Decimal`, `Date`, `DateTime`, `Values` |
| `DatatableValue` | `Null`, `String(String)`, `Number(serde_json::Number)`, `Bool(bool)`, `Date(Date)`, `DateTime(DateTime)` |

### Traits

```rust
trait DatatableQuery<Row>: Clone + Send + Sync + 'static {
    fn apply_where(self, condition: Condition) -> Self;
    fn apply_having(self, condition: Condition) -> Self;
    fn apply_order(self, order: OrderBy) -> Self;
    async fn get(&self, executor: &E) -> Result<Collection<Row>>;
    async fn paginate(&self, executor: &E, pagination: Pagination) -> Result<Paginated<Row>>;
}

trait Datatable: Send + Sync + 'static {
    const ID: &'static str;
    type Row: Serialize + Send + Sync + 'static;
    type Query: DatatableQuery<Self::Row>;
    fn query(ctx: &DatatableContext) -> Self::Query;
    fn columns() -> Vec<DatatableColumn<Self::Row>>;
    fn mappings() -> Vec<DatatableMapping<Self::Row>> { vec![] }
    async fn filters(ctx: &DatatableContext, query: Self::Query) -> Result<Self::Query> { Ok(query) }
    async fn available_filters(ctx: &DatatableContext) -> Result<Vec<DatatableFilterRow>> { Ok(vec![]) }
    fn relation_filters() -> Vec<DatatableRelationFilter<Self::Row, Self::Query>> { vec![] }
    fn default_sort() -> Vec<DatatableSort<Self::Row>> { vec![] }
    async fn json(app, actor, request) -> Result<DatatableJsonResponse>;
    async fn download(app, actor, request) -> Result<Response>;
    async fn queue_email(app, actor, request, recipient) -> Result<DatatableExportAccepted>;
}

trait DatatableExportDelivery: Send + Sync + 'static {
    async fn deliver(&self, export: GeneratedDatatableExport, recipient: &str) -> Result<()>;
}
```

### Structs

| Name | Summary |
|------|---------|
| `DatatableColumn<M>` | Column descriptor: name, label, sortable, filterable, exportable |
| `DatatableRelationFilter<M, Q>` | Typed relation-backed auto-filter declaration |
| `DatatableRelationColumn<M>` | Relation target column descriptor |
| `DatatableDescriptor` | Static registry descriptor used by generated frontend datatable manifests |
| `DatatableRelationFilterMeta` | Static relation-filter metadata: field, relation, and aliases |
| `DatatableSort<M>` | Default sort: column + direction |
| `DatatableMapping<M>` | Computed output field |
| `DatatableRequest` | Client request: page, per_page, sort, filters, search |
| `DatatableFilterInput` | Single filter: field, op, value |
| `DatatableSortInput` | Sort: field, direction |
| `DatatableContext<'a>` | Execution context: app, actor, request, locale, timezone |
| `DatatableJsonResponse` | JSON response: rows, columns, filters, pagination |
| `DatatableColumnMeta` | Column metadata for response, including sortable/filterable/exportable/relation flags |
| `DatatablePaginationMeta` | page, per_page, total, total_pages |
| `DatatableFilterField` | Filter metadata: name, kind, label, binding, options |
| `DatatableFilterBinding` | Backend filter contract: field, op, value_kind |
| `DatatableFilterOption` | Select option: value, label |
| `DatatableFilterRow` | Filter layout (single or pair) |
| `DatatableExportAccepted` | Export queued response |
| `DatatableActorSnapshot` | Serializable actor for jobs |
| `GeneratedDatatableExport` | Generated XLSX export data |
| `DatatableExportJob` | Background export job |
| `DatatableRegistry` | Registry of all datatables; `descriptors()` returns static manifest metadata |
| `NoopExportDelivery` | No-op delivery |

Relation filters are declared on the server with `Datatable::relation_filters()` and use the
normal `DatatableFilterInput` request shape. Clients may send fields such as `merchant.name`,
legacy aliases such as `merchant-name`, or declared multi-column `LikeAny` targets such as
`merchant.name|merchant.slug`; undeclared relation paths are rejected by the normal filter
validation flow.

---

## i18n/

Locale extraction and translation.

### Structs

| Name | Summary |
|------|---------|
| `I18nManager` | Translation catalog manager |
| `Locale(String)` | Per-request locale wrapper |
| `I18n` | Axum extractor — locale + translation |

### Macros

```rust
t!(i18n, "key")                     // simple translation
t!(i18n, "key {{var}}", var = "val") // with interpolation
```

### I18nManager — methods

```rust
fn load(config: &I18nConfig) -> Result<Self>
fn translate(&self, locale: &str, key: &str, values: &[(&str, &str)]) -> String
fn resolve_locale(&self, accept_language: &str) -> String
fn default_locale(&self) -> &str
fn fallback_locale(&self) -> &str
fn has_locale(&self, locale: &str) -> bool
fn locale_list(&self) -> Vec<&str>
fn descriptor(&self) -> I18nManifestDescriptor
```

### I18n (extractor) — methods

```rust
fn t(&self, key: &str) -> String
fn t_with(&self, key: &str, values: &[(&str, &str)]) -> String
fn locale(&self) -> &str
```

---

## translations/

Model field translations across locales.

### Structs

| Name | Summary |
|------|---------|
| `ModelTranslation` | Translation record: translatable_type, translatable_id, locale, field, value |
| `TranslatedFields` | Translations for one field across locales |

### Traits

```rust
trait HasTranslations {
    fn translatable_type() -> &'static str;
    fn translatable_id(&self) -> String;
    async fn set_translation(&self, app: &AppContext, locale: &str, field: &str, value: &str) -> Result<()>;
    async fn set_translations(&self, app: &AppContext, locale: &str, values: &[(&str, &str)]) -> Result<()>;
    async fn translation(&self, app: &AppContext, locale: &str, field: &str) -> Result<Option<String>>;
    async fn translations_for(&self, app: &AppContext, locale: &str) -> Result<HashMap<String, String>>;
    async fn translated_field(&self, app: &AppContext, field: &str) -> Result<TranslatedFields>;
    async fn all_translations(&self, app: &AppContext) -> Result<Vec<ModelTranslation>>;
    async fn delete_translations(&self, app: &AppContext, locale: &str) -> Result<u64>;
}
```

### Constants

```rust
task_local! { pub static CURRENT_LOCALE: String; }
```

### Functions

```rust
fn current_locale(app: &AppContext) -> String
```

Translation reads participate in the active model extension cache. Use
`ModelQuery::with_translated_field(...)`, `with_translations_for(...)`, or
`with_all_translations()` for explicit eager loading. If a helper is accessed without eager loading
inside an active scope, Foundry lazily batch-loads the same access shape for known sibling models.

---

## cli/

Command-line interface registration.

### Structs

| Name | Summary |
|------|---------|
| `CommandInvocation` | Context: app + arg matches |
| `CommandRegistry` | Command registry |

### Type Aliases

```rust
type CommandRegistrar = Arc<dyn Fn(&mut CommandRegistry) -> Result<()> + Send + Sync>;
```

### CommandInvocation — methods

```rust
fn app(&self) -> &AppContext
fn matches(&self) -> &ArgMatches
```

### CommandRegistry — methods

```rust
fn new() -> Self
fn command<I, F, Fut>(&mut self, id: I, command: Command, handler: F) -> Result<&mut Self>
```

---

## testing/

Test infrastructure.

### Functions

```rust
fn assert_safe_to_wipe(db_url: &str) -> Result<()>
```

### Structs

| Name | Summary |
|------|---------|
| `TestApp` | Test application bootstrapper |
| `TestAppBuilder` | Builder for TestApp |
| `TestClient` | HTTP test client |
| `TestRequestBuilder` | Request builder |
| `TestResponse` | Response assertions |
| `FactoryBuilder<M>` | Model factory builder |

### Traits

```rust
trait Factory: Model {
    fn definition() -> Vec<(&'static str, DbValue)>;
}
```

### TestApp

```rust
fn builder() -> TestAppBuilder
fn app(&self) -> &AppContext
fn client(&self) -> TestClient
```

### TestAppBuilder

```rust
fn load_config_dir(self, path: impl Into<PathBuf>) -> Self
fn register_provider<P>(self, provider: P) -> Self
fn register_routes<F>(self, registrar: F) -> Self
fn register_middleware(self, config: MiddlewareConfig) -> Self
async fn build(self) -> TestApp
```

### TestClient

```rust
fn get(&self, path: &str) -> TestRequestBuilder
fn post(&self, path: &str) -> TestRequestBuilder
fn put(&self, path: &str) -> TestRequestBuilder
fn patch(&self, path: &str) -> TestRequestBuilder
fn delete(&self, path: &str) -> TestRequestBuilder
```

### TestRequestBuilder

```rust
fn header(self, name: &str, value: &str) -> Self
fn bearer_auth(self, token: &str) -> Self
fn json(self, value: &impl Serialize) -> Self
async fn send(self) -> TestResponse
```

### TestResponse

```rust
fn status(&self) -> StatusCode
fn header(&self, name: &str) -> Option<&str>
fn json<T: DeserializeOwned>(&self) -> T
fn text(&self) -> String
fn bytes(&self) -> &[u8]
```

### FactoryBuilder\<M\>

```rust
fn new() -> Self
fn set(self, column: &str, value: impl Into<DbValue>) -> Self
fn count(self, n: usize) -> Self
async fn create<E>(self, executor: &E) -> Result<Vec<M>>
async fn create_one<E>(self, executor: &E) -> Result<M>
```

---

## metadata/

Key-value metadata for models.

### Structs

```rust
struct ModelMeta { id, metadatable_type, metadatable_id, key, value: Option<Value> }
```

### Traits

```rust
trait HasMetadata {
    fn metadatable_type() -> &'static str;
    fn metadatable_id(&self) -> String;
    async fn set_meta(&self, app: &AppContext, key: &str, value: impl Serialize + Send) -> Result<()>;
    async fn get_meta<T: DeserializeOwned>(&self, app: &AppContext, key: &str) -> Result<Option<T>>;
    async fn get_meta_raw(&self, app: &AppContext, key: &str) -> Result<Option<Value>>;
    async fn forget_meta(&self, app: &AppContext, key: &str) -> Result<bool>;
    async fn has_meta(&self, app: &AppContext, key: &str) -> Result<bool>;
    async fn all_meta(&self, app: &AppContext) -> Result<Vec<ModelMeta>>;
}
```

---

## openapi/

OpenAPI 3.1.0 spec generation.

### Traits

```rust
trait ApiSchema {
    fn schema() -> Value;
    fn schema_name() -> &'static str;
}
```

### Structs

| Name | Summary |
|------|---------|
| `SchemaRef` | Type-erased schema reference |
| `RouteDoc` | Route documentation builder |
| `DocumentedRoute` | `method`, `path`, `doc` |

### RouteDoc — builder

```rust
fn new() -> Self
fn method(self, m: &str) -> Self
fn get(self) / fn post(self) / fn put(self) / fn patch(self) / fn delete(self) -> Self
fn summary(self, s: &str) -> Self
fn description(self, d: &str) -> Self
fn tag(self, t: &str) -> Self
fn request<T: ApiSchema>(self) -> Self
fn response<T: ApiSchema>(self, status: u16) -> Self
fn deprecated(self) -> Self
```

### Functions

```rust
fn generate_openapi_spec(title: &str, version: &str, routes: &[DocumentedRoute]) -> Value
fn try_generate_openapi_spec(title: &str, version: &str, routes: &[DocumentedRoute]) -> Result<Value>
```

---

## app_enum/

Enum metadata and serialization.

### Enums

| Name | Variants |
|------|----------|
| `EnumKey` | `String(String)`, `Int(i32)` |
| `EnumKeyKind` | `String`, `Int` |

### Structs

| Name | Summary |
|------|---------|
| `EnumOption` | `value: EnumKey`, `label_key: String` |
| `EnumMeta` | `id`, `key_kind`, `options` |

### Traits

```rust
trait FoundryAppEnum: Sized {
    const DB_TYPE: DbType;
    fn id() -> &'static str;
    fn key(self) -> EnumKey;
    fn keys() -> Collection<EnumKey>;
    fn parse_key(key: &str) -> Option<Self>;
    fn label_key(self) -> &'static str;
    fn options() -> Collection<EnumOption>;
    fn meta() -> EnumMeta;
    fn key_kind() -> EnumKeyKind;
}
```

### Functions

```rust
fn to_snake_case(name: &str) -> String
fn to_title_text(name: &str) -> String
```

---

## attachments/

File attachments with lifecycle.

### Structs

| Name | Summary |
|------|---------|
| `Attachment` | Attachment record with disk, path, name, mime, size, etc. |
| `AttachmentSpec` | Model-level collection policy for attachment uploads. |
| `AttachmentImagePolicy` | Image resize, format, quality, and upscale policy. |
| `AttachmentUploadBuilder` | Upload pipeline builder |

### Traits

```rust
trait HasAttachments {
    fn attachable_type() -> &'static str;
    fn attachable_id(&self) -> String;
    fn attachment_specs() -> Vec<AttachmentSpec<Self>>;
    async fn attach(&self, app: &AppContext, collection: &str, file: UploadedFile) -> Result<Attachment>;
    async fn replace_attachment(&self, app: &AppContext, collection: &str, file: UploadedFile) -> Result<Attachment>;
    async fn attach_localized(&self, app: &AppContext, collection: &str, locale: &str, file: UploadedFile) -> Result<Attachment>;
    async fn replace_localized_attachment(&self, app: &AppContext, collection: &str, locale: &str, file: UploadedFile) -> Result<Attachment>;
    async fn localized_attachment(&self, app: &AppContext, collection: &str, locale: &str) -> Result<Option<Attachment>>;
    async fn localized_attachments(&self, app: &AppContext, collection: &str, locale: &str) -> Result<Vec<Attachment>>;
    async fn localized_attachment_or_default(&self, app: &AppContext, collection: &str, locale: &str) -> Result<Option<Attachment>>;
    async fn current_localized_attachment(&self, app: &AppContext, collection: &str) -> Result<Option<Attachment>>;
    async fn attachment(&self, app: &AppContext, collection: &str) -> Result<Option<Attachment>>;
    async fn attachments(&self, app: &AppContext, collection: &str) -> Result<Vec<Attachment>>;
    async fn detach(&self, app: &AppContext, attachment_id: &str) -> Result<()>;
    async fn detach_keep_file(&self, app: &AppContext, attachment_id: &str) -> Result<()>;
    async fn detach_all(&self, app: &AppContext, collection: &str) -> Result<u64>;
}
```

```rust
trait AttachmentSpecHook<M> {
    async fn before_store(&self, ctx: AttachmentBeforeStoreContext<'_, M>) -> Result<()>;
    async fn after_store(&self, ctx: AttachmentAfterStoreContext<'_, M>) -> Result<()>;
}
```

### Functions

```rust
fn available_attachment_locales(app: &AppContext) -> Result<Vec<String>>
fn localized_attachment_collection(collection: &str, locale: &str) -> String
```

### Attachment — methods

```rust
fn upload(file: UploadedFile) -> AttachmentUploadBuilder
fn is_image(&self) -> bool
fn is_video(&self) -> bool
fn is_audio(&self) -> bool
fn is_document(&self) -> bool
fn extension(&self) -> Option<&str>
fn human_size(&self) -> String
async fn url(&self, app: &AppContext) -> Result<String>
async fn temporary_url(&self, app: &AppContext, expires_at: DateTime) -> Result<String>
async fn image(&self, app: &AppContext) -> Result<ImageProcessor>
```

### AttachmentUploadBuilder — methods

```rust
fn collection(self, collection: impl Into<String>) -> Self
fn disk(self, disk: impl Into<String>) -> Self
fn resize(self, width: u32, height: u32) -> Self
fn resize_to_fit(self, max_width: u32, max_height: u32) -> Self
fn resize_to_fill(self, width: u32, height: u32) -> Self
fn format(self, format: ImageFormat) -> Self
fn quality(self, quality: u8) -> Self
fn upscale(self, upscale: bool) -> Self
async fn store(self, app: &AppContext, attachable_type: &str, attachable_id: &str) -> Result<Attachment>
```

### AttachmentSpec — methods

```rust
fn file(collection: impl Into<String>) -> Self
fn image(collection: impl Into<String>) -> Self
fn single(self) -> Self
fn resize_exact(self, width: u32, height: u32) -> Self
fn resize_to_fit(self, max_width: u32, max_height: u32) -> Self
fn resize_to_fill(self, width: u32, height: u32) -> Self
fn format(self, format: ImageFormat) -> Self
fn quality(self, quality: u8) -> Self
fn upscale(self, upscale: bool) -> Self
fn hook<H>(self, hook: H) -> Self
```

Attachment reads participate in the active model extension cache. Use
`ModelQuery::with_attachments(...)` or nested relation builder `with_attachments(...)` for explicit
eager loading. If a helper is accessed without eager loading inside an active scope, Foundry lazily
batch-loads that collection for known sibling models.

---

## countries/

Built-in country data (250 countries).

### Structs

| Name | Summary |
|------|---------|
| `Country` | Full country record: iso2, iso3, name, capital, region, currencies, calling_code, timezones, etc. |
| `CountrySeed` | Seed data record |
| `CountryCurrency` | `code`, `name`, `symbol`, `minor_units` |

### Country — methods

```rust
async fn find(app: &AppContext, iso2: &str) -> Result<Option<Country>>
async fn all(app: &AppContext) -> Result<Vec<Country>>
async fn by_status(app: &AppContext, status: &str) -> Result<Vec<Country>>
async fn enabled(app: &AppContext) -> Result<Vec<Country>>
async fn exists(app: &AppContext, iso2: &str) -> Result<bool>
```

### Functions

```rust
fn load_seed() -> Result<Vec<CountrySeed>>
async fn seed_countries_with(executor: &dyn QueryExecutor) -> Result<u64>
async fn seed_countries(app: &AppContext) -> Result<u64>
```

---

## imaging/

Image processing pipeline.

### Enums

| Name | Variants |
|------|----------|
| `ImageFormat` | `Jpeg`, `Png`, `WebP`, `Gif`, `Bmp`, `Tiff`, `Avif`, `Ico` |
| `Rotation` | `Deg90`, `Deg180`, `Deg270` |

### Structs

```rust
struct ImageProcessor; // chainable image processor
```

### ImageProcessor — methods

```rust
fn open<P: AsRef<Path>>(path: P) -> Result<Self>
fn from_bytes(bytes: &[u8]) -> Result<Self>
fn width(&self) -> u32
fn height(&self) -> u32
fn format(&self) -> Option<ImageFormat>

// Transforms (all chainable)
fn resize(self, width: u32, height: u32) -> Self
fn resize_to_fit(self, max_width: u32, max_height: u32) -> Self
fn resize_to_fill(self, width: u32, height: u32) -> Self
fn crop(self, x: u32, y: u32, width: u32, height: u32) -> Self
fn quality(self, q: u8) -> Self
fn blur(self, sigma: f32) -> Self
fn grayscale(self) -> Self
fn rotate(self, rotation: Rotation) -> Self
fn flip_horizontal(self) -> Self
fn flip_vertical(self) -> Self
fn brightness(self, value: i32) -> Self
fn contrast(self, value: f32) -> Self

// Output
fn save<P: AsRef<Path>>(&self, path: P) -> Result<()>
fn save_as<P: AsRef<Path>>(&self, path: P, format: ImageFormat) -> Result<()>
fn to_bytes(&self, format: ImageFormat) -> Result<Vec<u8>>
```

### ImageFormat — methods

```rust
fn from_extension(ext: &str) -> Option<Self>
fn extension(&self) -> &'static str
```
