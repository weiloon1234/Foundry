# Changelog

All notable changes to this project will be documented in this file.

The format is inspired by Keep a Changelog, adapted for Foundry's pre-`1.0` releases.

## [Unreleased]

### Security

- Typed routes registered inside `group_with_options` now inherit the group's guard, permissions, dynamic authorizer, middleware group, rate limit, audit area, tags, and documentation defaults. Explicit `.public()` remains the opt-out. Unknown middleware groups now fail router construction, and duplicate group IDs fail bootstrap instead of silently replacing or bypassing policy.
- MFA-pending access tokens can no longer subscribe to protected WebSocket channels, and MFA-pending refresh credentials can no longer renew the challenge beyond its bounded TTL. Confirmed TOTP factors cannot be replaced with an ordinary authenticated credential; factor replacement remains confined to the pending MFA exchange flow.
- WebSocket action failures no longer return internal handler or panic details to clients, and notification broadcasts now have `register_notification_websocket_channel`, a guarded server-only channel helper that restricts subscriptions to the authenticated actor's own room.
- Presence handlers now enumerate/count only members in their current channel-and-room subscription, preventing actor IDs from other rooms on the same presence channel from leaking through `presence_members` or `presence_count`. Guarded observability retains its intentional channel-wide administrative view.
- Audit JSON redaction now recursively removes credential-shaped keys inside nested objects and arrays, and `CryptConfig` redacts its encryption key from `Debug` output.
- Dependency locks now use patched `crossbeam-epoch`, `ammonia`, `anyhow`, `lettre`, `rand`, and `rustls-webpki` releases; consumer fixture locks were refreshed to current compatible releases, and `validator_derive` moved off the unmaintained `proc-macro-error2` chain. The upstream-constrained `object_store`/`quick-xml` advisory remains documented in the framework audit.
- Framework-generated outputs now reject symlinked path components beneath the selected output root before creating directories, writing files, or removing manifest-owned files, preventing TypeScript, API-doc, scaffold, config, and plugin generators from escaping through nested parent symlinks. A caller-selected output root may still itself be a symlink.
- Guarded WebSocket connections now revalidate cached bearer/session credentials on a bounded interval without extending sliding sessions, refresh actor abilities, and close revoked or unauthorized sockets before protected actions or broadcasts. Production same-origin checks now compare scheme, host, and effective port and honor forwarded origin metadata only from trusted proxies. New global and anonymous per-IP admission caps reserve capacity before upgrade, while authenticated limits are scoped by `(guard, actor ID)`.
- Auth now rejects actors whose stored guard does not match the requested guard instead of retagging them, preventing a user token/session from satisfying guard-only admin routes or WebSocket channels.
- `enable_observability()` now protects `/_foundry/*` with the app's default auth guard by default; public diagnostics require the explicit `enable_public_observability()` / `ObservabilityOptions::public()` opt-in, and production-like public registration logs a warning.
- TOTP MFA now uses HMAC-SHA1 per RFC 6238 so codes verify against mainstream authenticator apps (Google Authenticator, Authy, 1Password, etc.), which ignore the otpauth `algorithm` parameter and always compute SHA1. **TOTP factors enrolled on previous builds generate different codes and must be re-enrolled.**
- `min_numeric`, `max_numeric`, and `between` validation rules now reject values that fail to parse as a number, as well as `NaN` and infinities; previously non-numeric input (including `"NaN"`, which defeats all numeric comparisons) silently passed bound checks. The `numeric` rule is now parse-based: malformed shapes like `"1.2.3"` are rejected, and scientific notation such as `"1e10"` is accepted.
- The Postgres compiler now validates `EXTRACT` field names against the known date/time fields and restricts `Sql::op` custom operators to legal operator characters, closing a raw-SQL interpolation path if either was ever fed untrusted input.
- Password-reset and email-verification tokens now enforce their TTL inside the consuming `DELETE`, so presenting an expired token no longer destroys it; expired and unknown tokens return the same "invalid or expired" message.
- `StaticBearerAuthenticator` stores and compares SHA-256 digests of its tokens instead of plaintext map keys, removing token-comparison timing as a side channel.
- Secret-bearing config structs (`AppConfig.signing_key`, SMTP/Resend/Postmark/Mailgun/SES credentials, S3 secret) redact their secrets in `Debug` output, and `DatabaseConfig`/`RedisConfig` redact URL credentials, so `{:?}` logging can no longer leak credentials.

### Fixed

- HTTP route registration callbacks now execute exactly once during bootstrap. Named-route lookup, runtime routing, OpenAPI/contract metadata, and TypeScript export share one frozen route plan across every kernel profile.
- Database migration DDL and ledger writes now use the same pinned transaction/session, and failed queries no longer leave a session-level statement timeout behind. Repeated insert/update/model assignments use deterministic last-write-wins replacement instead of emitting duplicate SQL assignments.
- `#[derive(Validate)]` now keeps Rust, multipart, runtime validation, and TypeScript metadata aligned for required `Option` fields, typed numeric/boolean fields, collection `each` rules, and collection count bounds; invalid `each` placement receives a compile-time diagnostic.
- OpenAPI 3.1 output now represents nullable values as JSON Schema unions, treats `serde_json::Value` as unconstrained JSON, always marks path parameters required, and gives structurally distinct schemas unique identities. Contract route schemas are materialized once and incompatible duplicate registrations fail clearly.
- `#[derive(AppEnum)]` now rejects empty keys/aliases and detects collisions across key and alias namespaces. Model scaffolding no longer generates a duplicate `Clone` derive.
- Nullable `belongs_to` relations preserve their owner-key type, `Collection::chunk` works for non-`Clone` values, and `ContractError` is available from the public prelude.
- Malformed i18n configuration now fails bootstrap instead of silently disabling localization. CLI `--help`/`--version` are successful outcomes, and HTTP/WebSocket listeners accept IPv6 host values.
- `Setting::get_as` now reports stored type drift instead of returning `None`, and `SettingType::Password` is documented as presentation metadata rather than encryption or secret storage.
- `TestAppBuilder` and `TestRequestBuilder` are now nameable from `testing` and the prelude. `TestApp::from_builder` reuses a production `AppBuilder`, while `TestApp::shutdown` gracefully stops managed tasks and plugin hooks.
- `types:export` now rejects exact and ASCII case-only output collisions between DTOs, AppEnums, framework runtimes, route helpers, SDK actions, and the generated barrel before cleanup or writes, with an error naming both owners instead of silently overwriting one module.
- Immediate notifications now attempt all selected channels and return delivery or missing-registration failures instead of logging and reporting success. Queued notification jobs propagate email, database, broadcast, and custom-channel failures into worker retry/dead-letter handling, preserve custom routing values across serialization, and both `build_notification_job` and `notify_after_commit` now return pre-render failures instead of producing or registering a no-op job.
- Pinned the transitive `axum-extra`/`cookie` time parser dependency to `time 0.3.47`, preventing downstream deploy builds from resolving to newer `time 0.3.x` releases that break `cookie 0.18.1` in some toolchains.
- A worker whose job-lease heartbeat fails (lease expired, claimed elsewhere, or backend unreachable past the lease TTL) now cancels the running job instead of letting it race the redelivered copy on another worker. Transient renewal errors are retried while the last successful renewal still covers the lease TTL.
- Normal scheduler ticks now interpret cron expressions in the configured app timezone instead of UTC. Explicit `tick_at` and `run_once_at` calls retain UTC semantics, and ambiguous or nonexistent IANA daylight-saving wall times are skipped consistently with `LocalDateTime::in_timezone`.
- Scheduler `without_overlapping` now uses the shared owner-token lock, renews long-running leases, cancels protected work when ownership or renewal is lost, and fails closed on acquisition errors. Shutdown awaits compare-and-delete release, stale owners cannot delete successor locks, the deployed `schedule:<id>` key remains compatible, and `tick_at`/`run_once_at` no longer report overlap-skipped schedules as started. `ScheduleOptions::without_overlapping_for(Duration)` configures the lease duration while preserving the one-hour default.
- Plugins that booted successfully are now shut down (in reverse order) when a later plugin's `boot()` fails, so resources acquired during partial bootstrap no longer leak.
- The in-memory scheduler leadership backend no longer renews an already-expired lease; like the Redis backend, the previous holder must win a fresh election, preventing split-brain in single-process and test setups.
- The WebSocket pub/sub task now resubscribes with exponential backoff when its backend subscription ends or fails, instead of going permanently silent while publishes keep succeeding; dropping the server also aborts the task instead of leaking it.
- Scheduler leadership state uses acquire/release atomic ordering, and dropping the scheduler or a schedule overlap-lock guard outside a Tokio runtime now logs a warning instead of silently leaving the lock to lapse via TTL.
- Image dimension validation streams the upload from disk on the blocking thread pool instead of reading the entire file into memory on the async runtime.
- S3 `put_bytes` and `put_file` now persist a supplied content type as signed `Content-Type` object metadata. Disk visibility remains policy-based instead of emitting `x-amz-acl`, preserving compatibility with ACL-disabled AWS buckets and S3-compatible providers such as Cloudflare R2.
- Bodyless contract actions that still expose request metadata (for example path/query params on `GET`) no longer emit an OpenAPI `requestBody`, and generated SDK actions no longer require a payload argument that would be dropped at runtime.
- Generated `FoundryEndpoint.applyServerErrors` now safely ignores malformed or non-object error payloads before reading `.errors`, so unusual transport/client errors no longer mask the original failure with a runtime crash.
- Multipart `Validated<T>` extraction now removes framework-owned uploaded temp files when validation fails before the handler receives the DTO.
- `UploadedFile` now provides consuming `store*_and_cleanup` methods that remove framework-owned upload temp files after both successful and failed storage attempts while retaining the original borrowed `store*` methods for reusable attachment and image-processing workflows.
- `#[derive(Validate)]` now generates multipart cleanup hooks for `UploadedFile`, `Option<UploadedFile>`, and `Vec<UploadedFile>` fields, and rejects unsupported nested upload wrappers with a clear derive error.
- Datatable numeric filters now reject out-of-range `Int16`/`Int32` number values instead of wrapping them during downcast.
- WebSocket `on_leave` hooks and presence leave cleanup now run only when an `unsubscribe` actually removed an active subscription, so forged/unmatched unsubscribe frames cannot trigger leave-side effects.

### Added

- Serverless database pool tuning: `DatabaseConfig` now supports `connect_lazy`, optional
  `[database.write_pool]` and `[database.read_pool]` override sections, and resolved
  `write_pool_config()` / `read_pool_config()` helpers. This lets deployments keep the legacy
  flat pool defaults while independently capping primary and read-replica pools for serverless
  Postgres or provider poolers.
- Database diagnostics gained `DatabaseManager::has_read_pool()`, `ping_write()`, and
  `ping_read()` for targeted primary/read-replica readiness checks.
- Redis now enables Tokio rustls support so `rediss://` TLS Redis endpoints work for serverless
  Redis providers.
- Contract-first frontend generation foundation: `types:export` now emits `FoundryContractManifest.json`, `FoundryErrors.ts`, `FoundrySdk.ts`, `FoundryClient.ts`, and per-action `sdk/*.ts` modules. The manifest normalizes route actions, transport body kind, permissions, request/response DTO JSON schemas, validation schemas, standard errors, and realtime channel contracts; new frontend code can use `createFoundryClient(...)` business actions instead of endpoint helpers.
- Consumer update path for generated frontend clients: after upgrading, run `types:export`, import `createFoundryClient` from the configured generated TypeScript barrel, pass the existing Axios-compatible transport, and call generated business actions such as `api.userPortalLogin(payload, { params })`. Catch `FoundrySdkError` for normalized validation and HTTP failures. Existing `FoundryEndpoint` route helpers remain available for form-state screens, while `without_client_export()` / `client_export(false)` now opts a route out of both helper and SDK action output. File/file-list request fields automatically generate multipart SDK actions.
- TypeScript route endpoint helpers: `types:export` now emits a headless `FoundryEndpoint` base runtime plus per-route helper files with path/method constants, request/response aliases, typed `validateForm()`/`submitForm()`, busy/error state, and validation metadata from `#[derive(Validate)]`. Generated DTO files now include a Foundry do-not-edit header, and validated DTO fields include validation-rule comments. Routes can opt out with `without_client_export()` or `client_export(false)`.
- Datatable relation filters: model datatables can opt in to typed relation-backed filters with `Datatable::relation_filters()`, `DatatableRelationFilter`, and `DatatableRelationColumn`.
- Datatable relation filter coverage for belongs-to, has-many, many-to-many, legacy hyphen aliases, and `LikeAny` search across declared relation columns.
- Consumer-facing datatable request examples for direct filters, relation filters, legacy query params, and multi-column relation search.
- Generated TypeScript files and `foundry-build` generated Rust registries now start with an explicit `DO NOT EDIT` warning that names the generator and says the file will be overwritten.
- Release infrastructure: GitHub Actions CI, release-readiness workflow, release checklist, and local package dry-run verification.
- Consumer documentation: root README, contributing guide, and a first-class plugin example.
- WebSocket observability dashboard endpoints: `GET /_foundry/ws/channels`, `GET /_foundry/ws/presence/:channel`, `GET /_foundry/ws/history/:channel`, and `GET /_foundry/ws/stats`. History payloads are redacted by default; set `observability.websocket.include_payloads = true` to include them.
- Per-channel WebSocket Prometheus series on `/_foundry/metrics` (`foundry_websocket_subscriptions_total{channel=...}`, `foundry_websocket_active_subscriptions{channel=...}`, `foundry_websocket_channel_messages_total{channel=...,direction=...}`).
- HTTP request latency histograms on `/_foundry/runtime` and `/_foundry/metrics` via `foundry_http_request_duration_ms_bucket`, `_sum`, and `_count`, which can be used to compute p50/p95/p99 in Prometheus-compatible backends.
- `AppContext::websocket_channels()` accessor returning the registered channel registry.
- `WebSocketChannelDescriptor` and `WebSocketChannelRegistry` public types exposing registered WebSocket channels.
- `WebSocketChannelOptions::incoming_event`, `incoming_event_without_payload`, `outgoing_event`, and `outgoing_event_without_payload` for declaring typed realtime contracts alongside channel registration.
- Configurable TTL on WebSocket replay history (`websocket.history_ttl_seconds`, default 7 days). Every publish refreshes the TTL on `ws:history:<channel>`, so active channels never expire; channels idle past the window are auto-reaped by Redis. Set to `0` to disable.
- WebSocket hardening config: `websocket.outbound_buffer_size`, `websocket.allowed_origins`, and `websocket.history_buffer_size`.
- WebSocket protocol/lifecycle acceptance coverage for raw JSON actions, subscription enforcement, room routing, client events, ack success/error, socket-close cleanup, and force-disconnect cleanup.
- `doctor --strict` for production readiness gates; warnings now fail the command in strict mode and text output ends with a readiness verdict.
- `make:model`, `make:job`, and `make:command` now accept `--path <DIR>` and are registered even when a project has not published database config yet.
- Public API contract and recipe documentation covering production readiness, authenticated CRUD, queued email, uploads, datatables, and plugin extension.
- Public API acceptance coverage for the blessed consumer import surface.

### Changed

- Model `find`/`find_many` APIs now require the model's declared `TypedPrimaryKey`; `set_null` is available only for nullable columns, and text predicates are available only for string columns. These compile-time constraints prevent cross-model IDs and invalid SQL/type combinations.
- Middleware group registration and route references now use the semantic `MiddlewareGroupId` type. Define shared constants with `MiddlewareGroupId::new(...)`; use `MiddlewareGroupId::owned(...)` for genuinely dynamic IDs. Raw strings are no longer accepted by `AppBuilder`, `HttpRouteOptions`, `HttpScope`, `HttpRouteBuilder`, or `MiddlewareGroups` APIs.
- `DatabaseManager::ping()` now checks both the primary pool and the configured read-replica pool.
  `doctor` reports whether it checked the primary only or both primary and replica. Use
  `ping_write()` for primary-only health checks.
- Redis-backed runtime commands for locks, rate limits, jobs, scheduler leadership, and WebSocket
  publishes now reuse a cached multiplexed Redis connection instead of opening a new command
  connection for each operation.
- Config env overrides now support an explicit `FOUNDRY__` namespace (e.g. `FOUNDRY__SERVER__PORT`), which is stripped before applying and wins over the unprefixed form. Unprefixed `__`-delimited variables keep working; the prefix avoids collisions with ambient process variables.
- `TemplateRenderer` gained `render_async`, which renders on Tokio's blocking thread pool; `EmailMessage::template(...)` uses it internally.
- `Job::max_retries` and `jobs.max_retries` are now documented as the maximum number of *total attempts* (like Laravel's `tries`): `1` dead-letters on the first failure with no retry.
- `EventManager::dispatch` is now documented as sequential with stop-on-first-error semantics, mirroring Laravel's synchronous listeners.
- Database scaffold command helpers moved out of the migration lifecycle module into a dedicated scaffold module.
- Datatable blueprint/status documentation now reflects implemented JSON, filter, sort, download, export, registry, legacy query-param, and relation-filter acceptance coverage.
- Consumer starter documentation now recommends the split bootstrap layout proven by the blueprint app fixture.
- The README architecture diagram was replaced with an ASCII-safe AppBuilder/AppContext/kernel flow.
- `foundry-build` migration filename tests now assert the current parser diagnostics for invalid timestamps and malformed `YYYYMMDDHHMM_slug.rs` filenames.
- Crate metadata is now publish-ready for the `0.1.x` line.
- Verification contract now explicitly includes both fixture families and packaging checks.
- `MaxBodySize` now also updates Axum's default extractor body limit, so JSON/Form/String extractors honor the configured Foundry limit instead of staying capped at Axum's 2 MiB default.
- Framework model post-write events (`ModelCreatedEvent`, `ModelUpdatedEvent`, and `ModelDeletedEvent`) now dispatch after the active transaction commits, making event listeners safe for dependent writes and queued onboarding jobs that need the committed row to be visible.
- `WebSocketRuntimeSnapshot` now includes a `channels: Vec<WebSocketChannelSnapshot>` field in addition to the existing global counters.
- `WebSocketKernel::new` no longer takes a `Vec<WebSocketRouteRegistrar>`; registered channels are built once during `AppBuilder::bootstrap()` and resolved from the DI container. Direct callers of `WebSocketKernel::new` must drop the routes argument.
- `RuntimeDiagnostics` inbound-message recording at the kernel now runs after `serde_json::from_str` parses the client message (so only parseable messages are counted). Malformed frames no longer increment `inbound_messages_total`.
- WebSocket wire actions are documented as canonical `snake_case`; legacy PascalCase action aliases remain accepted for compatibility.
- WebSocket room routing is now explicit: channel-wide publishes reach all subscribers, while room publishes reach only exact room subscribers.
- WebSocket `on_leave` hooks and `presence:leave` now run for unsubscribe, socket close, heartbeat timeout, and force disconnect.
- WebSocket channel callbacks now receive owned context/channel/room values, which makes async closures easier to use safely.

### Breaking

- Middleware groups no longer accept raw `&str`/`String` IDs. Define a `MiddlewareGroupId` constant (or explicitly construct an owned ID for dynamic configuration) and reuse it for registration and route references.
- Model `find`/`find_many` reject IDs that do not match `Model::PrimaryKey`; `set_null` rejects non-nullable columns, and string-only predicates reject non-string columns at compile time.
- `HttpRegistrar::into_router` and `into_router_with_middlewares` now return `Result<Router>` so unknown middleware-group references cannot be ignored.
- `build_notification_job` and `AppTransaction::notify_after_commit` now return `Result`; callers must propagate or handle renderer/routing failures.
- Required `Option<T>` validation fields are no longer implicitly nullable, typed non-string numeric/boolean values validate with their actual value kinds, and collection `min`/`max` rules apply to item count.
- `Setting::get_as` returns an error when the stored setting type differs from the requested type instead of treating the row as absent.
- `DatabaseConfig` gained public fields: `connect_lazy`, `write_pool`, and `read_pool`. Consumers
  that construct `DatabaseConfig` with a Rust struct literal must add those fields or use
  `..DatabaseConfig::default()`.
- `DatabaseManager::ping()` now fails when `database.read_url` is configured but the read replica
  cannot be reached. Consumer readiness checks that intentionally only validate the primary should
  call `ping_write()` instead.
- `EmailMessage::template(...)` is now `async` (template files are read off the async runtime); add `.await` before the `?`.
- TOTP MFA switched from HMAC-SHA256 to RFC 6238 HMAC-SHA1 (see Security); previously enrolled TOTP factors must be re-enrolled.
- Validation: `min_numeric`, `max_numeric`, `between`, and `numeric` are stricter (see Security); requests that previously passed with non-numeric values in bounded fields now fail validation.
- WebSocket `message` and `client_event` frames now require an active matching channel/room subscription before handlers or client-event relay run.
