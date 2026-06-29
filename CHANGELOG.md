# Changelog

All notable changes to this project will be documented in this file.

The format is inspired by Keep a Changelog, adapted for Foundry's pre-`1.0` releases.

## [Unreleased]

### Security

- Auth now rejects actors whose stored guard does not match the requested guard instead of retagging them, preventing a user token/session from satisfying guard-only admin routes or WebSocket channels.
- `enable_observability()` now protects `/_foundry/*` with the app's default auth guard by default; public diagnostics require the explicit `enable_public_observability()` / `ObservabilityOptions::public()` opt-in, and production-like public registration logs a warning.
- TOTP MFA now uses HMAC-SHA1 per RFC 6238 so codes verify against mainstream authenticator apps (Google Authenticator, Authy, 1Password, etc.), which ignore the otpauth `algorithm` parameter and always compute SHA1. **TOTP factors enrolled on previous builds generate different codes and must be re-enrolled.**
- `min_numeric`, `max_numeric`, and `between` validation rules now reject values that fail to parse as a number, as well as `NaN` and infinities; previously non-numeric input (including `"NaN"`, which defeats all numeric comparisons) silently passed bound checks. The `numeric` rule is now parse-based: malformed shapes like `"1.2.3"` are rejected, and scientific notation such as `"1e10"` is accepted.
- The Postgres compiler now validates `EXTRACT` field names against the known date/time fields and restricts `Sql::op` custom operators to legal operator characters, closing a raw-SQL interpolation path if either was ever fed untrusted input.
- Password-reset and email-verification tokens now enforce their TTL inside the consuming `DELETE`, so presenting an expired token no longer destroys it; expired and unknown tokens return the same "invalid or expired" message.
- `StaticBearerAuthenticator` stores and compares SHA-256 digests of its tokens instead of plaintext map keys, removing token-comparison timing as a side channel.
- Secret-bearing config structs (`AppConfig.signing_key`, SMTP/Resend/Postmark/Mailgun/SES credentials, S3 secret) redact their secrets in `Debug` output, and `DatabaseConfig`/`RedisConfig` redact URL credentials, so `{:?}` logging can no longer leak credentials.

### Fixed

- A worker whose job-lease heartbeat fails (lease expired, claimed elsewhere, or backend unreachable past the lease TTL) now cancels the running job instead of letting it race the redelivered copy on another worker. Transient renewal errors are retried while the last successful renewal still covers the lease TTL.
- Plugins that booted successfully are now shut down (in reverse order) when a later plugin's `boot()` fails, so resources acquired during partial bootstrap no longer leak.
- The in-memory scheduler leadership backend no longer renews an already-expired lease; like the Redis backend, the previous holder must win a fresh election, preventing split-brain in single-process and test setups.
- The WebSocket pub/sub task now resubscribes with exponential backoff when its backend subscription ends or fails, instead of going permanently silent while publishes keep succeeding; dropping the server also aborts the task instead of leaking it.
- Scheduler leadership state uses acquire/release atomic ordering, and dropping the scheduler or a schedule overlap-lock guard outside a Tokio runtime now logs a warning instead of silently leaving the lock to lapse via TTL.
- Image dimension validation streams the upload from disk on the blocking thread pool instead of reading the entire file into memory on the async runtime.
- Bodyless contract actions that still expose request metadata (for example path/query params on `GET`) no longer emit an OpenAPI `requestBody`, and generated SDK actions no longer require a payload argument that would be dropped at runtime.
- Generated `FoundryEndpoint.applyServerErrors` now safely ignores malformed or non-object error payloads before reading `.errors`, so unusual transport/client errors no longer mask the original failure with a runtime crash.
- Multipart `Validated<T>` extraction now removes framework-owned uploaded temp files when validation fails before the handler receives the DTO.
- `#[derive(Validate)]` now generates multipart cleanup hooks for `UploadedFile`, `Option<UploadedFile>`, and `Vec<UploadedFile>` fields, and rejects unsupported nested upload wrappers with a clear derive error.
- Datatable numeric filters now reject out-of-range `Int16`/`Int32` number values instead of wrapping them during downcast.
- WebSocket `on_leave` hooks and presence leave cleanup now run only when an `unsubscribe` actually removed an active subscription, so forged/unmatched unsubscribe frames cannot trigger leave-side effects.

### Added

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

- `EmailMessage::template(...)` is now `async` (template files are read off the async runtime); add `.await` before the `?`.
- TOTP MFA switched from HMAC-SHA256 to RFC 6238 HMAC-SHA1 (see Security); previously enrolled TOTP factors must be re-enrolled.
- Validation: `min_numeric`, `max_numeric`, `between`, and `numeric` are stricter (see Security); requests that previously passed with non-numeric values in bounded fields now fail validation.
- WebSocket `message` and `client_event` frames now require an active matching channel/room subscription before handlers or client-event relay run.
