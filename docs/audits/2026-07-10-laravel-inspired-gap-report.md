# Foundry Laravel-Inspired Gap Report — 2026-07-10

Status: **complete — every public module and every repository-wide theme in the coverage matrix was reviewed at least once.**

Repository snapshot: `75d0444310b5e28d8f7e2969045c6274fe989312`

This report complements the earlier [security, typing, and correctness audit](2026-07-10-framework-audit.md). It does not replace that audit. This pass concentrates on product completeness, truthful contracts, production ergonomics, and Laravel-inspired developer experience.

## Scope and rules

The comparison baseline is the current [Laravel 13 documentation](https://laravel.com/docs/13.x/documentation), used as inspiration rather than a parity checklist. Foundry is an API/SPA-oriented, strongly typed Rust framework; Laravel features that exist mainly because of PHP or server-rendered Blade applications are not automatically Foundry gaps.

Accepted constraints, excluded from findings:

- PostgreSQL-only database support is acceptable.
- Raw SQL in migration bodies is acceptable.
- No recommendation in this report asks for MySQL support or a migration DSL.
- Blade/views, facades, controllers, and PHP-style global helpers are not required for Foundry's current API/SPA direction.
- Laravel's first-party package catalog is not a core-framework checklist. Billing, external search, feature flags, social login, and similar verticals should remain plugins or separate crates until the team has a real use case.

Priority labels:

- **Now** — observed correctness, security, misleading-contract, or production-reliability issue.
- **Next** — high-leverage developer experience or resilience improvement after the Now list.
- **Later** — useful only when a concrete consumer need appears.
- **No material improvement identified** — the module is coherent for its current scope; do not add features merely for parity.

No implementation changes were made during this review.

## Executive verdict

Foundry does **not** lack a framework foundation. It already covers substantially more than a typical Rust backend crate: five kernels, typed ORM/querying, auth, validation, queues, scheduler, realtime, email, notifications, storage, observability, plugins, client contracts, and consumer fixtures.

The important gaps are concentrated in four areas:

1. **Truthful runtime contracts.** Several APIs/config values claim behavior that is absent or silently becomes a no-op: cursor tokens, datatable delivery, selected notification channels, email queue/template settings, Redis cache flush, stored-file URLs, country timezones, and WebP quality.
2. **Identity and failure isolation.** Actor roles/permissions are lost across built-in session/MFA credentials; guard identity is omitted from HTTP throttles and WebSocket force-disconnect; one scheduler lock error can suppress unrelated due tasks.
3. **Boundary resilience.** Redis reconnection/timeouts, bounded direct image decoding, paginated orphan maintenance, deterministic locale/translation behavior, and logging initialization need tightening.
4. **Laravel-level DX.** Testing fakes/assertions/database isolation, a first-party outbound HTTP client, richer command I/O, and contract/OpenAPI polish would deliver more value than adding another large subsystem.

The highest-value discussion order is:

1. `AUTH-01`, `HTTP-01`, `WS-01` — principal identity and authorization state.
2. `DB-01`, `SCH-01` — pagination and scheduled-work correctness.
3. `CFG-01`, `LOG-01` — secret exposure and silent observability degradation.
4. `NOTIFY-01`, `MAIL-01` to `MAIL-04`, `DT-01`, `DT-02` — make delivery/config success truthful.
5. `STO-01`, `ATT-01`, `IMG-01`, `CACHE-01`, `REDIS-01` — storage/runtime resilience.
6. `WIRE-01`, `TEST-01`, `CLIENT-01` — contract and consumer productivity.

## Coverage matrix

The assignment was non-overlapping by top-level source module. Cross-cutting findings cite multiple modules where the contract spans them.

| Surface | Coverage | Surface | Coverage |
| --- | --- | --- | --- |
| Crate root/package | Reviewed | `app_enum` | Reviewed |
| `attachments` | Reviewed | `audit` | Reviewed |
| `auth` | Reviewed | `cache` | Reviewed |
| `cli` | Reviewed | `config` | Reviewed |
| `contract` | Reviewed | `countries` | Reviewed |
| `database` | Reviewed | `datatable` | Reviewed |
| `email` | Reviewed | `events` | Reviewed |
| `foundation` | Reviewed | `http` | Reviewed |
| `i18n` | Reviewed | `imaging` | Reviewed |
| `jobs` | Reviewed | `kernel` | Reviewed |
| `logging` | Reviewed | `metadata` | Reviewed |
| `notifications` | Reviewed | `openapi` | Reviewed |
| `plugin` | Reviewed | `public` / `prelude` | Reviewed |
| `redis` | Reviewed | `scheduler` | Reviewed |
| `settings` | Reviewed | `storage` | Reviewed |
| `support` | Reviewed | `testing` | Reviewed |
| `translations` | Reviewed | `typescript` | Reviewed |
| `validation` | Reviewed | `websocket` | Reviewed |
| Proc macros / `foundry-build` | Reviewed | Docs / examples / fixtures / release | Reviewed |
| Missing-theme boundary | Reviewed | Laravel parity exclusions | Reviewed |

## Per-module report

### Crate root and package shape

Current position: the crate exposes a coherent all-in-one framework, with only OpenTelemetry meaningfully feature-gated. This is reasonable before 1.0.

- **Now — `ROOT-01`: remove or implement the empty `webauthn` feature.** `Cargo.toml:83-85` advertises the feature, while `src/auth/mfa/mod.rs:30-31` exposes only an empty module. A feature flag is a public promise; leaving it empty is more confusing than omitting it.
- **Later — `ROOT-02`: feature-gate heavy optional integrations only after measuring consumer build cost.** Email providers, XLSX, imaging, S3, WebSocket, and client generation are unconditional dependencies (`Cargo.toml:29-76`). Do not split the crate speculatively, but establish compile-time/binary-size measurements before the public feature set freezes.

### `public` / `prelude`

The blessed import surface is broad but coherent, and `tests/public_api_acceptance.rs` exercises real consumer composition.

**No material improvement identified.** Keep the public/prelude layers stable and continue using compile fixtures for breaking-surface changes.

### `app_enum`

The module already provides typed stable keys, aliases, localized labels, database conversion, OpenAPI, and inventory-based TypeScript discovery (`foundry-macros/src/app_enum.rs:913-930`; `foundry-macros/src/typescript.rs:48-60`).

**No material improvement identified.** The old blueprint note about missing enum discovery is obsolete; current inventory registration already covers it.

### `foundation`

The two-phase provider lifecycle, strict container registration, panic isolation, managed tasks, and plugin rollback are strong.

- **Next — `FND-01`: distinguish a missing `.env` from a malformed/unreadable one.** Both bootstrap paths discard every dotenv error with `.ok()` (`src/foundation/app.rs:952`, `src/foundation/app.rs:1362`). Missing optional files may remain non-fatal, but other errors should warn or fail.
- **Later — `FND-02`: explicit scoped/test replacement bindings.** The no-overwrite container default is good, but a clearly test-only replacement API would make fakes easier without weakening production rules (`src/foundation/container.rs:28-99`).

### `kernel`

All five kernels have clear runtime boundaries and strong standard shutdown paths.

- **Next — `KERNEL-01`: expose full lifecycle shutdown for consumers of built kernels.** `run_*` owns cleanup, but public `build_*_kernel` callers cannot invoke the crate-private full app shutdown (`src/foundation/app.rs:366`, `src/foundation/app.rs:851-902`). Custom server/process integration can therefore bypass managed-task and plugin cleanup.

No other generic kernel improvement was identified.

### `http`

Typed routes/scopes, inherited guards and permissions, middleware groups, signed URLs, CSRF, CORS, rate limiting, trusted proxies, and edge tests are substantial.

- **Now — `HTTP-01`: include `GuardId` in actor rate-limit keys.** Both pre/post-auth paths use `actor:{actor.id}` (`src/http/middleware.rs:1253`, `src/http/middleware.rs:1348`), while actor identity is `(guard, id)` (`src/auth/mod.rs:103`). Equal IDs under different guards currently share quota.
- **Next — `HTTP-02`: typed route-model binding.** Consumer guidance still extracts `Path<String>`, queries manually, and emits 404 manually (`docs/guides/routes-and-middleware.md:884`). A `ModelPath<M>` extractor could provide Laravel-style binding while preserving Foundry's typed primary keys.
- **Later — `HTTP-03`: general flash/session state only if server-rendered flows become a goal.** Auth sessions are credential records, not a request session bag. For the current API/SPA direction this is not a core gap.

### `cli`

Command IDs, duplicate handling, Clap integration, and panic isolation are correct.

**No material correctness improvement identified.**

- **Next — `CLI-01`: capturable command I/O and exit status.** Handlers expose raw `ArgMatches` and return only `Result<()>`; guides use direct `println!` (`src/cli/mod.rs:41-82`; `docs/guides/cli-commands.md:12`). Add an output abstraction, confirmation/prompt/progress helpers, and typed exit codes so commands can be tested cleanly.

### `scheduler`

Timezone-aware cron, intervals, leadership, overlap leases, renewal, cancellation, hooks, and bounded draining are well developed.

- **Now — `SCH-01`: isolate overlap-lock errors per schedule.** The global `last_tick` advances before iteration, then a lock backend error returns from the whole tick (`src/kernel/scheduler.rs:116-121`, `src/kernel/scheduler.rs:157-183`). The next-tick retry message is inaccurate because the cron cursor already advanced (`src/kernel/scheduler.rs:260-267`), so later due tasks and that occurrence can be lost.

### `jobs`

Leases, heartbeats, retries, dead letters, middleware, uniqueness, rate limits, batches, chains, history, and graceful shutdown are already unusually complete.

- **Now — `JOB-01`: fix non-compiling dispatch guide examples.** `dispatch` and `dispatch_later` are async (`src/jobs/mod.rs:343-355`), while `docs/guides/background-processing.md:52` omits `.await`.
- **Next — `JOB-02`: make `track_history` independent or explicitly dual-gated.** Persistent history silently requires both `track_history` and diagnostics capture (`src/jobs/mod.rs:2115-2129`), but the guide says `track_history = true` is sufficient.
- **Later — `JOB-03`: typed delay overloads.** Add `DateTime` / `Duration` forms alongside raw epoch milliseconds.

### `websocket`

Guarded channels, dynamic authorization, credential revalidation, presence, replay, bounded queues, heartbeat, origin checks, and multi-instance pub/sub form a strong realtime kernel.

- **Now — `WS-01`: include `GuardId` in force-disconnect messages.** Public `disconnect_user` publishes only actor ID and each instance removes every matching ID (`src/websocket/mod.rs:333`; `src/kernel/websocket.rs:2178`), despite connection limits correctly keying `(GuardId, actor_id)`.
- **Next — `WS-02`: actively close sockets during server shutdown.** Axum receives the shutdown signal, but existing socket loops wait for peers before presence/lifecycle cleanup (`src/kernel/websocket.rs:104`, `src/kernel/websocket.rs:1206-1235`). Broadcast a Close/cancellation signal to active sockets.
- **Later — `WS-03`: durable session resume and binary frames.** These are explicitly deferred; bounded replay is adequate until a real product requires more.

### `config`

Split TOML merging, typed sections, environment overlays, config publishing, and individual secret-redacted config types are good foundations.

- **Now — `CFG-01`: make `ConfigRepository` Debug opaque/redacted.** It derives raw `Debug` over the entire TOML tree (`src/config/mod.rs:51-54`), bypassing secret-aware `Debug` implementations for app, Redis, database, and crypt config.
- **Now — `CFG-02`: separate deployment security tier from a free-form environment label.** Unknown labels become `Custom`, and only exact `production`/`staging` enable production-like checks (`src/config/mod.rs:66-95`). `eu-prod` therefore disables protections such as production WebSocket origin behavior.
- **Next — `CFG-03`: diagnose unknown keys and ambient unprefixed overlays.** Most structs use defaults without deny-unknown-field behavior, and unprefixed `__` variables remain accepted (`src/config/mod.rs:126`, `src/config/mod.rs:691`, `src/config/mod.rs:1370`). `doctor --strict` should report both before a breaking default change.

### `database`

The AST, typed columns/keys, models, relations, eager loading, projections, aggregates, streaming, lifecycle hooks, read replicas, and PostgreSQL lifecycle are strong. Relations and lifecycle need no material expansion for current scope.

- **Now — `DB-01`: repair cursor pagination as one contract.** It permits `after` and `before` together, can combine conflicting pre-existing order, accepts non-unique cursor columns without a tie-breaker, and always returns `next: None` / `prev: None` (`src/database/query.rs:252-275`, `src/database/query.rs:2716-2765`). Reject conflicting directions, own a deterministic order, append the primary key tie-breaker, and populate cursors automatically.
- **Now — `DB-02`: correct status documentation.** `docs/query-blueprint-status.md:120-126` claims `ProjectionQuery::cursor_paginate`, but only the model method exists.
- **Next — `DB-03`: make model scaffold table naming explicit.** Mechanical `format!("{snake}s")` generates names such as `categorys` (`src/database/scaffold.rs:357-368`). Add `--table` and optionally conservative inflection.
- **Later — `DB-04`: implement or remove inert `without_defaults()`.** It toggles `skip_defaults`, but no query path reads that flag (`src/database/query.rs:1983-2035`).

### `datatable`

Typed model/projection adapters, bounded JSON/XLSX output, explicit filter/sort targets, relation filters, actor snapshots, and a shared query pipeline are solid.

- **Now — `DT-01`: propagate locale and timezone end to end.** `DatatableContext::new` always sets no locale, dispatch stores `locale: None`, and the worker ignores both payload locale/timezone (`src/datatable/context.rs:20-34`; `src/datatable/export_job.rs:16-23`, `src/datatable/export_job.rs:50-57`, `src/datatable/export_job.rs:100-109`).
- **Now — `DT-02`: fail queued delivery when no delivery service exists.** Missing registration falls back to `NoopExportDelivery`, which silently returns success (`src/datatable/export_job.rs:71-79`; `src/datatable/export.rs:21-29`).
- **Later — `DT-03`: chunk/stream very large queued exports.** The 50,000-row default cap mitigates the current full materialization, so this is not urgent.

### `storage`

Multi-disk support, custom adapters, path normalization, local symlink defense, upload limits/temp ownership, S3 metadata, and signed URLs are good.

- **Now — `STO-01`: reconcile `StoredFile.url` and storage guide contracts.** Built-in local/S3 writes always return `url: None`, while the type and guide imply a usable URL; the guide also omits `.await` on async URL calls and references nonexistent `MultipartForm::from_multipart` (`src/storage/stored_file.rs:1-9`; `src/storage/local.rs:188-216`; `src/storage/s3.rs:90-110`; `docs/guides/storage-and-imaging.md:84-103`, `docs/guides/storage-and-imaging.md:304-321`).
- **Next — `STO-02`: streaming S3 file I/O.** `put_file` reads the complete temp file and `put_bytes` clones it; the adapter has no streaming surface (`src/storage/s3.rs:72-110`; `src/storage/adapter.rs:23-53`).
- **Next — `STO-03`: support the AWS/default credential chain.** Static access key and secret are mandatory (`src/storage/config.rs:119-143`; `src/storage/s3.rs:26-31`), blocking IAM/workload identity.
- **Later — `STO-04`: storage fake.** Implement through the cross-service testing-fake design, not as a separate ad hoc pattern.

### `attachments`

Collection specs, localization, image policies, hooks, eager/lazy batching, cleanup, and orphan tooling are feature-rich.

- **Now — `ATT-01`: paginate/rotate orphan scans.** Every recurring run examines only the same first `limit` objects (`src/attachments/orphans.rs:142-185`; `src/storage/local.rs:340-342`; `src/storage/s3.rs:197-220`). Later orphans can starve forever.
- **Next — `ATT-02`: enforce `.single()` cardinality under concurrency.** Replacement performs separate read/insert/delete operations and the table has no matching unique constraint (`src/attachments/mod.rs:1065-1113`; `database/migrations/000000000005_create_attachments.rs:32-35`). Use a lock plus transaction or schema-backed invariant.
- **Later — `ATT-03`: expose reorder operations or remove the implied ordering field.** Inserts use `sort_order = 0`, reads order by it, and no reorder API exists.

### `imaging`

The transform API and format support are clear; attachment-driven processing is correctly bounded and moved to blocking work.

- **Now — `IMG-01`: make bounded decoding the public default.** `ImageProcessor::open` / `from_bytes` decode immediately without the safety limits used by attachments (`src/imaging/mod.rs:132-167`; `src/attachments/mod.rs:945-988`). The guide currently shows direct untrusted processing in async handlers.
- **Now — `IMG-02`: make WebP quality truthful.** `quality()` promises JPEG/WebP control, but the WebP branch explicitly ignores it (`src/imaging/mod.rs:220-225`, `src/imaging/mod.rs:335-339`). Honor quality or narrow the contract.
- **Next — `IMG-03`: public blocking-safe helper.** Give direct imaging the same runtime-safe execution path as attachment processing.

### `cache`

Typed values, strict/fail-open modes, key validation, local single-flight, and optional distributed stampede protection are strong.

- **Now — `CACHE-01`: implement namespace-safe Redis `flush()`.** The public manager says flush clears the cache, Redis is the default, but Redis store always returns unsupported (`src/cache/mod.rs:118-123`; `src/cache/redis_store.rs:43-47`; `src/config/mod.rs:1120-1134`). A namespace generation/version strategy is safer than global flush.
- **Next — `CACHE-02`: cache tags.** The blueprint and Laravel-style invalidation story reference them, but the runtime surface has no tags (`blueprints/11-framework-improvements.md:303-365`; `src/cache/mod.rs:57-124`).

### `redis`

Namespaced typed keys/channels and common string/hash/set/counter commands are useful.

- **Now — `REDIS-01`: use reconnecting connections and bounded timeouts.** One unresettable cached `MultiplexedConnection` is cloned forever, and config exposes neither connect nor command timeouts (`src/redis/mod.rs:87-117`, `src/redis/mod.rs:148-167`; `src/config/mod.rs:364-389`).
- **Next — `REDIS-02`: deliberate low-level escape hatch.** The guide advertises use cases such as leaderboards, but there is no sorted set, list, stream, pipeline, transaction, script, or namespaced raw-command API (`src/redis/mod.rs:170-346`; `docs/guides/caching-and-redis.md:255-269`).

### `settings`

Admin metadata, typed serde reads, grouping, public filtering, prefix escaping, and unique keys are appropriate.

- **Now — `SET-01`: make missing updates observable.** `Setting::set` discards affected rows and always returns success (`src/settings/mod.rs:239-249`). Return a count/bool or not-found error.
- **Next — `SET-02`: reject unknown stored setting types.** Hydration silently maps corruption to `Text` (`src/settings/mod.rs:391-400`), inconsistent with the module's explicit typed-value drift errors.

### `metadata`

The polymorphic JSON store is small, typed on read, and uses atomic upsert.

No immediate CRUD correctness defect was found.

- **Next — `META-01`: eager/bulk metadata loading.** Per-model reads naturally produce N+1 queries; follow the attachment/translation batching pattern (`src/metadata/mod.rs:66-141`; `src/database/query.rs:2173-2212`).
- **Next — `META-02`: lifecycle cleanup or orphan audit.** Polymorphic rows cannot use owner foreign keys and currently have only per-key deletion.

### `countries`

The bundled 250-country upsert, ISO normalization, status filtering, and preservation of app-managed fields are useful.

- **Now — `COUNTRY-01`: populate or remove advertised timezones.** Runtime exposes the field, but all bundled records contain empty timezone arrays (`src/countries/mod.rs:60-63`; `src/countries/seed.json:1`).
- **Next — `COUNTRY-02`: reject unknown persisted status.** Every unrecognized value becomes `Disabled` (`src/countries/mod.rs:25-29`, `src/countries/mod.rs:373`).
- **Later — `COUNTRY-03`: strongly type JSON collections.** Currencies, calling suffixes, TLDs, and timezones are raw JSON despite Foundry's typed positioning.

### `auth`

Typed guards/policies/permissions, hashed tokens, atomic refresh rotation, session indexes, TOTP, recovery codes, password reset, email verification, and panic isolation are extensive.

- **Now — `AUTH-01`: introduce one guard-level actor hydrator.** Sessions persist only ID/guard/remember and reconstruct an actor with empty roles/permissions; HTTP checks permissions before model resolution (`src/auth/session.rs:19-25`, `src/auth/session.rs:60-64`, `src/auth/session.rs:116-127`; `src/http/mod.rs:1780-1783`). MFA full-token exchange likewise drops abilities (`src/auth/mfa/mod.rs:159-165`, `src/auth/token.rs:169-176`, `src/auth/mfa/mod.rs:564-574`). Session, token, MFA, HTTP, and WebSocket auth need one SSOT for current actor authorization state.
- **Now — `AUTH-02`: fix or rename lockout threshold semantics.** `max_failures = 5` is reduced to four with `saturating_sub(1)` (`src/auth/lockout.rs:118-156`), and the test called “locks on fifth attempt” records only four failures (`src/auth/lockout.rs:283-305`). The configured number must have an unambiguous meaning.
- **Now — `AUTH-03`: remove the empty WebAuthn promise.** Same finding as `ROOT-01`; passkeys can be implemented later, but an empty public feature should not remain.

Social auth, OAuth server, magic links, and team features are not core gaps until the team requests them; the plugin/guard interfaces are the right extension boundary.

### `validation`

Manual/derive validation, async custom rules, typed scalars/collections, multipart cleanup, file sniffing, i18n, and client metadata are strong.

- **Now — `VAL-01`: preserve a custom rule's returned message.** `ValidationError` requires a message, but named-rule execution keeps only its code and resolves a different fallback (`src/validation/types.rs:16-27`; `src/validation/executor.rs:242-260`).
- **Now — `VAL-02`: correct placeholder syntax in guides.** Runtime interpolation uses `{{attribute}}`, while one guide shows `:attribute` and another `{{field}}` (`src/validation/validator.rs:156-200`; `docs/guides/validation.md:270-291`; `docs/guides/i18n.md:210-230`).
- **Next — `VAL-03`: small conditional/presence rule set.** `required_if/unless/with`, `present/sometimes/prohibited`, typed boolean, and collection distinct would eliminate repetitive `after(...)` hooks without trying to clone every Laravel rule.

### `events`

Typed listeners, sequential semantics, context propagation, panic isolation, and job/WebSocket adapters are coherent.

- **Next — `EVT-01`: enforce `Event::ID` uniqueness.** Registration is keyed only by Rust `TypeId`; two event types can share one semantic ID (`src/events/mod.rs:18-20`, `src/events/mod.rs:139-170`; `src/foundation/provider.rs:128-135`).
- **Next — `EVT-02`: first-class after-commit event dispatch.** Transactions have job/notification helpers but only a generic callback for events (`src/foundation/app.rs:406-437`; `src/events/mod.rs:191-215`).

No other material event-bus expansion is needed now.

### `notifications`

Typed/frozen channel registration, immediate aggregate errors, panic isolation, queued retry/dead-letter integration, after-commit dispatch, and actor-scoped broadcasts are good.

- **Now — `NOTIFY-01`: selected built-in channels must deliver or fail.** Email/database/broadcast channels return success when their route/renderer/payload is absent, and queued delivery skips missing payloads (`src/notifications/channel.rs:26-95`; `src/notifications/mod.rs:284-331`; `src/notifications/job.rs:72-96`).
- **Next — `NOTIFY-02`: avoid replaying successful channels on retry.** One queued job stops at the first failure, so a retry can duplicate earlier channels (`src/notifications/job.rs:52-61`; `docs/guides/email-and-notifications.md:275-278`). Use per-channel jobs or persisted channel completion/idempotency.
- **Next — `NOTIFY-03`: database-notification repository.** The schema has `read_at`, but Foundry only inserts rows and provides no unread/read/mark-read API (`database/migrations/000000000003_create_notifications.rs:11-26`; `src/notifications/mod.rs:68-83`).

### `email`

Multiple providers, custom drivers, mailer selection, address/header validation, attachment limits, safe template paths, queue integration, and provider-error redaction are substantial.

- **Now — `MAIL-01`: honor `[email].queue`.** Config parses it, but queued mail uses ordinary dispatch and the job has no queue override (`src/email/config.rs:7-31`; `src/email/mailer.rs:44-61`; `src/email/job.rs:10-27`).
- **Now — `MAIL-02`: honor or remove `[email].template_path`.** Every caller must separately pass a template path (`src/email/config.rs:7-31`; `src/email/message.rs:81-102`).
- **Now — `MAIL-03`: make SMTP `encryption = "none"` actually plaintext.** It still calls `starttls_relay` (`src/email/smtp.rs:21-35`).
- **Now — `MAIL-04`: escape HTML template variables by default.** Current replacement inserts values verbatim (`src/email/template.rs:138-178`). Provide explicit raw syntax for trusted markup.

Provider failover/round-robin and mail preview tooling are useful later, but not ahead of these contract fixes and testing fakes.

### `i18n`

Immutable startup catalogs, nested JSON flattening, fallback chains, weighted language parsing, regional fallback, request extractors, and validation integration are strong.

- **Next — `I18N-01`: case-insensitive BCP 47 matching.** Catalog/header keys retain spelling and lookup is exact (`src/i18n/mod.rs:87-93`, `src/i18n/mod.rs:186-204`, `src/i18n/mod.rs:339-354`).
- **Next — `I18N-02`: deterministic catalog file order or duplicate errors.** Filesystem iteration is unsorted while duplicate keys use last-file-wins (`src/i18n/mod.rs:82-123`, `src/i18n/mod.rs:231-276`).
- **Later — `I18N-03`: pluralization/choice support.** Add only when real localized product copy needs it.

### `translations`

Transaction-aware writes, batching, typed joins, cache invalidation, locale maps, and deletion helpers are strong.

- **Now — `TRANS-01`: deterministic final fallback.** `HashMap::values().next()` chooses an arbitrary translation, while the guide promises the first available locale (`src/translations/mod.rs:175-189`; `docs/guides/model-extensions.md:476-482`).
- **Next — `TRANS-02`: support Foundry's non-UUID model keys or declare the restriction.** The trait parses IDs as UUID and the table column is UUID (`src/translations/mod.rs:198-237`, `src/translations/mod.rs:567-573`; `database/migrations/000000000007_create_model_translations.rs:11-17`).

### `logging`

Structured output, request/trace context, rotation, metrics, probes, bounded diagnostics, guarded dashboards, error reporters, and optional OTLP provide excellent coverage.

- **Now — `LOG-01`: do not report successful logging setup after subscriber/exporter failure.** Every `try_init()` result is discarded, and OTLP construction converts errors to `None` (`src/logging/mod.rs:84-131`, `src/logging/mod.rs:161-202`, `src/logging/mod.rs:222-238`). Explicit tracing should fail bootstrap or emit a clear degraded state.
- **Next — `LOG-02`: validate inbound request IDs and generate globally unique defaults.** Arbitrary client values are mirrored with no bound; generated IDs are process-local counters (`src/logging/middleware.rs:21-31`, `src/logging/middleware.rs:99-103`; `src/logging/request_id.rs:67-70`).
- **Next — `LOG-03`: non-blocking bounded file writer.** A global mutex and synchronous writes run on runtime threads (`src/logging/file_writer.rs:93-125`). Expose backpressure/drop metrics.

### `audit`

Transactional lifecycle capture, actor/request/area attribution, recursive redaction, opt-outs, and a typed query model are strong.

**No material correctness or security improvement identified.**

- **Next — `AUDIT-01`: optional manual/scoped audit API for jobs, scheduler, CLI, and domain actions.** Current capture requires an HTTP audit area (`src/audit/mod.rs:161-181`, `src/audit/mod.rs:239-245`; `docs/guides/database.md:782-785`).
- **Later — `AUDIT-02`: built-in retention/pruning.** Current retention is delegated to application/plugin scheduling.

### `contract`

The normalized manifest is the correct architectural direction and already unifies schemas, validation, HTTP actions, errors, and realtime descriptors.

- **Next — `CONTRACT-01`: explicit business action names.** `action_name` is mechanically derived from route ID (`src/contract/mod.rs:155-174`), coupling SDK naming to routing. Make it explicit and keep route ID as transport metadata.
- **Next — `CONTRACT-02`: typed parameter and error metadata.** The HTTP contract records only path-param names/body kind, and errors are global rather than per-action (`src/contract/mod.rs:139-152`, `src/contract/mod.rs:228-280`). Add parameter location/schema/requiredness and action-specific error responses before extending generators.
- **Next — `CONTRACT-03`: typed WebSocket handler decoding.** Descriptors declare typed payloads, but handlers still receive raw `serde_json::Value` (`src/websocket/mod.rs:252-262`, `src/websocket/mod.rs:500-549`).
- **Later — `CONTRACT-04`: other transports.** Command/workflow adapters should wait for actual generator consumers.

### `openapi`

OpenAPI 3.1 generation through the contract manifest and structural schema naming are good foundations.

- **Next — `OPENAPI-01`: emit standard operation and security metadata.** Current output has no `operationId`, `securitySchemes`, or `security`; guard/permissions appear only as `x-foundry-*` extensions (`src/openapi/spec.rs:37-60`, `src/openapi/spec.rs:123-128`).
- **Next — `OPENAPI-02`: typed query/header/cookie/path parameters and response descriptions.** Path parameters are always strings, request bodies support only JSON/multipart, and response descriptions are empty (`src/openapi/spec.rs:62-115`). This should be solved in `contract` first, then rendered here.
- **Next — `OPENAPI-03`: expose standard error responses per action.** The manifest knows common errors, but the generator does not attach them to operations.

### `typescript`

Inventory discovery, collision-safe writes, DTOs/AppEnums, validation metadata, route manifests, a pure SDK, errors, realtime metadata, and the contract JSON are already broad.

- **Next — `TS-01`: make the contract manifest the sole generator boundary.** `export_all_with_context` still creates compatibility route helpers directly from route metadata while SDK actions come from the manifest (`src/typescript/mod.rs:3127-3140`, `src/typescript/mod.rs:3187-3238`). Move the form helper behind an optional adapter generated from the manifest.
- **Later — `TS-02`: React/Vue/Flutter adapters.** The pure SDK should remain core. Add adapters only for the frontends the team actually uses; do not put UI state into the core transport layer.

### `plugin`

Compile-time discovery, dependency/SemVer validation, direct registration parity, lifecycle rollback/shutdown, assets, scaffolds, contribution metadata, and consumer fixtures are strong.

**No material core improvement identified.**

- **Later — `PLUGIN-01`: reusable plugin test harness and author template.** This is higher value than runtime dynamic-library loading. Crates.io/Cargo should remain package discovery; Foundry should not invent a second dependency manager.

### `support`

Typed semantic IDs, immutable temporal types, collections, Argon2id, authenticated encryption, tokens, sanitization, blocking isolation, and distributed locks are useful and coherent.

- **Next — `SUP-01`: password `needs_rehash`.** `HashManager` exposes only hash/check (`src/support/hash.rs:31-87`). A work-factor migration check is a small, high-value analogue of [Laravel's `needsRehash`](https://laravel.com/docs/13.x/hashing#determining-if-a-password-needs-to-be-rehashed).
- **Next — `SUP-02`: graceful encryption-key rotation.** `CryptConfig` accepts one key and decryption tries only that key (`src/config/mod.rs:1060-1071`; `src/support/crypt.rs:24-103`). Support previous decrypt-only keys, similar to [Laravel's key rotation](https://laravel.com/docs/13.x/encryption#gracefully-rotating-encryption-keys).

No need was identified for cloning Laravel's large string/helper surface; Rust's standard library and focused crates are the better boundary.

### `testing`

`TestApp` reuses production bootstrap, sends in-process requests, supports graceful shutdown, provides typed model factories, and protects destructive database cleanup.

- **Next — `TEST-01`: service fakes/spies as one coherent testing layer.** Add event, job, mail, notification, storage, outbound HTTP, and clock fakes with typed assertions. Laravel's [mocking/fake pattern](https://laravel.com/docs/13.x/mocking) is high leverage because it tests orchestration without executing side effects.
- **Next — `TEST-02`: rich response assertions.** `TestResponse` only exposes status/header/json/text/bytes (`src/testing/client.rs:285-320`). Add fluent status, JSON path/fragment/shape, validation-error, header, redirect, and download assertions, informed by [Laravel HTTP tests](https://laravel.com/docs/13.x/http-tests).
- **Next — `TEST-03`: PostgreSQL test isolation and database assertions.** Provide per-test transaction/rollback or isolated-schema helpers, factory states/sequences/relations, and typed `assert_database_has/count/missing`, analogous to [Laravel database testing](https://laravel.com/docs/13.x/database-testing).
- **Next — `TEST-04`: auth/time helpers and command testing.** `acting_as`, token/session setup, frozen clock, and capturable CLI output should compose with the same fake registry.

Testing is the largest Laravel-inspired DX gap in the current framework.

## Cross-cutting tooling and missing themes

### Proc macros and wire contracts

- **Now — `WIRE-01`: one shared wire-name model across serde, validation, multipart, OpenAPI, and TypeScript.** `ApiSchema` and `Validate` use Rust identifiers directly and do not parse `serde(rename)` / `rename_all`; multipart matches those same identifiers (`foundry-macros/src/openapi.rs:43-49`; `foundry-macros/src/validate.rs:241-275`; `foundry-macros/src/validate.rs:505-542`; `foundry-macros/src/validate.rs:1743-1750`). A DTO that serializes renamed fields can therefore validate, document, parse multipart, and generate clients under different names. Define one macro-level field-name resolver and add compile/contract fixtures before changing behavior.
- **Next — `WIRE-02`: decouple `ApiSchema` from mandatory TypeScript registration.** The derive always calls `expand_with_ts` (`foundry-macros/src/lib.rs:37-40`, `foundry-macros/src/lib.rs:57-81`). Schema generation and TypeScript export should be composable concerns even if the default convenience derive enables both.

`foundry-build` itself is focused and sufficient for PostgreSQL migration/seeder discovery under the accepted raw-SQL constraint. No migration DSL is recommended.

### First-party outbound HTTP client

- **Next — `CLIENT-01`: add a Foundry HTTP client abstraction.** `reqwest` is used internally by mail providers, but there is no consumer-facing module in `src/lib.rs:57-92`. The useful scope is not a cosmetic wrapper: shared timeouts, base URLs, retry/backoff, tracing/redaction, concurrency/pooling, typed errors, and fake sequences/request assertions. This is one of Laravel's most transferable facilities; see the [Laravel HTTP client](https://laravel.com/docs/13.x/http-client).

Do not build a second low-level networking stack. Wrap/configure `reqwest` and keep raw-client escape hatches.

### Scaffolding and development workflow

- **Next — `SCAFFOLD-01`: widen scaffolding only around repeated real patterns.** Existing commands cover migration, seeder, model, job, and command (`README.md:525-559`). Likely high-value additions are request/DTO, policy, event/listener, notification/mail, datatable, plugin, and test. Avoid generators for code that remains simpler to write directly.
- **Later — `CLI-02`: one development orchestration command.** A `dev` command could run selected HTTP/worker/scheduler/WebSocket processes with clear logs and restart behavior, but only after process bootstraps and logging behavior are stable.

### Documentation and release hygiene

- **Now — `DOC-01`: add the promised license file.** `Cargo.toml` declares MIT and README links `LICENSE`, but the file is absent (`Cargo.toml:5`; `README.md:10`).
- **Now — `DOC-02`: restore or remove the missing public API contract.** README and generated API index link `docs/api/public-api-contract.md`, which does not exist (`README.md:609`; `docs/api/index.md:8`; generator source `src/config/api_docs.rs:196`).
- **Now — `DOC-03`: compile-check consumer guide snippets.** Confirmed drift includes job `.await`, cursor promises, storage URL/async calls, nonexistent multipart construction, validation placeholders, and nonexistent projection cursor pagination. Extract representative snippets into fixture tests rather than relying on manual review.
- **Next — `DOC-04`: add a testing guide.** `docs/guides/README.md` claims all major modules are documented but has no testing guide. Document the current harness now and expand it alongside `TEST-01` to `TEST-04`.
- **Next — `DOC-05`: fill generated descriptions for `public`, `settings`, and `typescript`.** They currently appear blank because `src/config/api_docs_metadata.rs:5-42` omits them.

## Deliberate non-gaps and conditional themes

These were reviewed and should **not** be added merely because Laravel has them:

| Theme | Recommendation |
| --- | --- |
| MySQL/SQLite/Mongo drivers | Not a gap under the team's PostgreSQL-only constraint. |
| Migration schema DSL | Not a gap; raw SQL is accepted and PostgreSQL-specific migrations are often clearer. |
| Blade/views/controllers/facades | Not a gap for an API/SPA framework. Revisit only if server-rendered HTML becomes a product goal. |
| General session bag/flash/old input | Conditional on server-rendered redirect/form flows; auth sessions already solve a different problem. |
| External search abstraction | PostgreSQL full-text search already exists. Add a plugin only when an external engine is selected. |
| Feature flags | Good plugin candidate; not core without a team use case. |
| Billing/social auth/admin UI | Plugin/application concerns, not core-framework omissions. |
| SQS/database queues and non-S3 object stores | The traits permit expansion. Add drivers when infrastructure requires them, not for parity. |
| Dynamic plugin loading | Rust/Cargo compile-time plugins are the correct safety and deployment model. |
| React/Vue/Flutter UI state generation | Keep the core SDK headless; adapters should be optional and consumer-driven. |
| AI SDK/MCP parity | Not relevant to Foundry's stated framework contract today. |

## Suggested discussion batches

To avoid a single oversized implementation effort, discuss the report in these batches:

1. **Security/identity:** `AUTH-01..03`, `HTTP-01`, `WS-01`, `CFG-01..02`, `MAIL-04`.
2. **Truthful contracts:** `DB-01..02`, `DT-01..02`, `STO-01`, `IMG-02`, `CACHE-01`, `COUNTRY-01`, `NOTIFY-01`, `MAIL-01..03`.
3. **Runtime resilience:** `SCH-01`, `REDIS-01`, `ATT-01..02`, `IMG-01..03`, `LOG-01..03`, `I18N-01..02`, `TRANS-01`.
4. **Contract SSOT:** `WIRE-01..02`, `CONTRACT-01..03`, `OPENAPI-01..03`, `TS-01`.
5. **Consumer productivity:** `TEST-01..04`, `CLIENT-01`, `SCAFFOLD-01`, `CLI-01..02`, `DOC-01..05`.
6. **Optional backlog:** every item marked Later plus plugin candidates from the deliberate non-gaps table.

The report is complete when used as a discussion inventory; it is intentionally not an instruction to implement every listed item.
