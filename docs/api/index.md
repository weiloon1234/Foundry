# Foundry API Surface

> Auto-generated from `cargo doc`. Regenerate: `make api-docs`

Each file documents one module's public API (structs, enums, traits, functions).
Load only the file you need — don't read them all at once.

For import stability and compatibility expectations, see [Public API Contract](public-api-contract.md).

| Module | Description | Size |
|--------|-------------|------|
| [root](root.md) | Crate root: derive macros, re-exports | 10L |
| [app_enum](modules/app_enum.md) | Enum metadata and serialization (FoundryAppEnum) | 28L |
| [attachments](modules/attachments.md) | File attachments with lifecycle (HasAttachments) | 76L |
| [audit](modules/audit.md) | Built-in audit logging with automatic model mutation tracking and redaction | 40L |
| [auth](modules/auth.md) | Auth: guards, policies, tokens, sessions, password reset, email verification | 214L |
| [cache](modules/cache.md) | In-memory and Redis-backed caching (CacheManager) | 32L |
| [cli](modules/cli.md) | CLI command registration (CommandRegistry) | 19L |
| [config](modules/config.md) | TOML-based configuration (ConfigRepository, AppConfig, etc.) | 106L |
| [countries](modules/countries.md) | Built-in country data (250 countries) | 27L |
| [database](modules/database.md) | AST-first query system: models, relations, projections, compiler | 786L |
| [datatable](modules/datatable.md) | Server-side datatables: filtering, sorting, pagination, XLSX export | 217L |
| [email](modules/email.md) | Multi-driver email: SMTP, Mailgun, Postmark, Resend, SES | 167L |
| [events](modules/events.md) | Domain event bus with typed listeners | 31L |
| [foundation](modules/foundation.md) | Core: App, AppBuilder, AppContext, AppTransaction, Error, ServiceProvider | 137L |
| [http](modules/http.md) | HTTP: routes, middleware (CORS, CSRF, rate limit, etc.), cookies, resources | 306L |
| [i18n](modules/i18n.md) | Internationalization: locale extraction, translation catalogs | 28L |
| [imaging](modules/imaging.md) | Image processing pipeline (resize, crop, rotate, format conversion) | 36L |
| [jobs](modules/jobs.md) | Background job queue with leased at-least-once delivery | 60L |
| [kernel](modules/kernel.md) | 5 runtime kernels: HTTP, CLI, Scheduler, Worker, WebSocket | 67L |
| [logging](modules/logging.md) | Structured logging, observability, health probes, diagnostics | 89L |
| [metadata](modules/metadata.md) | Key-value metadata for models (HasMetadata) | 21L |
| [notifications](modules/notifications.md) | Multi-channel notifications: email, database, broadcast | 35L |
| [openapi](modules/openapi.md) | OpenAPI 3.1.0 spec generation (ApiSchema, RouteDoc) | 38L |
| [plugin](modules/plugin.md) | Compile-time plugin system with dependency validation | 98L |
| [public](modules/public.md) |  | 67L |
| [redis](modules/redis.md) | Namespaced Redis wrapper (RedisManager, RedisConnection) | 41L |
| [scheduler](modules/scheduler.md) | Cron + interval scheduling with Redis-safe leadership | 54L |
| [settings](modules/settings.md) |  | 37L |
| [storage](modules/storage.md) | File storage: local + S3, multipart uploads, file validation | 133L |
| [support](modules/support.md) | Utilities: typed IDs, datetime/clock, Collection<T>, crypto, hashing, locks | 216L |
| [testing](modules/testing.md) | Test infrastructure: TestApp, TestClient, Factory | 40L |
| [translations](modules/translations.md) | Model field translations across locales (HasTranslations) | 42L |
| [typescript](modules/typescript.md) |  | 14L |
| [validation](modules/validation.md) | Validation: 38+ rules, custom rules, request validation extractor | 149L |
| [websocket](modules/websocket.md) | Channel-based WebSocket with presence and typed messages | 68L |

**Total: 35 modules, 3529 lines across all files.**
