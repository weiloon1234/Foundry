# Foundry Framework Re-audit — 2026-07-10

Status: **complete — every module/theme was reviewed at least once, accepted repairs are integrated, and the final repository-wide verification passes.**

## Scope and method

This audit reviewed Foundry as a Laravel-inspired application framework, with extra weight on secure defaults, semantic typing, Rust-to-client contract consistency, and consumer developer experience. The review treated consumer application and plugin code as trusted, while HTTP/WebSocket clients, uploads, queue payloads, database data, request metadata, and external provider responses were treated as untrusted.

The work combined:

- a complete source/module inventory and trust-boundary map;
- source-level review of every module/theme listed below, including public exports, macros, generated artifacts, fixtures, examples, and guides;
- concrete attack-path tracing for auth, HTTP, WebSocket, scheduler/lock, storage, notification, database, plugin, and code-generation boundaries;
- API/typing review against Foundry's semantic-ID and single-source-of-truth conventions;
- focused unit, acceptance, compile-fail, and fixture-oriented validation while repairs were integrated; and
- a current OSV/RustSec query of the active Cargo dependency graph.

The pre-change baseline `make verify` passed, covering formatting, the then-current test suite, Clippy, and both consumer fixture families. The final post-audit `make verify` also passes after code, documentation, and lockfile reconciliation.

## Coverage matrix

Every row received at least one completed source-level review. “Reviewed” is the coverage label; all accepted repairs also passed the final repository-wide gate described below.

| Module/theme | Security, typing, and DX lenses checked | Coverage |
| --- | --- | --- |
| Foundation, container, providers, background tasks | Bootstrap errors, service ownership, plugin/runtime shutdown, shared route preparation | Reviewed |
| HTTP kernel and routing | Route inheritance, guards, middleware order/groups, proxies, bind behavior, runtime/manifest consistency | Reviewed |
| CLI kernel | Parse outcomes, help/version behavior, registration errors, consumer ergonomics | Reviewed |
| Scheduler kernel | Cron/timezone semantics, leadership, overlap locks, renewal, cancellation, shutdown | Reviewed |
| Worker and jobs | Payload trust, leases, retries/dead letters, middleware, failure propagation | Reviewed |
| WebSocket kernel | Upgrade/origin boundary, auth/MFA, cached credentials, connection limits, channels, presence, ACK errors | Reviewed |
| Configuration, publishing, doctor, i18n | Secret/debug exposure, malformed config, generated paths, environment overlays, diagnostics | Reviewed |
| Authentication and authorization | Guards, sessions, tokens, MFA enrollment/exchange/refresh, revocation, permissions | Reviewed |
| Database, models, queries, relations, lifecycle | SQL construction, typed keys/columns, transaction atomicity, statement state, relation typing | Reviewed |
| Validation and multipart | Runtime/derive/TypeScript parity, nullable semantics, collections, temp-file ownership | Reviewed |
| Datatable and export | Typed filters, injection boundaries, authorization, spreadsheet/export behavior | Reviewed |
| Storage, uploads, attachments, imaging | Traversal/symlinks, temp retention, MIME/size boundaries, S3 metadata and visibility | Reviewed |
| Events, scheduler, notifications | Registration IDs, job boundaries, delivery failures, private broadcast rooms | Reviewed |
| Email and providers | Templates, attachments, provider errors, header/path/secret boundaries | Reviewed |
| Logging, audit, observability | Recursive redaction, public diagnostics, internal error exposure | Reviewed |
| Redis, cache, distributed locks | Key ownership, atomic acquire/renew/release, failure behavior | Reviewed |
| Plugins, assets, scaffolds | Dependency/lifecycle ordering, rollback, generated filesystem writes | Reviewed |
| Settings, metadata, app enums, countries/translations | Stored-value typing, aliases, parsing, secret expectations, locale identifiers | Reviewed |
| Contracts, OpenAPI, TypeScript | Rust-to-client SSOT, schemas, nullability, route execution, generated-file identity/collisions | Reviewed |
| Support primitives | Crypto/debug behavior, semantic IDs, time, filenames, collections, locks | Reviewed |
| Testing utilities | Public builder/request types, production-builder reuse, managed shutdown, database-wipe guard | Reviewed |
| Proc macros, `foundry-build`, public exports | Compile-time diagnostics, generated typing, scaffold output, dependency hygiene | Reviewed |
| Examples, fixtures, guides, API docs, changelog | Copy-paste safety, API discoverability, migration impact, fixture compatibility | Reviewed |

## Prioritized accepted fixes

The following groups summarize the accepted working-tree repairs without duplicating the full changelog.

### Security

- **P0 — authorization and MFA:** Typed routes inside groups now inherit group guard/permission defaults. MFA-pending credentials can no longer renew the pending challenge beyond its bounded TTL or use protected WebSocket channels. Confirmed TOTP factors cannot be replaced by an ordinary full credential, and MFA exchange requires the pending credential state.
- **P0 — long-lived WebSocket authorization:** Guarded sockets now reauthenticate cached bearer/session credentials on a bounded interval without extending sliding sessions, replace the cached actor with the authenticator result, rerun declared permission checks, and disconnect invalid, revoked, expired, guard-mismatched, or MFA-pending actors before protected actions and broadcasts. The default revocation window is up to 30 seconds; custom channel authorizers remain subscription-time checks.
- **P1 — WebSocket boundary hardening:** Production same-origin matching compares scheme, host, and effective port; forwarded host/proto are trusted only from configured proxy peers. Process-local global and anonymous per-IP admission limits apply before upgrade, authenticated caps are keyed by `(GuardId, actor ID)`, internal handler/panic details are no longer returned in ACK frames, and handler-facing presence membership is isolated by channel and room.
- **P1 — HTTP middleware safety:** Typed route construction preserves inherited defaults, trusted-proxy resolution precedes rate limiting, and missing or duplicate middleware groups fail construction/bootstrap instead of silently producing an empty group.
- **P1 — concurrency and lifecycle ownership:** Scheduler overlap protection now uses owner tokens, fails closed, renews leases, cancels work after ownership loss, awaits safe release, and preserves deployed lock-key compatibility. Plugin boot rollback shuts down already-booted plugins in reverse order.
- **P1 — filesystem and upload boundaries:** Framework-owned generators reject symlinked components below their selected output root before write, directory creation, or manifest cleanup. New consuming upload store methods remove Foundry-owned temporary files after success or failure; borrowed methods retain explicit reuse semantics, with the adoption caveat recorded below.
- **P1 — notification and data exposure:** A guarded notification WebSocket helper authorizes only the authenticated actor's room and disallows client events. Audit JSON redaction is recursive, crypt configuration no longer exposes secrets through `Debug`, and database statement timeout state is cleaned after failed execution.
- **P2 — provider semantics:** S3 writes persist supplied content type as object metadata. Visibility remains an explicit bucket/CDN policy rather than emitting incompatible object ACL headers.
- **Dependency repair:** Active `crossbeam-epoch` was updated from vulnerable `0.9.18` to patched `0.9.20`. Consumer fixture locks were refreshed to patched `ammonia`, `anyhow`, `lettre`, `rand`, and `rustls-webpki` releases and dropped unmaintained `core2`; root `validator_derive` moved to its maintained patch dependency chain.

### Typing and correctness

- **P0 — semantic identifiers:** Middleware groups now use `MiddlewareGroupId` throughout registration and route APIs. Raw strings are not accepted accidentally; dynamically sourced IDs require the explicit owned constructor.
- **P0 — database type safety:** `find`/`find_many` require a model's declared primary-key type; `set_null` is limited to nullable columns; text predicates are limited to string columns; and `belongs_to` accepts typed nullable foreign keys without erasing the owner-key type. Compile-fail coverage protects cross-model keys and invalid predicates/assignments.
- **P0 — validation parity:** Required `Option<T>` fields are no longer implicitly nullable, typed numeric/boolean values remain typed, collection `each` rules and count bounds align across runtime and TypeScript contracts, and invalid derive combinations produce compile-time diagnostics.
- **P1 — contract SSOT:** Route callbacks are prepared once and reused by runtime routing, named-route metadata, OpenAPI, and TypeScript generation. Registered contract schemas are materialized and conflicting definitions fail. OpenAPI 3.1 nullability, unconstrained JSON, required path parameters, and structural schema identities were corrected.
- **P1 — generated clients:** WebSocket payload helpers use generated event metadata rather than an unsafe empty-map cast. TypeScript export planning rejects exact and ASCII case-only collisions across DTOs, app enums, framework runtimes, route helpers, SDK actions, and the barrel before cleanup or writes.
- **P1 — transaction/delivery correctness:** Migration DDL and ledger updates use the same pinned session and transaction. Immediate notifications attempt every selected channel and return aggregate failures; queued failures reach retry/dead-letter handling; missing channels and render/routing errors are no longer reported as success.
- **P2 — consistency repairs:** Repeated query/model assignments use a shared last-write-wins helper; app-enum keys/aliases reject empty and colliding parse namespaces; `Collection::chunk` no longer requires `Clone`; and `ContractError` plus public testing builder types are re-exported.

### DX and lifecycle

- **P0 — scheduler behavior:** Clock-driven cron evaluation uses the configured application timezone, while explicit injected-time hooks retain documented UTC semantics. Ambiguous/nonexistent DST wall times are skipped deterministically, and overlap TTL has a typed duration builder.
- **P1 — test lifecycle:** `TestApp` can reuse the production `AppBuilder`, exposes previously unreachable public builder/request types, and provides an awaited shutdown path for managed tasks and plugins.
- **P1 — fail early and explain:** Malformed i18n config fails bootstrap; settings type drift returns an error instead of disappearing as `None`; password-typed settings explicitly document that the type is presentation metadata, not encryption; and unknown middleware references name the missing ID.
- **P1 — operational ergonomics:** CLI `--help`/`--version` are successful outcomes, HTTP/WebSocket listener binding accepts IPv6 host syntax, and notification after-commit/render failures surface at the call site.
- **P2 — consumer clarity:** Model scaffolding no longer generates duplicate `Clone`; upload APIs distinguish final consumption from multi-use workflows; S3 ACL policy and temporary-file ownership are documented; examples, API references, and guides were reconciled with changed public APIs.

## Validation status

### Completed evidence

- The pre-change baseline and final post-change `make verify` both pass. The final run includes formatting, all targets, 1,150 library tests, integration/acceptance and compile-fail suites, strict Clippy with warnings denied, examples, and both consumer fixture families.
- `cargo test --test derive_ui` passes, including negative cases for app-enum collisions, cross-model primary keys, non-nullable `set_null`, non-text predicates, and incorrect manual key typing, plus the typed validation pass case.
- Focused suites for auth/HTTP authorization, contracts/OpenAPI/TypeScript, notifications, uploads/S3, WebSocket auth/origin/limits/presence, scheduler locking/timezone, foundation/plugin lifecycle, public API, collection/query/relation behavior, and generated-path symlink rejection passed during implementation.
- `make api-docs` regenerated 36 module files and the index from the settled public surface; generated Markdown is normalized to one trailing newline and `git diff --check` passes.
- Final OSV querybatch scans used active `cargo tree --target all` graphs: root 453 package/version pairs, blueprint fixture 448, plugin fixture 450, and API-doc tool 56. All patchable vulnerability advisories were removed. Remaining records are the constrained `quick-xml` advisories and the maintenance-only `paste`/`fxhash` notices described below. `cargo-audit` and `cargo-deny` are not installed.
- Postgres-specific tests compile, but their database bodies skip because `FOUNDRY_TEST_POSTGRES_URL` is not present. This limits runtime validation of MFA persistence, migration/ledger rollback, statement-timeout cleanup, and other Postgres-only paths.

### External validation still recommended

- Run the Postgres acceptance suites with `FOUNDRY_TEST_POSTGRES_URL` against a disposable Postgres 16 instance before release. This is an environment limitation of this audit, not a failing repository gate.

## Residual risks and backlog

| Severity | Residual item | Why it remains / recommended next step |
| --- | --- | --- |
| **High advisory; constrained reachability** | `object_store 0.13.2` pins `quick-xml 0.39.4`, affected by RUSTSEC-2026-0194 and RUSTSEC-2026-0195 denial-of-service advisories. | `object_store 0.14.0` still pins `quick-xml ^0.40.1`, below the patched `0.41`, while adding a larger dependency delta. The traced parser boundary is S3/list/multipart/STS-compatible XML returned by a configured provider; ordinary attacker-controlled object names are XML text rather than arbitrary parser structure. A clean repair needs an upstream `object_store` release accepting `quick-xml >=0.41`, or an explicit decision to carry a fork/patch. Track upstream and upgrade promptly; treat untrusted S3-compatible endpoints as unsafe meanwhile. |
| **Conditional Medium** | Legacy HTTP handlers that use borrowed upload `store*` methods, return early after multipart extraction, or never explicitly clean up can retain Foundry-owned temporary files; automatic pruning currently runs with Worker maintenance. | The consuming `store*_and_cleanup` APIs close the final-use path without breaking deferred attachment/image reuse, but they are opt-in. Migrate final-use routes to the consuming methods and run a Worker/pruner for defense in depth. A framework-wide automatic cleanup policy needs an explicit retain/ownership design before it can safely replace borrowed semantics. |
| **Medium hardening** | Configuration typo/unknown-field rejection and ambient unprefixed environment overlays are not uniformly strict. | Deployment config is a trusted input, so no direct exploit was established, but silent misspelling or ambient process variables can produce unsafe operational behavior. Add deny-unknown-field diagnostics where compatibility permits and make unprefixed overlays explicit/diagnosable. |
| **Medium correctness** | Rust wire-name handling can drift across `serde` rename rules, validation/multipart field names, API schemas, and generated TypeScript. | Resolving this cleanly requires one shared field-name model across several proc macros and generators. Establish that SSOT, then add rename/rename-all compile and contract fixtures before changing behavior. |
| **Low hardening** | Low-level raw image constructors do not all enforce the same resource bounds as the upload-validation path. | These are trusted-code escape hatches rather than a demonstrated remote path. Add bounded constructors or clearly named unchecked variants to prevent accidental memory/CPU amplification. |
| **Low DX/build** | Macro hygiene still assumes some consumer-visible direct dependencies, and `ApiSchema` has an unexpected TypeScript coupling. | Removing the coupling affects generated paths, trait bounds, and consumer manifests. Route generated references through Foundry re-exports and separate schema/TypeScript concerns with compile fixtures. |
| **Low architecture** | Some model defaults remain process-global rather than app-context scoped. | A normal deployment runs one app per process, so no current cross-tenant exploit was demonstrated. Move defaults into `AppContext` before supporting multiple independently configured apps in one process. |
| **Low product clarity** | `SettingType::Password` is presentation-only and does not encrypt stored values. | Documentation now states this explicitly. Applications must keep secrets in the secret/config system; any encrypted-settings feature needs separate key management and rotation design rather than an implicit behavioral change. |
| **Low maintenance** | Active `paste` (framework/image graph) and `fxhash` (standalone API-doc tool) releases are marked unmaintained. | These are maintenance advisories, not demonstrated vulnerabilities. Remove or replace them when upstream dependents permit, and keep them in dependency-monitoring output. |

The missing Postgres URL is the only remaining execution-environment validation gap. Residual design and upstream dependency items remain open until their stated decision or release is available.
