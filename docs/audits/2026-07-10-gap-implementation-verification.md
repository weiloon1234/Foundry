# Laravel-Inspired Gap Implementation Verification

Date: 2026-07-10  
Source report: [Laravel-inspired gap report](2026-07-10-laravel-inspired-gap-report.md)

This ledger re-checks the current working tree rather than treating the historical findings as current. It accounts for all 98 finding IDs exactly once. PostgreSQL-only database support and raw SQL migration bodies remain accepted project constraints.

Status meanings:

- **Implemented** — the current tree contains the runtime/API/documentation change and focused test coverage named in the evidence column.
- **Measured** — the finding called for evidence before deciding on feature gates; measurement is the deliverable, not a speculative crate split.
- **Conditional — no change** — adding the proposed feature without a demonstrated consumer need would make the framework broader without making its current contract better.

Breaking changes and newly available first-class support are consolidated in the forwardable [Framework Completeness Upgrade — Consumer Impact](../consumer-impact/2026-07-10-framework-completeness-upgrade.md). Database migrations in that document are the consumer deployment boundary.

## Package, bootstrap, and runtime surfaces

| ID | Status | Current-tree evidence |
| --- | --- | --- |
| `ROOT-01` | Implemented | The empty `webauthn` feature/module is absent from `Cargo.toml` and `src/auth/mfa/mod.rs`; removal and consumer action are covered by `CHANGELOG.md` and the consumer-impact document. |
| `ROOT-02` | Measured | `tools/build-metrics.sh` performs a clean dedicated-target release build and reports elapsed time, rlib bytes, target size, and dependency count; `make build-metrics`, `.github/workflows/release-readiness.yml`, and `docs/release-checklist.md` make that evidence a release input. The observed baseline is recorded in [Foundry Build-Cost Baseline](2026-07-11-build-cost-baseline.md); no optional integration was feature-gated speculatively. |
| `FND-01` | Implemented | `src/foundation/app.rs` now ignores only `NotFound` from dotenv and propagates malformed/unreadable files; its `dotenv_loader_ignores_only_missing_files` unit test covers all three outcomes. |
| `FND-02` | Implemented | Test-only replacement is explicit through `TestAppBuilder::replace_service` / `replace_service_arc` in `src/testing/client.rs`, backed by the strict container replacement path in `src/foundation/container.rs`; focused tests and `docs/guides/testing.md` cover registered and missing services. |
| `KERNEL-01` | Implemented | Public, idempotent `AppContext::shutdown()` in `src/foundation/app.rs` drains managed tasks and plugin hooks; kernel `app()` accessors and shutdown tests in that module plus plugin fixture tests cover custom-host cleanup. |
| `HTTP-01` | Implemented | Actor rate-limit storage keys in `src/http/middleware.rs` include both `GuardId` and actor ID; focused middleware tests cover equal IDs under distinct guards, and the behavioral change is documented in the consumer-impact document. |
| `HTTP-02` | Implemented | `src/http/model_path.rs` provides typed `ModelPath<M>` binding with distinct malformed-key and missing-model responses; `tests/http_model_path_acceptance.rs` covers UUID, integer, and text primary keys, and `docs/guides/routes-and-middleware.md` documents it. |
| `HTTP-03` | Conditional — no change | Foundry remains API/SPA-oriented and has no server-rendered request-session bag contract. General flash state would be product-scope expansion rather than a repair; auth credential sessions remain deliberately separate. |
| `CLI-01` | Implemented | `src/cli/io.rs`, `src/cli/mod.rs`, and `src/kernel/cli.rs` add injected command I/O, prompts/confirmations/progress, typed `CommandExit`, and status-returning runners; `src/testing/cli.rs` supplies `CommandIoFake`, with kernel/unit coverage and `docs/guides/cli-commands.md`. |
| `CLI-02` | Implemented | `src/cli/dev.rs` registers the CLI-only `dev` supervisor for selected `PROCESS` values, prefixed output, bounded opt-in restart/backoff, and coordinated signal/child cleanup; deterministic fake-runner tests cover restart exhaustion, clean exit, sibling cancellation, output prefixing, and help wording. `src/foundation/app.rs`, `docs/guides/cli-commands.md`, and the consumer-impact document expose the command. It is process orchestration, not project generation. |
| `SCH-01` | Implemented | `src/kernel/scheduler.rs` isolates overlap-lock backend errors per schedule and preserves the due cursor for retry; `overlap_lock_backend_error_isolated_and_cron_cursor_retries` and the interval-cursor test cover the regression. |
| `JOB-01` | Implemented | Async job dispatch examples now await their futures in `docs/guides/background-processing.md`; `tests/fixtures/blueprint_app/src/app/compile_checked_guide_examples.rs` compile-checks representative dispatch examples. |
| `JOB-02` | Implemented | History persistence in `src/jobs/mod.rs` is gated directly by `track_history`, independent of diagnostics capture; `tests/observability_acceptance.rs` and the background-processing guide cover disabled capture with enabled history. |
| `JOB-03` | Implemented | `JobDispatcher::dispatch_at(DateTime)` and `dispatch_after(Duration)` plus explicit queue variants live in `src/jobs/mod.rs`; unit tests cover scheduling and overflow, while the guide fixture compile-checks both typed forms. |
| `WS-01` | Implemented | `WebSocketPublisher::disconnect_actor` carries guard plus actor ID through `src/websocket/mod.rs` and `src/kernel/websocket.rs`; unit and `tests/auth_acceptance.rs` coverage verifies that an equal ID under another guard stays connected. |
| `WS-02` | Implemented | WebSocket shutdown in `src/kernel/websocket.rs` rejects racing upgrades, sends close code 1001, and drains connection/presence/lifecycle state within the application timeout; focused kernel tests and `docs/guides/websocket.md` cover the contract. |
| `WS-03` | Conditional — no change | Bounded replay remains adequate for current products. Durable resume and binary frames are intentionally deferred until a concrete protocol consumer establishes semantics, persistence, and compatibility requirements. |
| `CFG-01` | Implemented | `ConfigRepository` has an opaque/redacted `Debug` implementation in `src/config/mod.rs`; tests assert arbitrary nested secret values do not appear. |
| `CFG-02` | Implemented | `SecurityTier` and `AppConfig::resolved_security_tier` in `src/config/mod.rs` separate security posture from environment labels and fail custom labels closed; config/doctor tests and `docs/guides/getting-started.md` cover explicit overrides. |
| `CFG-03` | Implemented | Published config metadata in `src/config/published.rs` is the SSOT for unknown TOML and environment-overlay detection; `src/foundation/doctor.rs` reports unknown framework keys, unknown `FOUNDRY__` overlays, and legacy overlays, with strict-mode tests and guide documentation. |

## Data, files, and infrastructure

| ID | Status | Current-tree evidence |
| --- | --- | --- |
| `DB-01` | Implemented | Cursor pagination in `src/database/query.rs` rejects mixed directions, owns deterministic `(sort, primary key)` ordering, handles nulls/duplicates, and derives real previous/next tokens; unit and PostgreSQL cases in `tests/database_acceptance.rs` cover forward/backward boundaries and invalid tokens. |
| `DB-02` | Implemented | `docs/query-blueprint-status.md` no longer advertises projection cursor pagination; the database guide documents the actual model-query surface and the compile-checked guide fixture uses it. |
| `DB-03` | Implemented | `make:model --table` and conservative default pluralization are implemented in `src/database/scaffold.rs`; scaffold unit/acceptance coverage includes irregular naming and validation, with CLI guide and consumer-impact documentation. |
| `DB-04` | Implemented | The inert `ModelQuery::without_defaults()` surface was removed rather than pretending to support default eager relations; removal and consumer action are documented in `CHANGELOG.md` and the consumer-impact document. |
| `DT-01` | Implemented | `src/datatable/context.rs`, `export_job.rs`, and `download.rs` own and restore dispatch-time actor, locale, and timezone through query/mapping/XLSX work; unit and `tests/datatable_acceptance.rs` coverage exercise queued context. |
| `DT-02` | Implemented | The silent no-op delivery fallback is removed; `src/datatable/export_job.rs` requires exactly one `DatatableExportDelivery` and tests assert missing delivery fails. Registration is documented in `docs/guides/datatable.md`. |
| `DT-03` | Implemented | `GeneratedDatatableExportFile`, `DatatableExportDelivery::deliver_file`, and the 25 MiB legacy adapter bound in `src/datatable/export.rs` keep queued artifacts file-backed; `src/datatable/download.rs` streams database rows through a bounded channel into constant-memory XLSX output, and tests cover cleanup and compatibility. |
| `STO-01` | Implemented | Built-in local/S3 writes now set `StoredFile.url` only for stable public objects, while private `url()` fails; `src/storage/local.rs`, `src/storage/s3.rs`, storage tests, and `docs/guides/storage-and-imaging.md` align the contract and corrected async examples. |
| `STO-02` | Implemented | `StorageAdapter`, disk, and manager streaming APIs are in `src/storage/adapter.rs`, `disk.rs`, and `mod.rs`; local storage performs bounded atomic I/O and `src/storage/s3.rs` implements multipart upload/abort and streamed reads, with adapter tests and guide examples. |
| `STO-03` | Implemented | `src/storage/config.rs` permits either a complete explicit credential set (including optional session token) or the AWS provider chain; `src/storage/s3.rs` builds the selected provider, and config/storage tests cover valid and partial combinations. |
| `STO-04` | Implemented | `src/testing/storage.rs` implements `StorageFake` through the production `StorageAdapter` boundary with content/existence/write assertions; testing API docs and `docs/guides/testing.md` document opt-in custom-driver installation. |
| `ATT-01` | Implemented | `src/attachments/orphans.rs` walks cursor pages to exhaustion and reports incomplete custom-adapter scans; local/S3 `list_prefix_after` implementations and unit tests cover exclusive cursors, page continuation, and wildcard-safe database lookup. |
| `ATT-02` | Implemented | Attachment single/replacement operations in `src/attachments/mod.rs` use a PostgreSQL advisory transaction lock and atomic insert/removal; concurrent PostgreSQL coverage lives in `tests/attachments_acceptance.rs`. |
| `ATT-03` | Implemented | `HasAttachments::reorder_attachments` validates exact collection membership and atomically rewrites order; new inserts append after the maximum, and `tests/attachments_acceptance.rs` covers valid and invalid reorder requests. |
| `IMG-01` | Implemented | `ImageDecodeLimits::default()` is enforced by public `open`/`from_bytes` in `src/imaging/mod.rs`, with explicit trusted-input unbounded variants; limit and oversized-dimension tests plus the storage/imaging guide cover the boundary. |
| `IMG-02` | Implemented | `ImageProcessor::quality` is explicitly JPEG-only and WebP is documented/implemented as lossless; `src/imaging/mod.rs` tests assert WebP and other non-JPEG quality requests fail clearly. |
| `IMG-03` | Implemented | `ImageProcessor::process_file`, `process_bytes`, and limit variants run the entire pipeline through Foundry's blocking helper; async responsiveness/panic tests live in `src/imaging/mod.rs`, with guide examples. |
| `CACHE-01` | Implemented | Redis cache flush in `src/cache/redis_store.rs` advances a namespace generation rather than scanning/global deletion; memory/Redis cache tests and `docs/guides/caching-and-redis.md` cover cold-generation behavior. |
| `CACHE-02` | Implemented | `src/cache/tagged.rs` and `CacheManager::tags` implement canonical multi-tag, generation-based invalidation across processes; tag tests cover normalization/invalidation and the guide documents custom-store control-value requirements. |
| `REDIS-01` | Implemented | `src/redis/mod.rs` centralizes reconnect-invalidating cached connections and bounded connect/command/pub-sub timeouts; config and Redis tests exercise timeout validation, invalidation, and reconnect behavior. |
| `REDIS-02` | Implemented | Namespace-safe `command`, `pipeline`, `transaction`, and `script` builders in `src/redis/mod.rs` require typed `RedisKey` arguments before execution; unit/real-Redis tests and the caching/Redis guide cover ordinary and prefix-key commands. |

## Domain services

| ID | Status | Current-tree evidence |
| --- | --- | --- |
| `SET-01` | Implemented | `Setting::set` in `src/settings/mod.rs` now errors when no row is updated and directs intentional creation to `upsert`; PostgreSQL coverage in `tests/support_stores_acceptance.rs` and the model-extensions guide cover the distinction. |
| `SET-02` | Implemented | `SettingType::parse` failure now produces contextual hydration errors instead of coercing to text; unit and support-store acceptance tests cover unknown stored types. |
| `META-01` | Implemented | `with_meta` / `with_metadata` on model, relation, and many-to-many queries use the model-extension cache and lazy sibling batching; implementation spans `src/metadata/mod.rs` and `src/database/{extensions,query,relation}.rs`, with PostgreSQL coverage in `tests/support_stores_acceptance.rs`. |
| `META-02` | Implemented | `delete_all_meta`, `MetadataOwner`, `audit_metadata_orphans`, and `prune_metadata_orphans` in `src/metadata/mod.rs` provide explicit lifecycle maintenance; unit and support-store acceptance tests cover validation, audit, prune, and cache invalidation. |
| `COUNTRY-01` | Implemented | `src/countries/iana-zone-2026a.tab` feeds country seed timezones through `src/countries/mod.rs`; unit and PostgreSQL support-store tests verify Malaysia and the intentionally unmapped exceptions, and the guide documents reseeding. |
| `COUNTRY-02` | Implemented | `CountryStatus::parse` returns `Option` and row hydration reports unknown values contextually; `src/countries/mod.rs` unit tests and `tests/public_api_acceptance.rs` cover known/unknown cases. |
| `COUNTRY-03` | Implemented | Country JSON fields now hydrate as `Vec<CountryCurrency>` / `Vec<String>` rather than raw JSON; serialization/hydration tests in `src/countries/mod.rs` and `tests/support_stores_acceptance.rs` preserve the wire shape. |
| `AUTH-01` | Implemented | One guard-scoped `ActorHydrator` registry in `src/auth/mod.rs` is shared by token, session, custom bearer, HTTP, and WebSocket paths; identity drift/absence reject credentials and token abilities are intersected. Unit, auth, token, and plugin acceptance tests cover registration and hydration. |
| `AUTH-02` | Implemented | `src/auth/lockout.rs` now locks on exactly the configured Nth failure; focused tests cover thresholds of one and five plus reset/expiry behavior, and the consumer-impact document calls out the semantic correction. |
| `AUTH-03` | Implemented | The empty passkey promise was removed together with the package feature; `Cargo.toml`, `src/auth/mfa/mod.rs`, `CHANGELOG.md`, and consumer guidance consistently describe the removal. |
| `VAL-01` | Implemented | Named custom-rule execution in `src/validation/executor.rs` preserves the returned `ValidationError.message` unless an explicit override/i18n message wins; validation unit tests cover precedence and returned text. |
| `VAL-02` | Implemented | Validation and i18n guides now use the runtime `{{attribute}}` placeholder contract; representative validation examples are included in the compile-checked consumer fixture. |
| `VAL-03` | Implemented | Manual and derive paths implement `required_if`, `required_unless`, `required_with`, `present`, `sometimes`, `prohibited`, typed boolean, and collection `distinct`; coverage spans `src/validation`, `tests/validate_derive_acceptance.rs`, compile-fail/pass UI fixtures, OpenAPI, and TypeScript metadata tests. |
| `EVT-01` | Implemented | `EventRegistry` in `src/events/mod.rs` enforces global semantic `Event::ID` uniqueness across Rust types while retaining multiple listeners per type; provider/plugin collision tests cover bootstrap rejection. |
| `EVT-02` | Implemented | `AppTransaction::dispatch_event_after_commit` in `src/foundation/app.rs` snapshots actor/request origin and runs only after commit; `tests/event_transaction_acceptance.rs` covers commit, rollback, and origin behavior. |
| `NOTIFY-01` | Implemented | Built-in channel preparation in `src/notifications/channel.rs`, `mod.rs`, and `job.rs` fails when a selected email/database/broadcast channel has no required output; unit and `tests/notification_queue_acceptance.rs` cover immediate and queued paths. |
| `NOTIFY-02` | Implemented | Queued preparation now produces one `SendNotificationJob` per selected channel so retry/history/dead-letter state is independent; compatibility serialization and per-channel behavior are tested in `src/notifications/mod.rs` and `tests/notification_queue_acceptance.rs`. |
| `NOTIFY-03` | Implemented | `src/notifications/database.rs` provides typed ownership-scoped list/page/unread/count/mark/delete APIs; migration `000000000014_add_notification_notifiable_type` adds the morph type/indexes, with PostgreSQL coverage in `tests/support_stores_acceptance.rs`. |
| `MAIL-01` | Implemented | `EmailManager` dispatches immediate/delayed mail on `[email].queue`, explicit queue dispatch APIs live in `src/jobs/mod.rs`, and worker polling includes configured priorities; email/job tests and the email guide cover the behavior. |
| `MAIL-02` | Implemented | `EmailManager::render_template` in `src/email/mod.rs` resolves beneath configured `template_path`, retaining the explicit per-call message escape hatch; unit tests create a configured template tree and verify rendering. |
| `MAIL-03` | Implemented | SMTP construction in `src/email/smtp.rs` uses a true plaintext builder for `encryption = "none"`; configuration/transport tests and `docs/guides/email-and-notifications.md` distinguish none, STARTTLS, and TLS. |
| `MAIL-04` | Implemented | HTML rendering in `src/email/template.rs` escapes `{{variable}}`, reserves triple braces for trusted raw markup, and leaves text templates unescaped; focused tests cover escaping, raw syntax, missing values, and non-recursive replacement. |
| `I18N-01` | Implemented | Canonical locale lookup in `src/i18n/mod.rs` is ASCII case-insensitive while preserving directory spelling; tests cover lookup, regional fallback, private subtags, and deterministic locale listing. |
| `I18N-02` | Implemented | Locale directories/files are sorted and case-only locales or duplicate flattened keys fail instead of using filesystem last-wins behavior; focused tests and `docs/guides/i18n.md` cover both diagnostics. |
| `I18N-03` | Conditional — no change | Plural/choice rules remain deferred because no current localized product copy has established the needed cardinal/ordinal semantics. The immutable deterministic catalog contract is complete without guessing a plural API. |
| `TRANS-01` | Implemented | Final translation fallback in `src/translations/mod.rs` selects the lexicographically first locale; unit tests cover current, default, and deterministic final fallback, matching `docs/guides/model-extensions.md`. |
| `TRANS-02` | Implemented | Translation owner IDs bind/store as text and joins cast model keys at the boundary; runtime changes in `src/translations/mod.rs` plus migration `000000000013_alter_model_translation_ids_to_text` support UUID, integer, and text keys, with unit/PostgreSQL coverage. |

## Observability, contracts, and extensibility

| ID | Status | Current-tree evidence |
| --- | --- | --- |
| `LOG-01` | Implemented | `src/logging/mod.rs` propagates subscriber, file-writer, and OTLP setup failures; explicit host ownership uses `use_external_tracing_subscriber`, covered by unit and `tests/logging_initialization_acceptance.rs`. |
| `LOG-02` | Implemented | `RequestId` in `src/logging/request_id.rs` validates visible ASCII/length and generates UUIDv7 values; middleware replaces invalid inbound IDs, with request-ID and middleware tests plus consumer guidance. |
| `LOG-03` | Implemented | `src/logging/file_writer.rs` uses a bounded non-blocking worker with whole-record drops and shutdown flushing; `src/logging/metrics.rs` exposes pressure/error/timeout counters, with focused unit and observability acceptance coverage. |
| `AUDIT-01` | Implemented | `AuditContext`, `scope_audit`, and `AuditManager::record` in `src/audit/mod.rs` support attributed/redacted non-HTTP and domain audit entries through any executor; `tests/audit_acceptance.rs` covers scoped and manual records. |
| `AUDIT-02` | Implemented | Configured/manual pruning lives in `src/audit/mod.rs` and CLI command `src/audit/cli.rs`; default retention zero is non-destructive, while unit/acceptance tests cover explicit cutoff/configured pruning and zero-policy refusal. |
| `CONTRACT-01` | Implemented | Named exported routes must declare an explicit, unique business `action_name`; collection/validation is in `src/contract/mod.rs` and HTTP route builders, with contract tests and blueprint fixture actions. |
| `CONTRACT-02` | Implemented | Contract manifest v2 records typed path/query/header/cookie parameters, request media type, and per-action errors in `src/contract/mod.rs`; unit tests and `docs/api-reference.md` cover validation and serialized shape. |
| `CONTRACT-03` | Implemented | `typed_channel` / `typed_channel_with_options` in `src/websocket/mod.rs` decode typed payloads before handlers and export the message schema; tests cover successful decoding, 422 rejection, and explicit `raw_channel`. |
| `CONTRACT-04` | Conditional — no change | Command/workflow transport adapters remain deferred because there is no generator consumer defining their wire contract. HTTP and realtime are the only frozen manifest transports, avoiding an unused speculative schema. |
| `OPENAPI-01` | Implemented | `src/openapi/spec.rs` renders explicit action names as `operationId` plus `bearerAuth` security scheme/operation security while retaining Foundry extensions; focused spec tests cover public and guarded actions. |
| `OPENAPI-02` | Implemented | The OpenAPI renderer consumes manifest-v2 typed path/query/header/cookie parameters, request media types, schemas, and canonical status descriptions; `src/openapi/spec.rs` tests cover each location/content type. |
| `OPENAPI-03` | Implemented | Standard and action-specific error schemas are grouped by status into operation responses without overwriting; spec tests cover shared-status `oneOf` output and descriptions. |
| `TS-01` | Implemented | `src/typescript/mod.rs` freezes one `ContractManifest` before rendering route manifest, SDK, realtime descriptors, and optional form adapters; pure SDK is default and `route_form_adapter` is opt-in, with generator tests and TypeScript guide documentation. |
| `TS-02` | Conditional — no change | Core output deliberately remains framework-neutral transport code. React/Vue/Flutter state adapters should be separate, opt-in consumers only when the team chooses a concrete frontend contract. |
| `PLUGIN-01` | Implemented | `src/testing/plugin.rs` provides `PluginTestHarness` / `PluginTestApp` over production bootstrap, dependency resolution, contributions, and shutdown; `tests/plugin_acceptance.rs` and the plugin consumer fixture cover author workflows, while component authoring has the in-app `make:plugin` file generator. Cargo remains package discovery. |
| `SUP-01` | Implemented | `HashManager::needs_rehash` in `src/support/hash.rs` checks Argon2 algorithm/version/work factors; focused tests cover current, weaker, stronger, malformed, and other-algorithm hashes, with API/consumer docs. |
| `SUP-02` | Implemented | `CryptConfig.previous_keys` and `CryptManager` in `src/support/crypt.rs` use the primary key for encryption and ordered previous keys only for decryption; tests cover rotation, invalid keys, and redacted debug output. |

## Testing, wire contracts, tooling, and documentation

| ID | Status | Current-tree evidence |
| --- | --- | --- |
| `TEST-01` | Implemented | `src/testing/fakes.rs`, `storage.rs`, `http_client.rs`, and `clock.rs` provide event/job/mail/notification/storage/outbound-HTTP/clock fakes with typed records/assertions and builder installers; unit plus `tests/testing_layer_acceptance.rs` exercise composition. |
| `TEST-02` | Implemented | `TestResponse` in `src/testing/client.rs` has fluent status, header, exact/path/fragment/shape JSON, validation, redirect, and download assertions; focused tests cover pass/fail messages and `docs/guides/testing.md` documents use. |
| `TEST-03` | Implemented | `src/testing/database.rs` supplies rollback-only `DatabaseTestTransaction` and typed database assertions; `src/testing/factory.rs` adds states, indexed sequences, and parent keys, with PostgreSQL coverage in `tests/testing_layer_acceptance.rs`. |
| `TEST-04` | Implemented | Test clients support `acting_as`, bearer/session defaults, and frozen application time through `ClockFake`; CLI output is captured by `CommandIoFake`. Unit and testing-layer acceptance coverage exercises these helpers together. |
| `WIRE-01` | Implemented | `foundry-macros/src/common.rs` is the shared serde wire-name resolver used by schema, validation, multipart, and generated metadata paths; derive acceptance and pass/fail UI fixtures cover `rename`, `rename_all`, and rejected asymmetric names. |
| `WIRE-02` | Implemented | `ApiSchema` no longer implies standalone TypeScript registration; `foundry-macros/src/lib.rs` separates schema and explicit `foundry::TS`, with pass/fail derive fixtures and migration guidance in the consumer-impact document. |
| `CLIENT-01` | Implemented | `src/http_client/` provides configured pooled transport, base URL/headers, timeout/concurrency/retry policy, redacted tracing, typed errors/responses, and raw escape hatch; `HttpClientFake` and `tests/http_client_acceptance.rs` cover deterministic request sequences/assertions, with `docs/guides/http-client.md`. |
| `SCAFFOLD-01` | Implemented | `src/database/scaffold.rs` provides one-file component generators for request/DTO, policy, event/listener, notification/mail, datatable, in-app plugin component, and test. Unit plus `tests/database_lifecycle_acceptance.rs` generate and format every template; README/CLI/consumer docs state explicitly that no starter/skeleton project generator or installer is included. |
| `DOC-01` | Implemented | Root `LICENSE` now matches the declared MIT package license; `tests/api_audit.rs` asserts the promised file exists. |
| `DOC-02` | Implemented | `docs/public-api-contract.md` exists and is linked from `README.md`, generated API index metadata, and `docs/api/index.md`; `tests/api_audit.rs` checks the link targets. |
| `DOC-03` | Implemented | Confirmed guide drift was corrected across background jobs, cursor pagination, storage/imaging, validation, and datatables; `tests/fixtures/blueprint_app/src/app/compile_checked_guide_examples.rs` compile-checks representative consumer snippets and fixture acceptance keeps them in CI. |
| `DOC-04` | Implemented | `docs/guides/testing.md` now covers production-bootstrap reuse, response assertions, fakes, auth/time/CLI helpers, database rollback, factories, and PostgreSQL safety; it is indexed by `docs/guides/README.md`. |
| `DOC-05` | Implemented | `src/config/api_docs_metadata.rs` contains descriptions for `public`, `settings`, and `typescript`; generated `docs/api/modules/public.md`, `settings.md`, and `typescript.md` contain the descriptions, with metadata tests guarding them. |

## Final root-run verification record

The rows above record implementation and focused coverage present in the tree. The following results were observed from the settled working tree, using PostgreSQL 16 and the local Redis service where applicable.

Focused results observed after the CLI supervisor finalized:

- `cargo test --lib cli::dev` — 9 passed, 0 failed, 0 ignored.
- `cargo test kernel::cli::tests --lib` — 7 passed.
- `cargo test --test blueprint_fixture_acceptance` — 2 passed.
- `cargo check --all-targets`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --all --check` — passed at that focused checkpoint.
- Worker lease-finalization regression coverage — 36 job tests passed; the PostgreSQL/Redis observability suite passed three consecutive stress runs, one post-polish focused run, and both full integration runs without the prior self-cancellation.

- [x] `cargo fmt --all -- --check` and `git diff --check` — passed before release verification and after the final polish.
- [x] `cargo check --all-targets` — passed after the final polish.
- [x] `cargo clippy --all-targets -- -D warnings` — passed in both the release gate and the post-polish full verification.
- [x] PostgreSQL 16: `FOUNDRY_TEST_POSTGRES_URL=postgres://... make test-postgres` — passed all targets, including 1,343 library tests at that checkpoint and every PostgreSQL acceptance suite.
- [x] Redis-backed focused/runtime coverage — Redis primitive tests passed 12/12, cache tests 28/28, Redis job-backend tests 5/5, and the distributed/observability paths passed in full verification.
- [x] `make api-docs` — passed after final code changes and wrote 37 module files plus the index; generated links/content were covered by the API audit and `git diff --check` passed.
- [x] `make build-metrics` — passed; the observed clean-release, artifact-size, target-size, and dependency baseline is recorded in the linked build-cost document.
- [x] `make verify` — passed before release verification and again after the required final polish, including all targets, Clippy, and both fixture families.
- [x] `make verify-release` — exited successfully; both support crates packaged and verified, and the root dry-run reported the documented unpublished-support-crate registry prerequisite.
- [x] Required final code-simplifier pass — completed across the recent worker, CLI/scaffold, and TypeScript changes; focused suites and the full `make verify` gate passed afterward.
