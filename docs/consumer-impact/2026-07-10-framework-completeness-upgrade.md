# Framework Completeness Upgrade — Consumer Impact

This document is intended to be forwarded to Foundry consumer teams. It records required migrations and newly available first-class framework support introduced while resolving the 2026-07-10 Laravel-inspired gap report.

Only changes that can require consumer action, alter a public contract, or add a new first-class module belong here. It will be extended as the implementation proceeds.

## TypeScript DTO registration is now explicit

`foundry::ApiSchema` now owns only JSON Schema/OpenAPI registration. It no longer implicitly registers the type as a standalone TypeScript export.

Add `foundry::TS` to every DTO that should be written by `types:export`:

```rust
// Before
#[derive(serde::Serialize, ts_rs::TS, foundry::ApiSchema)]
struct UserResponse {
    id: String,
}

// After
#[derive(serde::Serialize, ts_rs::TS, foundry::TS, foundry::ApiSchema)]
struct UserResponse {
    id: String,
}
```

No change is required for OpenAPI-only DTOs. They may continue deriving only `foundry::ApiSchema`. `AppEnum` also retains its automatic TypeScript metadata export.

After updating derives, run:

```bash
cargo run -- types:export
```

Verify that every consumer-imported DTO remains present under the configured `typescript.output_dir`. A type that intentionally omits `foundry::TS` may disappear from standalone generated files while remaining available as JSON Schema/OpenAPI metadata.

## Serde field names now define every DTO wire boundary

`#[serde(rename = "...")]` and `#[serde(rename_all = "...")]` now drive validation error keys, multipart field matching, OpenAPI properties, enum values, and generated validation metadata as well as serde JSON.

If a DTO previously accepted multipart fields or exposed validation errors under its Rust field names despite using serde renames, update the consumer to send and read the serialized names. For example, `display_name` under `#[serde(rename_all = "camelCase")]` is now consistently `displayName`.

Foundry requires one shared wire name. Asymmetric serde declarations such as different `serialize` and `deserialize` rename rules are rejected for DTOs deriving `ApiSchema` or `Validate`. Align both directions or split the read and write DTOs.

## Empty WebAuthn feature removed

The `webauthn` Cargo feature and empty module were removed because they did not provide passkey behavior. Remove `features = ["webauthn"]` from consumer dependency declarations. No runtime authentication migration is needed because the feature previously implemented nothing.

## Bootstrap now reports invalid dotenv files

A missing optional `.env` file remains valid. A present file that is malformed, unreadable, or otherwise cannot be loaded now fails bootstrap instead of being silently ignored.

Validate deployed `.env` files before upgrading. If configuration is injected entirely by the environment, no action is required when `.env` is absent.

## Login lockout threshold is exact

`LoginThrottle.max_failures = N` now locks on the Nth recorded failure. Previous builds locked one attempt early. Review configured thresholds if operational policy compensated for the old off-by-one behavior.

## Setting updates now distinguish missing keys

`Setting::set` now returns an error when the requested key does not exist. Use `Setting::upsert` when creating a missing setting is intentional. Corrupt or unknown persisted setting types now surface a hydration error instead of being treated as text.

## Country values are strongly typed

The public country fields changed as follows:

- `currencies` is now `Vec<CountryCurrency>`.
- `calling_suffixes`, `tlds`, and `timezones` are now `Vec<String>`.
- `CountryStatus::parse` now returns `Option<CountryStatus>` and rejects unknown values.

Serialized JSON shapes remain arrays/objects. Update Rust consumers that previously indexed these fields as `serde_json::Value`, and handle `CountryStatus::parse` explicitly. Invalid persisted status or collection JSON now produces a contextual hydration error.

## Rate-limit actor keys are guard-scoped

Actor rate-limit storage keys now include both guard and actor ID. Existing short-lived actor buckets reset once during deployment; no application-code change is required.

## HTML email variables are escaped by default

In HTML templates, `{{variable}}` now HTML-escapes the substituted value. Trusted markup must use explicit triple-brace syntax:

```html
<!-- Escaped text -->
<p>{{customer_name}}</p>

<!-- Trusted or pre-sanitized markup -->
<div>{{{summary_html}}}</div>
```

Review templates whose variables intentionally contain markup and migrate only those placeholders to triple braces. Text templates remain unescaped, and replacements are non-recursive.

## SMTP `none` is now genuinely plaintext

`encryption = "none"` no longer initiates STARTTLS. Use this setting only with a trusted local relay. Deployments that relied on the previous accidental encryption should change the setting to `"starttls"` before upgrading.

## Localization loading is deterministic and strict

Locale matching is now case-insensitive while preserving canonical directory spelling. Case-only duplicate locale directories and duplicate flattened translation keys across JSON files fail bootstrap instead of resolving by filesystem order.

Remove duplicate keys before upgrading. When a model translation has neither the current nor default locale, fallback now uses the lexicographically first locale; output may change from the previous nondeterministic selection.

## Custom validation rules preserve their returned message

When no inline, validator, or i18n override exists, the message returned by a custom validation rule is now sent to the consumer instead of a generic fallback. Clients should continue branching on stable error codes rather than exact display text.

## Password work-factor upgrades

`HashManager::needs_rehash` is new first-class support for detecting stored Argon2 hashes that use an older algorithm, version, or work factor. A typical login flow checks the password first, then rehashes it after successful authentication when this method returns `true`.

## Fluent HTTP test assertions

`TestResponse` now provides first-class fluent assertions for common status codes, headers, exact/path/fragment/shape JSON, validation errors, redirects, and downloads. Existing accessor-based tests remain compatible; consumers may adopt the assertions incrementally.

`TestAppBuilder::replace_service` and `replace_service_arc` can replace an
already registered service after bootstrap and before router construction.
Production duplicate registration remains an error.

## First-class testing fakes, auth/time helpers, and database isolation

`EventFake`, `JobFake`, `MailFake`, and `NotificationFake` can now be installed
directly on `TestAppBuilder`. They record typed/predicate-assertable work and
suppress the corresponding listeners, queue backend, transport, or delivery
channels. `TestAppBuilder::fake_http` installs the existing `HttpClientFake`,
and `ClockFake` controls application time read through `AppContext::clock()`.

`TestClient` and `TestRequestBuilder` now support `acting_as`. The supplied
actor bypasses credential lookup but must match the route guard and still runs
MFA, permission, policy, dynamic authorization, and post-auth middleware checks.
Client-wide bearer and configured session-cookie defaults are available through
`with_bearer_token` and `with_session`; request-level helpers remain available.

`DatabaseTestTransaction` is a `QueryExecutor` and `ModelWriteExecutor` intended
for one test's factories, queries, and typed `assert_database_has`,
`assert_database_missing`, and `assert_database_count` calls. End the test with
explicit `rollback()` so rollback errors remain visible. Rollback intentionally
drops deferred after-commit callbacks; tests that verify after-commit behavior
must commit separately and clean up through an isolated schema/table strategy.
Factories also support reusable typed states, indexed sequences, and typed
belongs-to keys through `for_parent`.

`StorageFake` stays behind the production `StorageAdapter` boundary. Register
`storage_fake.driver_factory()` as a custom driver from a test provider and
select it in the test disk config; Foundry does not silently replace the
application's configured `StorageManager`.

These APIs are entirely additive. Existing tests remain compatible, and there
is no required consumer migration, database migration, runtime configuration
change, or breaking API change. Storage fake configuration is opt-in only for
tests that select that custom driver.

## Custom kernel lifecycle shutdown

Consumers using a public `build_*_kernel` method must call
`kernel.app().shutdown().await` when their custom process host stops. The method
drains framework-managed background tasks and runs plugin shutdown hooks in the
same way as Foundry's standard `run_*` methods. It is safe to call repeatedly.

## Reusable plugin author tests

`PluginTestHarness` and `PluginTestApp` provide an isolated author test surface
that still uses Foundry's production plugin bootstrap, dependency resolution,
config precedence, routing, and lifecycle behavior:

```rust
let app = PluginTestHarness::new(MY_PLUGIN_ID, MyPlugin)
    .register_plugin(MyDependencyPlugin)
    .build()
    .await?;

assert_eq!(app.manifest().id(), &MY_PLUGIN_ID);
assert_eq!(app.contributions().route_count, 1);
app.shutdown().await?;
```

`TestAppBuilder::register_plugin` and `register_plugins` are additive
conveniences for tests that prefer the general test-app surface. Existing
plugin tests and `TestApp::from_builder(...)` remain compatible. This is
additive, adds no configuration, and introduces no breaking consumer change or
required migration.

## Redis cache flush and cache tags

Redis-backed `cache.flush()` is now supported through namespace generation
keys. Existing cache entries become cold once after deployment; old physical
keys remain only until their current TTL expires.

`CacheManager::tags` / `TaggedCache` is new first-class support for shared
version-based invalidation across processes. Custom `CacheStore`
implementations remain source-compatible, but must override `get_control_raw`
and `put_control_raw` before tag flushing can work on that backend.

## Job scheduling and history behavior

`JobDispatcher::dispatch_at(DateTime)` and `dispatch_after(Duration)` provide
typed absolute and relative scheduling. Existing `dispatch_later(..., i64)`
callers remain compatible.

`jobs.track_history = true` now writes persistent job history even when
`observability.capture_enabled = false`. Set `track_history = false` explicitly
on deployments that do not want those database writes.

## Event IDs and after-commit dispatch

Every Rust event type must now have a globally unique `Event::ID`, including
events contributed by plugins. Resolve any bootstrap collision by assigning a
distinct semantic ID; multiple listeners for the same event type remain valid.

`AppTransaction::dispatch_event_after_commit` is new first-class support. It
captures the transaction/current actor and request origin when called, then
dispatches only after a successful commit.

## Selected notification channels must produce output

When `Notification::via()` selects email, database, or broadcast, the
notification must provide the required route/message/payload. Immediate and
queued preparation now return an explicit error for missing output instead of
silently reporting success. Renderers for unselected channels remain untouched.

## Guard-scoped WebSocket force-disconnect

`WebSocketPublisher::disconnect_user(actor_id)` was replaced by the explicit
guard-scoped API:

```rust
app.websocket()?
    .disconnect_actor(GuardId::new("web"), actor_id)
    .await?;
```

Direct publishers to the internal `__system:disconnect` topic must serialize
both `guard` and `actor_id`. Coordinate mixed-version deployments: new kernels
reject legacy actor-only payloads, while old kernels do not understand the new
guard boundary and retain broad ID-only behavior.

## Typed HTTP route-model binding

`ModelPath<M>` is new first-class support for binding a route segment to a
model's declared primary-key type. It returns 400 for a malformed key and 404
for a valid but absent key, and supports UUID as well as manual integer/text
primary keys:

```rust
async fn show(ModelPath(user): ModelPath<User>) -> Json<UserResource> {
    Json(UserResource::make(&user))
}
```

Existing manual `Path` plus query handlers remain compatible.

## Bounded image decoding and explicit JPEG quality

`ImageProcessor::open` and `from_bytes` now apply `ImageDecodeLimits::default()`:
50 MiB input, 50 million pixels, and 12,000 pixels per dimension. Use the
`*_with_limits` variants at stricter boundaries. `open_unbounded` and
`from_bytes_unbounded` are explicit trusted-input escape hatches.

`ImageProcessor::process_file`, `process_bytes`, and their custom-limit variants
run the full decode/transform closure on Foundry's blocking pool and are the
preferred APIs inside async handlers and jobs.

`.quality(...)` is now JPEG-only. WebP uses lossless encoding; remove a quality
call before WebP/non-JPEG output or change the output to JPEG when lossy quality
control is required. Previously ignored non-JPEG quality settings now fail
clearly.

## Graceful encryption-key rotation

`CryptConfig.previous_keys` is new first-class decrypt-only key rotation support:

```toml
[crypt]
key = "NEW_BASE64_KEY"
previous_keys = ["OLD_BASE64_KEY"]
```

New ciphertext always uses `key`. Decryption tries the primary key, then prior
keys in listed order. Keep the old primary in `previous_keys` during rollout,
rewrite or naturally expire old ciphertext, then remove it. Every configured
key is validated at bootstrap and all key values remain redacted from debug
output.

Consumers constructing `CryptConfig` with a Rust struct literal must add
`previous_keys: Vec::new()` or use `..CryptConfig::default()`.

## Country timezone seed data is populated

`seed_countries` now writes IANA tzdb 2026a timezone identifiers instead of an
empty array for every country. Rerun the idempotent countries seeder after
upgrading to populate existing rows. Bouvet Island (`BV`) and Heard Island and
McDonald Islands (`HM`) remain empty because IANA assigns neither a zone;
Kosovo's user-assigned `XK` code explicitly uses `Europe/Belgrade`.

## First-class outbound HTTP client and fake

`AppContext::http_client()` now resolves a default pooled `HttpClient`. The new
module provides upstream base URLs/default headers, connect and per-attempt
request timeouts, shared concurrency limits, conservative retry policy, typed
requests/responses/errors, redacted tracing, pluggable transports, and a raw
reqwest escape hatch.

The default has no base URL, a 10-second connect timeout, a 30-second request
timeout, concurrency 64, and up to three total attempts for read-only
`GET`/`HEAD`/`OPTIONS` calls on transient transport/status failures. Mutations
are not retried unless explicitly opted in. A provider-registered `HttpClient`
wins over the framework default.

`HttpClientFake` is new testing support for queued responses/errors, recorded
typed requests, and closure-based request assertions. Register `fake.client()`
as the `HttpClient` singleton (or use `TestAppBuilder::replace_service`) so
unexpected calls fail with `FakeExhausted` instead of reaching the network.

Calling `HttpClient::raw()` deliberately bypasses Foundry retry, timeout,
concurrency, tracing, and fake behavior.

## WebSocket shutdown now closes and drains live sockets

WebSocket kernel shutdown now sends close code 1001 with reason
`server shutdown`, rejects racing upgrades with 503, and drains subscriptions,
presence, actor tracking, and lifecycle hooks. Existing
`app.background_shutdown_timeout_ms` now also bounds this cleanup; a value of
`0` means immediate cutoff. No public Rust signature changed.

## Model translations support manual primary keys

`model_translations.translatable_id` and all runtime bindings now use text, so
`HasTranslations` supports generated UUID IDs plus manual integer and text keys.
`translatable_id()` continues returning `String`; no trait signature changed.

For an existing app, publish and run the new schema migration before deploying
the new runtime:

```bash
cargo run -- migrate:publish
cargo run -- db:migrate
```

The migration `000000000013_alter_model_translation_ids_to_text` converts the
existing UUID column with `USING translatable_id::text`; indexes and existing
values remain intact. Coordinate deployment because new code sends text
bindings that an old UUID column rejects. The down migration intentionally
refuses to restore UUID once any non-UUID ID has been stored.

## Cursor pagination tokens and ordering changed

Cursor pagination now owns deterministic ascending `(selected column, primary
key)` ordering, including PostgreSQL `NULLS LAST` behavior, and produces
automatic previous/next tokens from real row positions. Earlier `order_by` and
offset clauses are cleared. Supplying both `after` and `before` is an error.

Old single-display-value cursor tokens are intentionally invalid because they
cannot identify a row among duplicate sort values. Remove any calls to the
deleted `CursorPaginated::encode_cursor` and `with_cursors` helpers. Discard
persisted cursors, make one request without `after`/`before`, then pass returned
`cursors.next` and `cursors.prev` strings through unchanged. The new tokens are
opaque, versioned, table/column/type-bound positions; there is no legacy escape
hatch.

## Email queue and template configuration are active

`EmailManager::queue` and `queue_later` now dispatch on `[email].queue`, and
worker kernels automatically poll that queue. Deployments that set it to a
non-default value must run a worker with the upgraded framework; old workers
only polling registered/global queues will not consume it. Add the queue under
`jobs.queue_priorities` when its relative priority should be explicit.

`EmailManager::render_template(message, name, variables)` is new first-class
support that resolves the template beneath `[email].template_path`.
`EmailMessage::template(name, explicit_path, variables)` remains compatible as
the deliberate per-call escape hatch.

`JobDispatcher` also exposes `dispatch_on`, `dispatch_at_on`,
`dispatch_after_on`, and `dispatch_later_on`. Dynamic application queues should
be listed in `jobs.queue_priorities` so long-running workers include them in
their poll set.

## Explicit security tier and configuration diagnostics

`AppConfig.security_tier: Option<SecurityTier>` separates security posture from
the descriptive environment label. Defaults are relaxed for
`development`/`testing`, strict for `production`/`staging`, and fail-closed
strict for every custom label. A custom label without an explicit tier produces
a doctor warning (and fails `doctor --strict`) until confirmed:

```toml
[app]
environment = "preview-eu"
security_tier = "strict"
```

The explicit tier may override any built-in label, including an intentional
relaxed production/staging environment. WebSocket origin handling, trusted
proxy warnings, and public-observability warnings now use this resolved tier.
Review custom labels before upgrading. Rust `AppConfig` struct literals must add
`security_tier` or use `..AppConfig::default()`.

Generated environment examples now use collision-safe names such as
`FOUNDRY__DATABASE__URL`. Legacy `DATABASE__URL`-style overlays still work at
lower precedence, but `doctor` reports them for migration. Doctor also reports
unknown framework TOML paths and unknown `FOUNDRY__` overlays using the
published config schema as its source of truth; `doctor --strict` turns these
warnings into a non-zero result. Custom top-level application config remains
allowed.

## Logging initialization, request IDs, and file delivery changed

Logging bootstrap now returns tracing-subscriber, file-writer, and
OpenTelemetry exporter setup errors. A process that installs its own global
subscriber must declare that ownership explicitly:

```rust
let builder = App::builder().use_external_tracing_subscriber();
```

The opt-out keeps Foundry tracing events but does not install Foundry stdout,
file, or OpenTelemetry layers. `TestAppBuilder` exposes the same method for test
processes with a shared subscriber.

`RequestId` now accepts only non-empty visible ASCII values up to 128 bytes.
Use `RequestId::try_new`, `FromStr`, or `TryFrom` for untrusted values;
`RequestId::new` intentionally panics on invalid programmer input. Invalid
inbound `x-request-id` headers are replaced with generated UUIDv7 values and
the replacement is echoed in the response.

JSON file logs now pass through a bounded, non-blocking background writer. Add
the new fields to direct `LoggingConfig` literals or use
`..LoggingConfig::default()`:

```toml
[logging]
file_queue_capacity = 8192
file_max_record_bytes = 65536
file_flush_timeout_ms = 5000
```

A full queue drops the newest record, an oversized record is dropped whole,
and a disconnected writer rejects new records. `AppContext::shutdown()` waits
up to the configured deadline for accepted records to flush. Monitor
`RuntimeSnapshot.logging` or the `foundry_log_file_*` Prometheus series for
pressure, drops, write errors, and flush timeouts.

## First-class capturable CLI I/O and exit statuses

Command handlers can now write, prompt, confirm, and report progress through
`CommandInvocation` instead of process-global `println!`/stdin calls. Existing
`CommandRegistry::command` handlers still return `Result<()>` unchanged:

```rust
registry.command(GREET, Command::new("greet"), |inv| async move {
    let name = inv.prompt("Name")?;
    inv.line(format!("Hello, {name}"))?;
    Ok(())
})?;
```

Use `command_with_exit` for a typed nonzero outcome and read it through
`CliKernel::run_status()` or `run_with_args_status(...)`. Compatibility
`run()`/`run_with_args(...)` continue returning `Result<()>` and map nonzero
statuses to an error.

Tests can pass `CommandIoFake` through `CliKernel::with_io(...)`, queue prompt
answers, and assert captured stdout/stderr. Help and version rendering also use
the injected stream. This is additive; consumers need no migration unless they
want to replace direct terminal calls for testability.

## Local multi-process development command

The CLI boot profile now includes `dev`, which supervises the current
application executable under the existing `PROCESS=http`, `worker`,
`scheduler`, and `websocket` values. Omitting positional process names starts
all four; passing names starts only that subset. Output is process-prefixed,
Ctrl+C/SIGTERM cleans up all children, and either a clean child exit or one
exhausted process stops its siblings rather than leaving a partially running
development stack.

Restarts remain disabled by default. Teams can opt in with
`--max-restarts <COUNT>` (maximum 100) and
`--restart-backoff-ms <MILLISECONDS>` (100–60000 ms); retry delay doubles up to
60 seconds. This is additive and requires no configuration, database migration,
or application API change. The application entry point must already dispatch
the documented `PROCESS` values to the matching kernels. `dev` is not a
starter-project generator or installer.

## Model scaffold table naming and inert query API

`make:model` now accepts `--table <TABLE>`. Its default table name uses a small,
predictable pluralizer, so `Category` generates `categories` and `Status`
generates `statuses` instead of `categorys`/`statuss`. Use the explicit option
for irregular or existing schemas:

```bash
cargo run -- make:model --name Person --table people
```

The generated identifier is restricted to lowercase PostgreSQL identifier
characters so it can be embedded safely in the model attribute.

`ModelQuery::without_defaults()` was removed. It only toggled an unread field;
Foundry has no `always_with` derive or automatic default-relation execution, so
calling it never affected SQL or eager loading. Remove the call. A future
default-eager-loading feature, if justified, will ship as a complete typed
contract rather than preserving this inert switch.

## Component-level scaffold commands

Foundry now provides one-file generators for the repeated application
components covered by `make:request`, `make:dto`, `make:policy`, `make:event`,
`make:listener`, `make:notification`, `make:mail`, `make:datatable`,
`make:plugin`, and `make:test`. These commands do not generate or install a
starter application.

This is additive and requires no configuration or migration. Each command
accepts `--name`, `--path`, and `--force`; listeners additionally require
`--event <TYPE>`, and datatables require `--model <TYPE>`. Generated policy
code denies by default, and generated files print their explicit follow-up
registration/customization step.

## Datatable export context, delivery, and memory behavior

Queued exports now snapshot the current locale and application timezone along
with the actor. The worker restores both before query callbacks, mappings,
translations, and XLSX cell generation, so emailed output matches the request
that queued it. `DatatableContext.locale` is now `Option<String>` rather than
`Option<&str>`; update borrowed uses to `ctx.locale.as_deref()`.

The silent `NoopExportDelivery` fallback was removed. Register exactly one
delivery service before running queued exports:

```rust
registrar.singleton(
    Box::new(AppDatatableExportDelivery) as Box<dyn DatatableExportDelivery>
)?;
```

Without it, `DatatableExportJob` fails explicitly and follows its normal retry
and dead-letter policy. Remove imports or construction of
`NoopExportDelivery`; tests that intentionally discard output should register
an explicit test delivery whose behavior is visible to the test.

XLSX generation now streams rows from PostgreSQL through a bounded 256-row
channel into `rust_xlsxwriter` constant-memory mode. `max_export_rows` keeps the
same contract and error; setting it to `0` no longer materializes all query rows
and worksheet cells at once.

Queued delivery now remains bounded through the completed artifact as well.
The worker writes the XLSX ZIP to a temporary file and calls the new provided
`DatatableExportDelivery::deliver_file` method. New delivery implementations
should stream or copy that path before returning:

```rust
#[async_trait]
impl DatatableExportDelivery for AppDatatableExportDelivery {
    async fn deliver_file(
        &self,
        export: GeneratedDatatableExportFile,
        recipient: &str,
    ) -> Result<()> {
        self.upload_and_notify(recipient, export.path(), export.filename())
            .await
    }
}
```

The file is removed when the delivery future completes, errors, or unwinds, so
do not retain its path for later work. HTTP downloads still hold their final
response bytes once; this change targets queued delivery.

Existing implementations of `deliver(GeneratedDatatableExport, ...)` compile
unchanged. The provided adapter checks file metadata before allocation and
buffers only artifacts up to the public
`LEGACY_DATATABLE_EXPORT_MAX_BYTES` limit (25 MiB). A larger queued export now
fails explicitly instead of allocating without a bound; override `deliver_file`
before upgrading if existing exports can exceed that size. No database
migration, runtime config change, or new dependency is required.

## Optional authoritative actor hydration

Foundry now has one guard-scoped `ActorHydrator` registry shared by token,
session, and custom bearer authentication across HTTP and WebSocket initial and
revalidation paths. Existing applications are unchanged when no hydrator is
registered. Opt in when stored credentials must immediately reflect current
roles, permissions, account deletion, or disabled state:

```rust
#[async_trait]
impl ActorHydrator for CurrentUserHydrator {
    async fn hydrate(&self, credential: &Actor, app: &AppContext) -> Result<Option<Actor>> {
        load_current_actor(app, credential).await
    }
}

registrar.register_actor_hydrator(API_GUARD, CurrentUserHydrator)?;
```

Register at most one hydrator per `GuardId` through `ServiceRegistrar` or
`PluginRegistrar`; duplicate ownership fails bootstrap. A returned actor must
keep the exact credential ID and guard. Return `None` for deleted/disabled
accounts. Errors, panics, identity drift, and `None` all reject authentication.

Full-scope tokens use current hydrated permissions. Explicit token abilities
are intersected with current permissions, so hydration cannot expand their
scope. MFA-pending credentials deliberately skip hydration until challenge
completion and retain only their restricted MFA abilities.

## Per-channel notification retries and database notification repository

New queued notification dispatch creates one `SendNotificationJob` per selected
channel. Successful email/database/broadcast/custom channels are no longer
replayed when a different channel fails. Queue history, metrics, retry, and
dead-letter volume therefore changes from one job per notification to one job
per channel. Enqueue continues across channels and returns an aggregate error,
so callers must treat an error as potentially partially enqueued.

`build_notification_job` and the existing job ID/wire shape remain as aggregate
compatibility. New workers read old aggregate payloads, and old workers can read
the additive single-channel payload. Deploy custom channel adapters everywhere
before producers select them. Keep `Notifiable::notifiable_type()` at its
default while old workers remain, then drain/upgrade them before enabling a
custom type.

Database notifications now persist and scope on both `notifiable_type` and
`notifiable_id`. Publish and apply the schema migration before deploying the
new runtime:

```bash
cargo run -- migrate:publish
cargo run -- db:migrate
```

Migration `000000000014_add_notification_notifiable_type` assigns existing rows
the `default` type and replaces indexes with deterministic typed and partial
unread indexes. If adopting a custom type, backfill existing rows before
switching the implementation or those default-scoped rows will not appear.

`DatabaseNotificationRepository` is new first-class support for typed,
ownership-scoped list/paginate/unread/read/count/mark-read/mark-all-read/delete
operations, including `*_with` transaction/executor variants. Use
`for_actor_as` when the database morph type intentionally differs from the
actor's guard.

## Non-HTTP/manual audit entries and retention

Jobs, scheduler tasks, CLI commands, and domain services can now opt model
lifecycle writes into the built-in audit trail with
`scope_audit(AuditContext, future)`. The context carries an area plus optional
actor, request ID, IP, and user agent; HTTP route-area behavior remains
unchanged and unscoped non-HTTP writes remain unaudited by default.

`AuditManager::record` adds explicit domain audit entries through any
`QueryExecutor`, including an application transaction. Manual before/after/
changes JSON receives the same recursive sensitive-field redaction as model
lifecycle payloads. The manager is available through public
`AppContext::audit()`.

`AuditConfig` gained `retention_days`; direct struct literals must add it or use
`..AuditConfig::default()`. Its default is `0`, which keeps all history. No rows
are removed merely by upgrading. Configure a positive window and schedule
`AuditManager::prune_retention`, call `prune_before` with an explicit cutoff, or
run:

```bash
cargo run -- audit:prune
cargo run -- audit:prune --days 90
```

The command refuses an effective zero-day policy so an operator cannot mistake
"retention disabled" for a successful cleanup.

## Storage URLs, streaming, and S3 credential providers

`StoredFile.url` is now truthful. A public local/S3 write populates it when the
adapter has a stable URL source; a private write always returns `None`.
`StorageDisk::url()` now rejects private disks instead of manufacturing a
public URL. Use `temporary_url()` for private S3 delivery.

`StorageAdapter`, `StorageDisk`, and `StorageManager` gained `put_stream` and
`get_stream`. Existing custom adapters compile unchanged because the trait
defaults buffer through `put_bytes`/`get`; override both methods for native
bounded behavior. Built-in local storage uses atomic streaming writes and
bounded reads. S3 uploads stream through bounded multipart parts, abort failed
uploads, and return provider download chunks without collecting the object.

S3 static credentials are optional. Omitting both `key` and `secret` uses the
AWS environment/workload/instance provider chain. Explicit credentials still
take precedence and may include `session_token`; configuring only one half of
the key/secret pair, or a token without the pair, is rejected.

`ResolvedS3Config.key` and `.secret` changed to `Option<String>` and the struct
gained `session_token: Option<String>`. Update direct literals accordingly.
TOML using the old explicit pair remains compatible.

## Batched metadata loading and explicit orphan maintenance

Model metadata now participates in Foundry's model-extension cache. Use
`with_meta("key")` or `with_metadata()` on `ModelQuery`, `RelationDef`, or
`ManyToManyDef` when rendering collections. Existing `get_meta`, `has_meta`,
and `all_meta` calls remain source-compatible; inside an HTTP request (or an
explicit `app.with_model_batching(...)` scope), a lazy read also batches known
sibling model IDs. Metadata writes invalidate the affected cache entry.

`HasMetadata::delete_all_meta` removes every key for one model.
`MetadataOwner::for_model::<M>()`, `audit_metadata_orphans`, and
`prune_metadata_orphans` provide deliberate maintenance for polymorphic rows
whose owner was deleted. Foundry does not infer owner tables or run destructive
cleanup automatically; schedule one explicit owner declaration per model type
where pruning is wanted. The published metadata table stores UUID owner IDs,
so `for_model` rejects non-UUID primary keys. No schema migration is required.

## Concurrent attachment cardinality, ordering, and complete orphan scans

`.single()` collection uploads and `replace_attachment` now serialize on a
PostgreSQL advisory transaction lock for `(attachable type, owner ID,
collection)`. The new row and removal of replaced rows commit atomically, so
concurrent requests leave exactly one record. Multi-file inserts now append
increasing `sort_order` values instead of writing zero for every row.

`HasAttachments::reorder_attachments(app, collection, ordered_ids)` atomically
replaces a collection's order. The ID slice must be an exact permutation of
the current collection; duplicate, missing, or foreign IDs fail without
changing positions. Existing rows need no migration—the next reorder
normalizes their positions, and new uploads append after the current maximum.

`AttachmentSpecHook::after_store` now runs while the atomic collection
transaction is active, after insertion but before commit. Use the supplied
`ctx.attachment`; queries made through another pooled connection cannot rely on
seeing that uncommitted row. A hook failure rolls back the replacement and
removes its prepared file. Dispatch dependent work only after the attachment
operation returns successfully.

Orphan maintenance now scans a prefix page by page to exhaustion. The storage
batch setting and `attachment:orphans --limit` are page sizes rather than total
scan caps. Built-in local and S3 disks support exclusive path cursors. Custom
adapters remain source-compatible, but must override
`StorageAdapter::list_prefix_after` as well as `list_prefix` for complete
multi-page orphan scans; otherwise Foundry reports the scan as incomplete. No
database migration is required.

## Contract manifest v2 and explicit business actions

Every named HTTP route included in contract export must now declare an explicit
business action name. Route IDs remain URL/transport metadata and no longer
derive SDK names:

```rust
scope.get("/{id}", "show", show_user, |route| {
    route
        .action_name("GetUser")
        .path_parameter::<ModelId<User>>("id")
        .query_parameter::<bool>("include_roles", false)
        .response::<UserResponse>(200)
        .error::<ApiError>(404, "user_not_found");
});
```

For `HttpResourceRoutes`, use `index_with_action`, `store_with_action`,
`show_with_action`, `update_with_action`, and `destroy_with_action` (or attach
equivalent documentation through route options). Export fails for a missing or
duplicate action name, including named routes with client export disabled.

`FoundryContractManifest.json` changes from version 1 to 2. Its HTTP action
shape replaces raw path-name strings with typed `parameters` carrying location,
schema, and requiredness; adds per-action `errors`; and removes
`ContractHttpTransport.path_params`. Realtime channels add
`message_payload`. Update any custom manifest readers and public struct
literals. Regenerate frontend artifacts after every route is annotated:

```bash
cargo run -- types:export
```

Generated SDK filenames and client method names now follow the explicit action.
SDK options separate `params`, `query`, `headers`, and `cookies`, and a required
parameter group makes the options argument required. Update imports/call sites
for any action name changed from the previous route-derived spelling.

### OpenAPI metadata, custom media types, and form adapters

OpenAPI now uses each explicit business action name as `operationId`. Guarded
actions declare the `bearerAuth` security scheme and operation security while
retaining Foundry guard/permission extensions. Typed path, query, header, and
cookie parameters retain their schema references; successful responses use
canonical HTTP status descriptions; standard and action-specific errors are
grouped by status without overwriting one another.

Request documentation can opt into a custom media type after declaring its
schema:

```rust
route
    .request::<CreateUserRequest>()
    .request_content_type("application/x-www-form-urlencoded");
```

This changes contract/OpenAPI metadata only. The route handler must use an
extractor compatible with the declared media type, and generated SDK code does
not automatically form-encode the request. Declaring a content type without a
request schema is rejected during route-manifest collection.

`types:export` now generates the contract manifest and pure SDK by default. It
no longer writes the compatibility form-oriented `routes/*.ts` adapters. If an
existing frontend still imports those adapters, regenerate them explicitly:

```bash
cargo run -- types:export --route-form-adapter
```

Programmatic exporters use the equivalent opt-in:

```rust
TypeScriptExportContext {
    route_form_adapter: true,
    ..Default::default()
}
```

The route manifest, SDK, realtime descriptors, and optional form adapters all
consume the same frozen `ContractManifest`; no generator reads live HTTP route
metadata after that boundary.

Direct Rust struct literals must add the new public fields:

- `TypeScriptExportContext::route_form_adapter`
- `DocumentedRoute::auth`
- `RouteManifestEntry::request_content_type`
- `ContractHttpTransport::content_type`

WebSocket handlers can now use
`typed_channel::<Payload, _>`/`typed_channel_with_options` to deserialize before
invocation and export the message schema. Invalid payloads receive 422. Use
`raw_channel` for intentional dynamic JSON; existing `channel` remains a
source-compatible raw alias.

## Conditional and request-presence validation

The derive and manual validator now share first-class `required_if`,
`required_unless`, `required_with`, `present`, `sometimes`, `prohibited`,
`boolean`, and collection `distinct` rules. Existing rules and request DTOs are
unchanged. New derive forms include:

```rust
#[validate(required_if("status", "published", "scheduled"))]
published_at: Option<DateTime>,

#[validate(sometimes, boolean)]
notify: Option<bool>,

#[validate(distinct, each(min_length(2)))]
tags: Vec<String>,
```

Cross-field arguments use Rust field names; serde wire names still define error
keys and generated client metadata. Built-in JSON validation now retains raw
top-level keys, and generated multipart extraction retains part names. Thus
`present` accepts explicit null/empty input but rejects an absent field, while
`sometimes` skips only absent fields. Manual validators can use
`optional_field` or `field_with_presence` for the same distinction.

`distinct` is case-sensitive and exact. OpenAPI emits `uniqueItems: true` and
marks `present` fields required; conditional semantics remain in Foundry's
validation metadata/TypeScript runtime because ordinary per-property OpenAPI
constraints cannot express the cross-field rule accurately. No migration or
existing call-site change is required.

## Redis reconnect timeouts and low-level command escape hatch

`RedisConfig` gained `connect_timeout_ms` and `command_timeout_ms`, both defaulting
to 5000. Existing TOML/environment configuration requires no change. Direct
Rust struct literals must add both fields or use `..RedisConfig::default()`:

```toml
[redis]
connect_timeout_ms = 5000
command_timeout_ms = 5000
```

Redis cache, jobs, scheduler, locks, public commands, and pub/sub now share a
reconnecting provider. A connection or timeout failure invalidates that cached
generation; the next operation reconnects. Foundry never automatically retries
the failed command because Redis may already have applied a mutation before its
response was lost. Callers may retry only when their domain operation is known
to be idempotent.

`RedisManager::command` adds a namespace-safe low-level escape hatch for sorted
sets, lists, streams, and provider commands. The typestate builder accepts
ordinary prefix arguments but cannot execute until `.key(&RedisKey)` supplies a
typed namespaced key. Additional keys remain typed. `pipeline`, `transaction`,
and `script` provide batched, atomic, and Lua paths using the same key contract.
Existing convenience operations are unchanged.
