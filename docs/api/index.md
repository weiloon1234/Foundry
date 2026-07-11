# Foundry API Surface

> Auto-generated from `cargo doc`. Regenerate: `make api-docs`

Each file documents one module's public API (structs, enums, traits, functions).
Load only the file you need — don't read them all at once.

For import stability and compatibility expectations, see [Public API Contract](../public-api-contract.md).

| Module | Description | Size |
|--------|-------------|------|
| [root](root.md) | Crate root: derive macros, re-exports | 10L |
| [app_enum](modules/app_enum.md) | Enum metadata and serialization (FoundryAppEnum) | 27L |
| [attachments](modules/attachments.md) | File attachments with lifecycle (HasAttachments) | 76L |
| [audit](modules/audit.md) | Built-in audit logging with automatic model mutation tracking and redaction | 61L |
| [auth](modules/auth.md) | Auth: guards, policies, tokens, sessions, password reset, email verification | 215L |
| [cache](modules/cache.md) | In-memory and Redis-backed caching (CacheManager) | 41L |
| [cli](modules/cli.md) | CLI command registration (CommandRegistry) | 43L |
| [config](modules/config.md) | TOML-based configuration (ConfigRepository, AppConfig, etc.) | 117L |
| [contract](modules/contract.md) | Normalized contract manifest for generated SDKs, OpenAPI, validation, and realtime | 43L |
| [countries](modules/countries.md) | Built-in country data (250 countries) | 26L |
| [database](modules/database.md) | AST-first query system: models, relations, projections, compiler | 833L |
| [datatable](modules/datatable.md) | Server-side datatables: filtering, sorting, pagination, XLSX export | 228L |
| [email](modules/email.md) | Multi-driver email: SMTP, Mailgun, Postmark, Resend, SES | 171L |
| [events](modules/events.md) | Domain event bus with typed listeners | 30L |
| [foundation](modules/foundation.md) | Core: App, AppBuilder, AppContext, AppTransaction, Error, ServiceProvider | 143L |
| [http](modules/http.md) | HTTP: routes, middleware (CORS, CSRF, rate limit, etc.), cookies, resources | 343L |
| [http_client](modules/http_client.md) | Outbound HTTP: pooled transport, timeouts, safe retries, typed responses, and fakes | 299L |
| [i18n](modules/i18n.md) | Internationalization: locale extraction, translation catalogs | 27L |
| [imaging](modules/imaging.md) | Image processing pipeline (resize, crop, rotate, format conversion) | 49L |
| [jobs](modules/jobs.md) | Background job queue with leased at-least-once delivery | 65L |
| [kernel](modules/kernel.md) | 5 runtime kernels: HTTP, CLI, Scheduler, Worker, WebSocket | 68L |
| [logging](modules/logging.md) | Structured logging, observability, health probes, diagnostics | 96L |
| [metadata](modules/metadata.md) | Key-value metadata for models (HasMetadata) | 30L |
| [notifications](modules/notifications.md) | Multi-channel notifications: email, database, broadcast | 73L |
| [openapi](modules/openapi.md) | OpenAPI 3.1.0 spec generation (ApiSchema, RouteDoc) | 49L |
| [plugin](modules/plugin.md) | Compile-time plugin system with dependency validation | 98L |
| [public](modules/public.md) | Stable convenience re-exports for consumer applications | 254L |
| [redis](modules/redis.md) | Namespaced Redis wrapper (RedisManager, RedisConnection) | 64L |
| [scheduler](modules/scheduler.md) | Cron + interval scheduling with Redis-safe leadership | 54L |
| [settings](modules/settings.md) | Typed persistent application settings with cache integration | 38L |
| [storage](modules/storage.md) | File storage: local + S3, multipart uploads, file validation | 147L |
| [support](modules/support.md) | Utilities: typed IDs, datetime/clock, Collection<T>, crypto, hashing, locks | 220L |
| [testing](modules/testing.md) | Test infrastructure: TestApp, clients/fakes, assertions, and model factories | 206L |
| [translations](modules/translations.md) | Model field translations across locales (HasTranslations) | 41L |
| [typescript](modules/typescript.md) | TypeScript contracts, route helpers, validation metadata, and SDK export | 25L |
| [validation](modules/validation.md) | Validation: 38+ rules, custom rules, request validation extractor | 168L |
| [websocket](modules/websocket.md) | Channel-based WebSocket with presence and typed messages | 79L |

**Total: 37 modules, 4557 lines across all files.**
