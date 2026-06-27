# typescript

[Back to index](../index.md)

## foundry::typescript

```rust
struct TsAppEnum
struct TsType
fn builtin_cli_registrar(routes: Vec<RouteRegistrar>) -> CommandRegistrar
fn export_all(dir: &Path) -> Result<()>
fn export_all_with_routes( dir: &Path, routes: &[RouteManifestEntry], ) -> Result<()>
fn export_all_with_routes_and_websocket_channels( dir: &Path, routes: &[RouteManifestEntry], websocket_channels: &[WebSocketRouteRegistrar], ) -> Result<()>
```

## Notes

- `types:export` emits `HttpManifest.ts` from safe HTTP runtime config alongside
  route/auth/cache/storage manifests. It includes CSRF cookie/header names,
  CORS method/header settings, request limits, security-header flags, and the
  global rate-limit shape without exporting trusted-proxy CIDRs, rate-limit key
  prefixes, or raw CSP policy text.
- `AppManifest.ts` includes frontend-safe `AppConfig` metadata: app name,
  environment label, environment kind and booleans, timezone, and background
  shutdown timeout without exporting the signing key.
- `AuthManifest.ts` also includes `AuthRuntimeManifest` from frontend-safe auth
  config: token/session TTLs, session cookie integration metadata,
  password-reset/email-verification expiry, lockout policy, MFA settings, and
  per-guard token TTL overrides.
- `WebSocketChannelManifest.ts` also includes `WebSocketRuntimeManifest` from
  frontend-safe WebSocket config: path, heartbeat timings, query-token
  metadata, client-facing limits, and history caps without exporting bind
  host/port, allowed origins, or transport buffer internals.
- `JobManifest.ts` also includes `JobRuntimeManifest` from frontend-safe job
  config: default queue, retry/timeout/concurrency policy, queue priorities,
  and history retention without exporting worker polling, lease, requeue,
  shutdown, or prune scheduling internals.
- `ScheduleManifest.ts` also includes `SchedulerRuntimeManifest` from
  `SchedulerConfig`: tick interval, leadership lease TTL, and shutdown timeout
  for admin/tooling displays.
- `DatatableManifest.ts` also includes `DatatableRuntimeManifest` from
  `DatatableConfig`: JSON page-size and XLSX/export row caps for frontend table
  builders and admin tooling.
- `DatabaseManifest.ts` includes browser-safe database pagination defaults from
  `DatabaseConfig`, including the `default_per_page` value used by the direct
  `Pagination` extractor, without exporting URLs, schema names, migration
  paths/tables, pool settings, or SQL observability internals.
- `StorageManifest.ts` also includes `StorageRuntimeManifest` from
  `StorageConfig`: configured default disk, upload caps, image decode caps,
  temp-upload pruning settings, and attachment-orphan policy without exporting
  disk roots, buckets, endpoints, or credentials.
- `EmailManifest.ts` also includes `EmailRuntimeManifest` from `EmailConfig`:
  configured default mailer, outbound queue, and attachment byte caps without
  exporting provider endpoints, sender settings, templates, or credentials.
- `AuditManifest.ts` includes backend-owned audit event types, redaction
  marker, sensitive field names, sensitive-name segment heuristics, and
  generated helpers for audit dashboards without copying audit event strings or
  redaction rules.
- `LoggingManifest.ts` includes frontend-safe `LoggingConfig` metadata: log
  level, output format, log directory, and retention days while reusing the
  generated `LogLevel` union.
- `ObservabilityManifest.ts` includes frontend-safe `ObservabilityConfig`
  metadata: route base path, enable/capture flags, sample/channel retention,
  tracing flag, service name, and WebSocket payload visibility without
  exporting the OTLP endpoint.
- `NotificationManifest.ts` includes backend-owned notification type metadata,
  built-in notification channel ids, canonical broadcast channel/event
  constants, payload schema names, and typed broadcast narrowing helpers.
