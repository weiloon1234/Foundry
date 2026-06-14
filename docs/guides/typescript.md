# TypeScript Type Generation

Auto-generate TypeScript types from Rust structs and enums. Types are discovered at compile time — no manual registration.

---

## How It Works

Types that derive `ApiSchema`, `AppEnum`, or `foundry::TS` are automatically registered for TypeScript export. Run `types:export` to generate `.ts` files.

```
Rust DTO (#[derive(ApiSchema)]) → auto-discovered → types:export → TypeScript file
```

---

## Quick Start

### 1. Derive on your types

Request/response DTOs already derive `ApiSchema` — they're auto-included:

```rust
#[derive(Debug, Deserialize, ts_rs::TS, foundry::ApiSchema)]
pub struct CreateOrderRequest {
    pub product_id: String,
    pub quantity: u32,
}
```

Enums that derive `AppEnum` — also auto-included:

```rust
#[derive(Clone, Debug, PartialEq, foundry::AppEnum)]
pub enum OrderStatus {
    Pending,
    Confirmed,
    Shipped,
}
```

### 2. Run export

```bash
cargo run -- types:export
# or
make types
```

### 3. Use in frontend

```typescript
import type { CreateOrderRequest, OrderStatus } from "@shared/types/generated";
```

The configured `typescript.output_dir` is the supported frontend import path. `types:export`
tracks Foundry-owned output in `.foundry-types-manifest.json` and cleans only those generated files on
later runs, so colocated manual `.ts` files are not deleted. If you see raw ts-rs files in a root
`bindings/` directory, treat them as manual or stale output rather than a second source of truth.

---

## Config

```toml
# config/typescript.toml
[typescript]
output_dir = "frontend/shared/types/generated"
```

Foundry’s `types:export` command writes to this configured directory and builds the framework-facing barrel there. Apps should import from this path, not from any separate raw `bindings/` output.

Override via CLI flag:

```bash
cargo run -- types:export --output some/other/dir
```

Override via `.env`:

```
TYPESCRIPT__OUTPUT_DIR=frontend/shared/types/generated
```

---

## Derives

### `foundry::ApiSchema` (request/response DTOs)

Auto-registers for TypeScript export. Must also derive `ts_rs::TS`:

```rust
#[derive(Debug, Deserialize, ts_rs::TS, foundry::ApiSchema)]
pub struct MyRequest { ... }
```

### `foundry::AppEnum` (enums)

Auto-registers for TypeScript export on its own, and also implements `ts_rs::TS`
for DTO fields:

```rust
#[derive(Clone, Debug, PartialEq, foundry::AppEnum)]
pub enum MyEnum { ... }
```

Use AppEnum fields directly in request/response DTOs. Manual
`#[ts(type = "import(...)")]` overrides are not needed, including for
`Option<MyEnum>` and collections:

```rust
#[derive(Debug, serde::Serialize, ts_rs::TS, foundry::ApiSchema)]
pub struct OrderResponse {
    pub status: OrderStatus,
    pub previous_status: Option<OrderStatus>,
    pub allowed_statuses: Vec<OrderStatus>,
}
```

Default metadata follows Foundry conventions:

```rust
#[derive(Clone, Debug, PartialEq, foundry::AppEnum)]
#[foundry(label_prefix = "admin.orders.statuses")] // optional
pub enum OrderStatus {
    Pending,
    Confirmed,
}
```

Generated TypeScript includes the union type plus runtime metadata:

```typescript
export type OrderStatus = "pending" | "confirmed";

export const OrderStatusValues = [
  "pending",
  "confirmed",
] as const;

export const OrderStatusOptions = [
  { value: "pending", labelKey: "admin.orders.statuses.pending" },
  { value: "confirmed", labelKey: "admin.orders.statuses.confirmed" },
] as const;

export const OrderStatusMeta = {
  id: "order_status",
  keyKind: "string",
  options: OrderStatusOptions,
} as const;
```

String AppEnums whose keys are consistently shaped as `<module>.<action>` also export
grouped helpers. This keeps permission call sites typed without maintaining a
parallel frontend map:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, foundry::AppEnum)]
#[foundry(id_type = PermissionId)]
pub enum Permission {
    #[foundry(key = "audit_logs.read")]
    AuditLogsRead,
    #[foundry(key = "observability.view")]
    ObservabilityView,
}
```

```typescript
export const PermissionGroups = {
  auditLogs: { read: "audit_logs.read" },
  observability: { view: "observability.view" },
} as const;
```

The exporter fails fast if one enum mixes dotted and non-dotted keys, or if two
module names normalize to the same camelCase TypeScript property.

### Route Manifest

`types:export` also boots the registered HTTP route modules and writes a
generated `RouteManifest.ts`. Named routes from `scope()`, `route_named()`, and
`resource()` become typed frontend URL helpers:

```typescript
import {
  RouteIds,
  createRouteUrlBuilder,
  routeUrl,
} from "@shared/types/generated";

routeUrl(RouteIds.admin.users.show, { id: userId });

const adminRouteUrl = createRouteUrlBuilder({ basePath: "/api/v1/admin" });
adminRouteUrl(RouteIds.admin.users.show, { id: userId });
// -> "/users/123" after substituting and stripping the admin API base path
```

The generated manifest includes route id, path, method, path params, guard,
permissions, summary, request schema name, and response schema names when Foundry
can infer them from the registered route. Route id groups are camelCased for
TypeScript property access, so `admin.audit_logs.index` becomes
`RouteIds.admin.auditLogs.index`.

Route params support Axum `{id}` / `{*path}` patterns and legacy `:id` patterns.
The helper URL-encodes substituted params and throws a clear runtime error if a
required param is missing.

This is route URL and metadata generation only. Request body trust still belongs
to Foundry validation extractors such as `JsonValidated<T>` and `Validated<T>`.

### `foundry::TS` (escape hatch)

For any type that isn't a DTO or AppEnum but needs TypeScript export:

```rust
#[derive(Serialize, ts_rs::TS, foundry::TS)]
pub struct SomeCustomType {
    pub name: String,
    pub value: f64,
}
```

---

## ts_rs Attributes

Control TypeScript output with `#[ts(...)]` attributes:

```rust
#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
pub struct Example {
    pub name: String,

    #[ts(type = "number")]           // Override TS type
    pub count: u64,

    #[ts(optional)]                  // T | undefined
    pub nickname: Option<String>,

    #[ts(type = "Record<string, any>")]  // Complex type override
    pub metadata: serde_json::Value,
}
```

Common attributes:
- `#[ts(type = "...")]` — override generated TypeScript type
- `#[ts(optional)]` — make field optional (`T | undefined`)
- `#[ts(rename = "...")]` — rename in TypeScript output
- `#[ts(export_to = "...")]` — rare filename/subdirectory override inside `typescript.output_dir`

Do not use `#[ts(export)]` on types deriving `foundry::ApiSchema` or
`foundry::TS`. Foundry registers those types automatically and `types:export`
writes them to the configured `typescript.output_dir`; direct ts-rs export
creates unmanaged files such as root `bindings/*.ts`.

---

## Framework Types

These types are auto-exported by the framework (no configuration needed):

| Type | Module | TypeScript |
|------|--------|------------|
| `CountryStatus` | `foundry::countries` | `"enabled" \| "disabled"` |
| `TokenPair` | `foundry::auth::token` | `{ access_token, refresh_token, ... }` |
| `RefreshTokenRequest` | `foundry::auth::token` | `{ refresh_token }` |
| `TokenResponse` | `foundry::auth::token` | `{ tokens: TokenPair }` |
| `WsTokenResponse` | `foundry::auth::token` | `{ token }` |
| `MessageResponse` | `foundry::http::response` | `{ message }` |
| `DatatableRequest` | `foundry::datatable::request` | typed filters + sorts + pagination |
| `DatatableJsonResponse` | `foundry::datatable::response` | typed columns + filters + applied filters + sorts |
| `JobHistoryStatus` | `foundry::jobs` | `"succeeded" \| "retried" \| "dead_lettered"` |
| `SettingType` | `foundry::settings` | `"text" \| "textarea" \| "number" \| ...` |

Datatable exports now keep JSON-facing numeric fields as `number` and include the supporting filter option imports needed by generated metadata files.

---

## Generated Output

```
frontend/shared/types/generated/
├── index.ts                    ← barrel (auto-generated)
├── CreateOrderRequest.ts       ← from project
├── OrderStatus.ts              ← from project
├── CountryStatus.ts            ← from framework
├── DatatableJsonResponse.ts    ← from framework
├── DatatableRequest.ts         ← from framework
├── MessageResponse.ts          ← from framework
├── RefreshTokenRequest.ts      ← from framework
├── TokenPair.ts                ← from framework
├── TokenResponse.ts            ← from framework
├── WsTokenResponse.ts          ← from framework
└── ...
```

The barrel `index.ts` re-exports all types:

```typescript
// Auto-generated barrel. Do not edit.
export type { CreateOrderRequest } from "./CreateOrderRequest";
export { type CountryStatus, CountryStatusValues, CountryStatusOptions, CountryStatusMeta } from "./CountryStatus";
export type { DatatableJsonResponse } from "./DatatableJsonResponse";
export type { DatatableRequest } from "./DatatableRequest";
export type { MessageResponse } from "./MessageResponse";
export { type OrderStatus, OrderStatusValues, OrderStatusOptions, OrderStatusMeta } from "./OrderStatus";
export type { RefreshTokenRequest } from "./RefreshTokenRequest";
export type { TokenPair } from "./TokenPair";
export type { TokenResponse } from "./TokenResponse";
export type { WsTokenResponse } from "./WsTokenResponse";
```

---

## Integration with Makefile

```makefile
# Generate types (auto-discovered)
types:
    @PROCESS=cli cargo run -- types:export

# Dev: generates types before starting servers
dev: types
    ...

# Build: generates types before frontend build
build: types
    cd frontend/admin && npm run build
    cargo build --release
```

---

## Workflow

1. Add or modify a Rust struct/enum with `ApiSchema`, `AppEnum`, or `foundry::TS`
2. Run `make types` (or `make dev` / `make build` which include it)
3. TypeScript types are generated — import and use in frontend

No registration files. No manual type lists. Derive → export → use.
