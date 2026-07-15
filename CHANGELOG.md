# Changelog

All notable changes to this project will be documented in this file.

The format is inspired by Keep a Changelog, adapted for Foundry's pre-`1.0` releases.

## [Unreleased]

### Security

- Optional guard-scoped `ActorHydrator` registration refreshes authoritative roles, permissions, and account eligibility on token, session, custom bearer, HTTP, and WebSocket authentication. Returned identity drift and deleted/disabled (`None`) actors reject the credential; scoped token abilities are intersected with current permissions and cannot be widened.
- Inbound request IDs are accepted only when they are non-empty visible ASCII and at most 128 bytes; invalid values are replaced with UUIDv7 IDs before entering tracing or response headers, preventing unbounded or control-character correlation data from reaching logs.
- Untrusted image decoding is bounded by default by input size, dimensions, pixel count, and decoder allocation limits. Explicit unbounded constructors remain available only for trusted-input call sites.
- Actor rate-limit keys now include both guard and actor ID, preventing principals with equal IDs under different guards from sharing a throttle bucket. `ConfigRepository` debug output is opaque so arbitrary nested configuration cannot bypass typed secret redaction.
- HTML email templates now escape ordinary `{{variable}}` substitutions, with explicit `{{{variable}}}` syntax for trusted markup. SMTP `encryption = "none"` now creates a truly plaintext connection instead of accidentally initiating STARTTLS.
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

- Manual validation now resolves dot/bracket composite field names against their top-level request root without discarding explicit nested absence, so `required`, `sometimes`, and other presence-gated rules behave correctly for fields such as `name.en` and `rows[0].open_time`.
- CI and release-readiness now export the `FOUNDRY_TEST_POSTGRES_URL` variable consumed by every PostgreSQL acceptance target; the stale `FORGE_TEST_POSTGRES_URL` name previously caused those test bodies to skip silently.
- Redis command paths now use reconnect-invalidating shared connections with bounded connect/command/pub-sub timeouts. Failed operations are not replayed, while the next operation reconnects instead of cloning an unusable cached transport forever.
- Attachment orphan maintenance now paginates every local/S3 prefix to exhaustion instead of repeatedly scanning only the first configured batch. `.single()`/replacement writes are serialized and atomic per owner collection, preventing concurrent uploads from leaving multiple live rows.
- Public local/S3 writes now populate `StoredFile.url` when a stable URL exists; private disks clear the field and reject `url()` instead of exposing a misleading public-address contract.
- Queued notifications now fan out to one job per selected channel, so successful channels are not replayed when another channel retries. Enqueue attempts continue across channels and aggregate failures; after-commit notification dispatch registers the same independent jobs.
- Queued datatable exports now preserve dispatch-time locale and timezone through worker query/mapping/XLSX generation, require an explicit delivery, and write the completed ZIP to a temporary file instead of collecting the HTTP body with an unbounded byte limit. File artifacts are removed after delivery on success, error, and panic unwinding.
- Logging bootstrap now returns tracing-subscriber, file-writer, and OpenTelemetry exporter setup failures instead of silently running without the requested observability pipeline. Embedded hosts can explicitly retain subscriber ownership with `use_external_tracing_subscriber()`.
- `[email].queue` now controls immediate and delayed queued-mail dispatch and is included in worker polling; `[email].template_path` is now consumed by `EmailManager::render_template` instead of being inert.
- Cursor pagination is deterministic across duplicate and nullable sort values through a primary-key tiebreaker, correct forward/backward predicates, N+1 boundary detection, and automatic previous/next cursors. Supplying both directions is rejected before query execution.
- Model translations now support manual text and integer-shaped model keys as well as UUIDs. The polymorphic ID column/bindings use text, eager loading retains batching, and typed translation joins cast owner keys at the boundary.
- Built-in country seeds now populate timezone arrays from IANA tzdb 2026a instead of persisting empty arrays for every country. `BV` and `HM` remain intentionally empty because IANA assigns neither a zone; `XK` explicitly uses `Europe/Belgrade`.
- Serde field and enum renames now remain consistent across JSON, validation errors, multipart extraction, OpenAPI schemas, and generated validation metadata. DTOs with different serialize/deserialize names are rejected because Foundry exposes one wire contract.
- Bootstrap ignores only a missing optional `.env`; malformed or unreadable dotenv files now fail clearly. Login lockout now occurs on the configured failure count instead of one attempt early.
- `Setting::set` now errors for missing keys, setting hydration rejects unknown stored types, and country hydration rejects invalid status/collection data instead of silently coercing it.
- Custom validation rules preserve their returned message when no higher-priority override exists. Locale lookup is case-insensitive, catalog loading is deterministic, duplicate locale keys fail clearly, and model-translation fallback is lexicographically stable.
- Redis cache `flush()` is now namespace-safe and O(1) through generation keys instead of returning unsupported or deleting unrelated Redis data.
- Scheduler overlap-lock coordination errors are isolated per schedule: later due work is still evaluated and the failed occurrence remains eligible for retry. Job history now follows `jobs.track_history` independently from passive observability capture.
- Selected built-in notification channels now require their route/message/payload in immediate and queued flows instead of silently succeeding with missing output.
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
- A worker whose job-lease heartbeat fails (lease expired, claimed elsewhere, or backend unreachable past the lease TTL) now cancels the running job instead of letting it race the redelivered copy on another worker. Transient renewal errors are retried while the last successful renewal still covers the lease TTL, while intentional success/retry/dead-letter transitions are atomically distinguished from lease loss so their post-transition history and hooks cannot be cancelled by the heartbeat.
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

- File-backed queued datatable delivery through `GeneratedDatatableExportFile` and `DatatableExportDelivery::deliver_file`, with stream/copy access, RAII temp cleanup, and the named 25 MiB `LEGACY_DATATABLE_EXPORT_MAX_BYTES` compatibility bound.
- Component-level `make:request`, `make:dto`, `make:policy`, `make:event`, `make:listener`, `make:notification`, `make:mail`, `make:datatable`, `make:plugin`, and `make:test` commands for repeated application files. These are single-component generators, not a starter-project generator or installer.
- Namespace-safe low-level Redis command builders, pipelines, atomic transactions, and Lua scripts with typed key arguments, including prefix arguments for stream commands such as `XREAD`/`XGROUP`.
- Conditional/presence validation (`required_if`, `required_unless`, `required_with`, `present`, `sometimes`, `prohibited`), typed boolean validation, and distinct collection validation across derive, manual, JSON/multipart presence, OpenAPI, and generated TypeScript metadata/runtime paths.
- Explicit business action names, typed path/query/header/cookie parameters, per-action error schemas, and typed WebSocket message handlers now flow through contract manifest v2 into OpenAPI and generated TypeScript SDKs. Raw WebSocket JSON handling remains available through `raw_channel`.
- OpenAPI operations now emit contract action names as `operationId`, bearer security metadata for guarded actions, typed parameters, custom request media types, canonical response descriptions, and grouped standard/action error responses.
- Atomic `HasAttachments::reorder_attachments` with exact-membership validation, append-position assignment for new multi-file attachments, and cursor-aware `StorageAdapter`/disk/manager `list_prefix_after` APIs for complete maintenance scans.
- Batched model metadata loading through `ModelQuery`, `RelationDef`, and `ManyToManyDef` `with_meta`/`with_metadata` helpers, lazy sibling batching inside model-extension scopes, per-owner bulk deletion, and explicit `MetadataOwner` orphan audit/prune APIs.
- Storage `put_stream`/`get_stream` APIs with source-compatible buffered adapter defaults, bounded atomic local I/O, S3 multipart upload with abort-on-error, and streamed S3 response chunks. S3 configuration also supports optional AWS provider-chain credentials and explicit session tokens.
- First-class non-HTTP audit attribution via `AuditContext`/`scope_audit`, explicit redacted domain audit entries through `AuditManager::record`, public cutoff/configured-retention pruning, and the `audit:prune` command. Retention defaults to disabled (`0`) so upgrades never delete history automatically.
- Typed, ownership-scoped database notification reads and mutations through `DatabaseNotificationRepository`, including list/paginate, unread/read, unread count, mark one/all read, delete, transaction-aware variants, and explicit notifiable morph types.
- Public `ActorHydrator` plus provider/plugin registration, frozen once per `GuardId`, for applications that need every credential validation to restore current actor authorization from an authoritative store. Apps without a hydrator retain existing behavior.
- `make:model --table <TABLE>` for explicit model table naming, with conservative `-y`/sibilant pluralization for the default instead of mechanically appending `s`.
- Injectable CLI I/O through `CommandIo`, invocation output/prompt/confirmation/progress helpers, typed `CommandExit` statuses via `command_with_exit`, status-returning kernel runners, and a `CommandIoFake` for captured command tests. Existing `command` handlers and `Result<()>` runners remain compatible.
- A built-in CLI-only `dev` process orchestrator that launches selected `http`, `worker`, `scheduler`, and `websocket` runtimes from the current executable through the documented `PROCESS` selector. Child output is prefixed, shutdown cleans up every child, and opt-in restarts are count-bounded with capped exponential backoff. This is additive process orchestration with no configuration or migration change, not a starter-project generator or installer.
- Bounded background JSON file logging with configurable queue capacity, maximum record size, and shutdown flush deadline; `RuntimeSnapshot.logging` and Prometheus metrics expose accepted, written, dropped, rejected, oversized, write-error, pending, and timeout outcomes.
- Explicit `SecurityTier::{Relaxed, Strict}` independent from environment labels, plus config/doctor diagnostics for unknown framework keys, unknown prefixed overlays, and legacy unprefixed overlays derived from published metadata.
- Explicit queue variants on `JobDispatcher` (`dispatch_on`, `dispatch_at_on`, `dispatch_after_on`, and `dispatch_later_on`); queues declared in `jobs.queue_priorities` are now included in worker polling.
- First-class outbound `HttpClient` support with pooled reqwest transport, base URLs/headers, timeouts, bounded concurrency, safe idempotent retries, redacted tracing, typed responses/errors, raw escape hatch, pluggable transports, and `HttpClientFake` request assertions.
- Graceful encryption-key rotation through `CryptConfig.previous_keys`; new ciphertext uses only the primary key while old keys remain decrypt-only.
- `ModelPath<M>` for typed route-model binding with distinct malformed-key (400) and missing-model (404) responses.
- Public `ImageDecodeLimits` plus blocking-safe `ImageProcessor::process_file` / `process_bytes` pipelines and explicit trusted-input unbounded constructors.
- `HashManager::needs_rehash` for detecting stored Argon2 hashes that should be upgraded to the current algorithm, version, or work factors.
- Fluent `TestResponse` assertions for status, headers, JSON paths/fragments/shapes, validation errors, redirects, and downloads.
- First-class `EventFake`, `JobFake`, `MailFake`, `NotificationFake`, `StorageFake`, and `ClockFake` testing support with typed records/assertions and `TestAppBuilder` installers; `acting_as`, bearer/session client defaults, rollback-only `DatabaseTestTransaction`, typed database assertions, and factory states/sequences/parent keys complete the additive testing layer without config, migration, or breaking changes.
- Reusable `PluginTestHarness` / `PluginTestApp` author testing with real plugin bootstrap, dependency resolution, config precedence, contribution metadata, service/HTTP assertions, and graceful shutdown. `TestAppBuilder::register_plugin` / `register_plugins` are also public testing conveniences; the addition has no config or breaking API change.
- Public idempotent `AppContext::shutdown()` for custom kernel hosts, and test-only `TestAppBuilder::replace_service` APIs that preserve strict production container registration.
- First-class cache tags through `CacheManager::tags` / `TaggedCache`, with canonical multi-tag identity and cross-process version-based invalidation.
- Typed `JobDispatcher::dispatch_at(DateTime)` and `dispatch_after(Duration)` scheduling alongside the compatible raw epoch-millisecond API.
- `AppTransaction::dispatch_event_after_commit`, with actor/request origin captured when registered and dispatch only after a successful commit.
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
- A first-class testing guide covering production-bootstrap reuse, in-process HTTP assertions, test service replacement, typed factories, and PostgreSQL cleanup safety.
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

- Contract manifest version 2 separates business `action_name` from route ID, replaces string-only `params` with typed `parameters`, adds action errors/realtime message payloads, and makes generated SDK filenames/client methods follow the explicit action name.
- TypeScript route manifests, SDK actions, realtime descriptors, and compatibility form adapters now render from the frozen contract manifest. The pure SDK remains the default; form-oriented `routes/*.ts` adapters are generated only with `types:export --route-form-adapter` or `TypeScriptExportContext::route_form_adapter`.
- New queued notification dispatch produces N jobs for N channels, giving each channel independent retry/history/dead-letter/metrics state. The legacy aggregate job builder and serialized shape remain readable for rolling compatibility.
- XLSX exports stream database rows through a bounded channel into `rust_xlsxwriter` constant-memory worksheets. Queued exports also stream the completed ZIP to a temporary file and into an overriding `deliver_file` implementation, so neither rows, worksheet cells, nor the final queued artifact require an unbounded `Vec<u8>`.
- `RequestId` now has validated `try_new`/`FromStr`/`TryFrom` construction and UUIDv7 generation. File writes are performed by a named background worker; overload drops the newest complete record rather than blocking request tasks, and application shutdown flushes accepted records within `logging.file_flush_timeout_ms`.
- Custom environment labels now fail closed to strict security unless `app.security_tier` explicitly confirms/overrides them. Generated env examples use `FOUNDRY__...`; legacy unprefixed overlays remain lower-precedence-compatible but produce doctor warnings.
- WebSocket server shutdown now sends close code 1001 (`server shutdown`), rejects racing handshakes, and drains subscriptions, presence, actor tracking, and lifecycle hooks within `app.background_shutdown_timeout_ms`.
- WebSocket force-disconnect now requires both guard and actor ID, preventing equal IDs in different identity domains from disconnecting each other. `ImageProcessor::quality` is explicitly JPEG-only; WebP uses lossless encoding and rejects an explicitly supplied quality.
- Event semantic IDs are now globally unique across Rust event types; provider/plugin collisions fail bootstrap while multiple listeners for one event type remain supported.
- Removed the empty `webauthn` Cargo feature and module. They exposed no passkey implementation; consumers selecting the feature must remove it from their dependency declaration.
- Country JSON collections now hydrate to `Vec<CountryCurrency>` / `Vec<String>`, and `CountryStatus::parse` returns `Option` for explicit invalid-value handling. Serialized country JSON remains compatible.
- `ApiSchema` now registers only JSON Schema/OpenAPI metadata and no longer implicitly registers a DTO for TypeScript export. DTOs that should produce standalone `.ts` files must derive both `ts_rs::TS` and `foundry::TS`. See `docs/consumer-impact/2026-07-10-framework-completeness-upgrade.md`.
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

- Existing byte-only `DatatableExportDelivery::deliver` implementations remain source compatible, but queued artifacts above 25 MiB now fail the bounded legacy adapter instead of allocating an unbounded buffer. Override `deliver_file` to stream or copy larger exports.
- `RedisConfig` struct literals must add `connect_timeout_ms` and `command_timeout_ms` or use `..RedisConfig::default()`. TOML/environment configuration remains compatible and receives 5-second defaults.
- Named routes included in contract export must declare `action_name(...)`; duplicate actions fail export even when route IDs differ. Contract/route manifest struct literals must adopt typed parameter/error/message fields, contract JSON consumers must accept version 2, and generated SDK names/options may change to the explicit action plus separated path/query/header/cookie groups.
- `types:export` no longer creates compatibility `routes/*.ts` form adapters unless `--route-form-adapter` is passed; programmatic callers that still need them must set `route_form_adapter: true`. Direct `TypeScriptExportContext`, `DocumentedRoute`, `RouteManifestEntry`, and `ContractHttpTransport` struct literals must add `route_form_adapter`, `auth`, `request_content_type`, and `content_type` respectively. Custom request media types are opt-in metadata declared with `request_content_type(...)`; handlers remain responsible for using a compatible extractor.
- `ResolvedS3Config.key` and `secret` are now `Option<String>` and direct literals must add `session_token`. Calling `url()` on a private storage disk now returns an error; use `temporary_url()` for supported private delivery.
- `AuditConfig` struct literals must add `retention_days` or use `..AuditConfig::default()`; the default is `0` (keep forever).
- Apply published migration `000000000014_add_notification_notifiable_type` before deploying new notification insert/query code. It adds the typed ownership column and deterministic/unread indexes; existing rows receive `default` and must be backfilled before adopting a custom `Notifiable::notifiable_type()`.
- Removed `NoopExportDelivery`; queued datatable exports now require a registered `Box<dyn DatatableExportDelivery>`. `DatatableContext.locale` changed from `Option<&str>` to owned `Option<String>` so queued work can preserve it; use `.as_deref()` at borrowed call sites.
- Removed inert `ModelQuery::without_defaults()`; no derive or execution path implemented the advertised automatic default relations, so the method never changed a query. Remove calls until a real typed default-eager-loading contract exists.
- `LoggingConfig` struct literals must add `file_queue_capacity`, `file_max_record_bytes`, and `file_flush_timeout_ms` or use `..LoggingConfig::default()`. Processes that install tracing before Foundry must call `App::builder().use_external_tracing_subscriber()`; implicit subscriber conflicts now fail bootstrap. `RequestId::new` rejects invalid values, so fallible input paths should use `RequestId::try_new`.
- `AppConfig` struct literals must add `security_tier` or use `..AppConfig::default()`. Custom labels that previously behaved relaxed now use strict WebSocket-origin/proxy/public-observability policy until explicitly overridden.
- Cursor tokens are now opaque version-1 typed `(sort column, primary key)` positions; old single-value tokens are rejected. Remove `CursorPaginated::encode_cursor` / `with_cursors`, restart once without a cursor, and thereafter pass returned `cursors.next` / `cursors.prev` unchanged.
- Existing `model_translations.translatable_id UUID` tables must run the published `000000000013_alter_model_translation_ids_to_text` migration before deploying the new text-bound translation runtime. Its rollback fails intentionally once non-UUID IDs exist.
- Rust struct literals for `CryptConfig` must add `previous_keys` (normally `Vec::new()`) or use `..CryptConfig::default()`.
- Replace `WebSocketPublisher::disconnect_user(actor_id)` with `disconnect_actor(guard, actor_id)`. Direct `__system:disconnect` publishers must include both serialized fields and rolling deployments must coordinate old/new kernels.
- Image pipelines that call `.quality(...)` before encoding WebP or another non-JPEG format now return an error; remove the quality call for lossless WebP or encode JPEG when lossy quality control is required.
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
