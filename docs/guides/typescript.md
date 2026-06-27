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
#[derive(Debug, serde::Deserialize, foundry::ts_rs::TS, foundry::ApiSchema)]
#[ts(crate = "foundry::ts_rs")]
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
Route helpers are generated only from backend-registered TypeScript contracts:
if a documented request or response schema is not exported, `types:export` fails
with the missing schema name instead of emitting an `unknown` alias. Derive
`foundry::ts_rs::TS` with `foundry::ApiSchema`, register a manual
`foundry::TS` export, or opt that route out of client export when the schema is
intentionally server-only.

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

Auto-registers for TypeScript export. Must also derive `foundry::ts_rs::TS`:

```rust
#[derive(Debug, serde::Deserialize, foundry::ts_rs::TS, foundry::ApiSchema)]
#[ts(crate = "foundry::ts_rs")]
pub struct MyRequest { ... }
```

The `make:request` scaffold derives `serde::Deserialize`,
`foundry::ts_rs::TS`, `foundry::ApiSchema`, and `foundry::Validate`, so a newly
generated request DTO can be used with `JsonValidated<T>` / `Validated<T>` and
the generated route helper/OpenAPI validation metadata.
The `make:response` scaffold derives `serde::Serialize`,
`foundry::ts_rs::TS`, and `foundry::ApiSchema`, so a newly generated response
DTO can be used with `route.response::<T>(status)`, `types:export`, and OpenAPI
route docs without adding a direct `ts-rs` dependency.
The `make:model` scaffold derives `serde::Serialize`,
`foundry::ts_rs::TS`, and `foundry::ApiSchema`, so a newly generated model can
be used as a typed response contract without adding a direct `ts-rs` or
`serde_json` dependency to the consumer crate. The scaffold still expects the
same `serde` dependency used by generated jobs and request/response DTOs.
`make:event` follows the same exportable-contract pattern for domain events and
also registers `TsEventPayload`, so the generated payload appears in
`EventManifest.ts`.
`make:notification` does the same for broadcast notification payloads and
registers `TsNotification`, so generated clients can narrow
`NotificationBroadcastPayload.data` by backend-owned notification type.

### `foundry::AppEnum` (enums)

Auto-registers for TypeScript export on its own, and also implements
`foundry::ts_rs::TS` for DTO fields:

```rust
#[derive(Clone, Debug, PartialEq, foundry::AppEnum)]
pub enum MyEnum { ... }
```

Use AppEnum fields directly in request/response DTOs. Manual
`#[ts(type = "import(...)")]` overrides are not needed, including for
`Option<MyEnum>` and collections:

```rust
#[derive(Debug, serde::Serialize, foundry::ts_rs::TS, foundry::ApiSchema)]
#[ts(crate = "foundry::ts_rs")]
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

function freezeOrderStatusEnumValue<TValue>(value: TValue): TValue {
  // Generated helper recursively freezes arrays/objects.
  return value;
}

export const OrderStatusValues = freezeOrderStatusEnumValue([
  "pending",
  "confirmed",
] as const);

export const OrderStatusOptions = freezeOrderStatusEnumValue([
  { value: "pending", labelKey: "admin.orders.statuses.pending", aliases: [] },
  { value: "confirmed", labelKey: "admin.orders.statuses.confirmed", aliases: [] },
] as const);

export const OrderStatusAliases = freezeOrderStatusEnumValue({} as const);
export type OrderStatusAlias = keyof typeof OrderStatusAliases;

export const OrderStatusMeta = freezeOrderStatusEnumValue({
  id: "order_status",
  keyKind: "string",
  options: OrderStatusOptions,
} as const);

export const OrderStatusKeys = freezeOrderStatusEnumValue({
  pending: "pending",
  confirmed: "confirmed",
} as const);

export function getOrderStatusValues(): OrderStatus[];
export function getOrderStatusValueCount(): number;
export function hasOrderStatusValues(): boolean;
export function getOrderStatusFirstValue(): OrderStatus | undefined;
export function getOrderStatusFirstValueOrNull(): OrderStatus | null;
export function getOrderStatusOptions(): Array<(typeof OrderStatusOptions)[number]>;
export function getOrderStatusOptionCount(): number;
export function hasOrderStatusOptions(): boolean;
export function getOrderStatusFirstOption(): (typeof OrderStatusOptions)[number] | undefined;
export function getOrderStatusFirstOptionOrNull(): (typeof OrderStatusOptions)[number] | null;
export function getOrderStatusMeta(): typeof OrderStatusMeta;
export function isOrderStatus(value: unknown): value is OrderStatus;
export function getOrderStatusAliases(): typeof OrderStatusAliases;
export function getOrderStatusAliasNames(): OrderStatusAlias[];
export function getOrderStatusAliasCount(): number;
export function hasOrderStatusAliases(): boolean;
export function isOrderStatusAlias(value: unknown): value is OrderStatusAlias;
export function getOrderStatusAliasTarget(value: OrderStatusAlias): OrderStatus;
export function getOrderStatusAliasTargetOrNull(value: unknown): OrderStatus | null;
export function getOrderStatusCanonicalValue(value: OrderStatus | OrderStatusAlias): OrderStatus;
export function parseOrderStatusOrNull(value: unknown): OrderStatus | null;
export function getOrderStatusOption(value: OrderStatus): (typeof OrderStatusOptions)[number];
export function getOrderStatusLabelKey(value: OrderStatus): (typeof OrderStatusOptions)[number]["labelKey"];
export function getOrderStatusKeys(): typeof OrderStatusKeys;
export function getOrderStatusKeyNames(): Array<keyof typeof OrderStatusKeys>;
export function getOrderStatusKeyCount(): number;
export function hasOrderStatusKeys(): boolean;
export function getOrderStatusFirstKeyName(): keyof typeof OrderStatusKeys | undefined;
export function getOrderStatusFirstKeyNameOrNull(): keyof typeof OrderStatusKeys | null;
export function getOrderStatusFirstKeyValue(): (typeof OrderStatusKeys)[keyof typeof OrderStatusKeys] | undefined;
export function getOrderStatusFirstKeyValueOrNull(): (typeof OrderStatusKeys)[keyof typeof OrderStatusKeys] | null;
```

When a Rust variant declares `#[foundry(aliases = ["legacy"])]`, generated TypeScript keeps the union canonical but exports the alias map. `parse{Name}OrNull(...)` accepts either canonical values or aliases and returns the canonical value, `is{Name}(...)` remains a canonical-value guard, and generated `app_enum` validation metadata includes the aliases from `FoundryAppEnum::accepted_keys()`.

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
export const PermissionGroups = freezePermissionEnumValue({
  auditLogs: { read: "audit_logs.read" },
  observability: { view: "observability.view" },
} as const);

export function getPermissionGroups(): typeof PermissionGroups;
export function getPermissionGroupNames(): Array<keyof typeof PermissionGroups>;
export function getPermissionGroupCount(): number;
export function hasPermissionGroups(): boolean;
export function getPermissionFirstGroupName(): keyof typeof PermissionGroups | undefined;
export function getPermissionFirstGroupNameOrNull(): keyof typeof PermissionGroups | null;
export function getPermissionFirstGroup(): (typeof PermissionGroups)[keyof typeof PermissionGroups] | undefined;
export function getPermissionFirstGroupOrNull(): (typeof PermissionGroups)[keyof typeof PermissionGroups] | null;
```

The exporter fails fast if one enum mixes dotted and non-dotted keys, or if two
module names normalize to the same camelCase TypeScript property. Direct
`TsAppEnum` metadata entries must also use unique TypeScript identifier names,
non-empty trimmed enum ids, at least one option, values matching the declared
`keyKind`, unique option values, and non-empty trimmed label keys before
`types:export` writes frontend files.
Generated AppEnum values, options, metadata, string key constants, and grouped
permission helpers are frozen at runtime, so direct mutation cannot change
backend-owned enum metadata. AppEnum selector helpers such as
`getOrderStatusValues()`, `getOrderStatusValueCount()`,
`hasOrderStatusValues()`, `getOrderStatusFirstValue()`,
`getOrderStatusFirstValueOrNull()`,
`getOrderStatusOptions()`, `getOrderStatusOptionCount()`,
`hasOrderStatusOptions()`, `getOrderStatusFirstOption()`,
`getOrderStatusFirstOptionOrNull()`,
`getOrderStatusMeta()`, `getOrderStatusAliases()`,
`getOrderStatusAliasNames()`, `getOrderStatusAliasCount()`,
`hasOrderStatusAliases()`, `isOrderStatusAlias()`,
`getOrderStatusAliasTarget()`, `getOrderStatusAliasTargetOrNull()`,
`getOrderStatusCanonicalValue()`, `parseOrderStatusOrNull()`,
`getOrderStatusKeys()`, `getOrderStatusKeyNames()`,
`getOrderStatusKeyCount()`, `hasOrderStatusKeys()`,
`getOrderStatusFirstKeyName()`, `getOrderStatusFirstKeyNameOrNull()`,
`getOrderStatusFirstKeyValue()`, `getOrderStatusFirstKeyValueOrNull()`,
`getPermissionGroups()`, `getPermissionGroupNames()`,
`getPermissionGroupCount()`, `hasPermissionGroups()`,
`getPermissionFirstGroupName()`, `getPermissionFirstGroupNameOrNull()`,
`getPermissionFirstGroup()`, and `getPermissionFirstGroupOrNull()` clone or
summarize non-scalar metadata before returning it, so UI code can add local
labels, ordering, or display grouping to selector results without local
`Object.keys(...)`, `.length`, `> 0`, first-entry wrappers, or
`... ?? null` / `isOrderStatus(value) ? value : null` wrappers.

### Route Manifest and Endpoint Helpers

`types:export` also boots the registered HTTP route modules and writes a
generated `RouteManifest.ts`. Named routes from `scope()`, `route_named()`, and
`resource()` become typed frontend URL helpers:

```typescript
import {
  RouteIds,
  createRouteUrlBuilder,
  createRouteUrlBuilderOrNull,
  routeMatch,
  routeMatchAny,
  routeUrl,
  routeUrlOrNull,
} from "@shared/types/generated";

routeUrl(RouteIds.admin.users.show, { id: userId });
routeUrlOrNull(RouteIds.admin.users.show, { id: userId });
const matched = routeMatch(RouteIds.admin.users.show, "/api/v1/admin/users/123");
matched?.params.id;
routeMatchAny("/api/v1/admin/users/123")?.name;

const adminRouteUrl = createRouteUrlBuilder({ basePath: "/api/v1/admin" });
adminRouteUrl(RouteIds.admin.users.show, { id: userId });
// -> "/users/123" after substituting and stripping the admin API base path
const safeAdminRouteUrl = createRouteUrlBuilderOrNull({ basePath: "/api/v1/admin" });
safeAdminRouteUrl(RouteIds.admin.users.show, { id: userId });
```

The generated manifest includes route id, path, method, path params, whether the
route requires auth, guard, permissions, dynamic `authorize(...)` callback
presence, middleware group, resolved audit area, OpenAPI-style docs metadata
(`operationId`, `summary`, `description`, `tags`, `deprecated`), effective route
rate-limit metadata, MFA pending-token allowance, request schema name, response
schema names plus backend-owned `hasBody` / `mediaType` response metadata, and
whether a route endpoint helper was generated.
Directly constructed `RouteManifestEntry` values must use non-empty, trimmed
request and response schema names whose plain atoms contain only ASCII letters,
digits, dots, hyphens, or underscores. Generic schema expressions must use
Foundry's supported wrappers (`Array<T>`, `Map<T>`, `Nullable<T>`,
`PaginatedResponse<T>`, `CursorPaginated<T>`, or `Collection<T>`) with a
non-empty inner schema. Direct entries must also use a non-empty trimmed `path`
that starts with `/`, and a `params` list that exactly matches the params
declared by `path`, so generated URL helper types stay aligned with runtime
substitution. Guard, permission, or authorize callback metadata requires
`requires_auth: true`, and middleware group / audit area metadata must be
non-empty and trimmed when present. Guard ids, permission ids, `operationId`,
`summary`, `description`, and route tags must also be non-empty and trimmed when
present; permissions and tags must be unique per route, and `operationId` values
must be unique across the route manifest to match OpenAPI.
Route rate-limit metadata must use positive max/window values, route-level
`windowSeconds` values must fit within JavaScript's safe integer range, and
actor-only rate limits require `requires_auth: true` because they run after
authentication.
Request metadata follows the backend-collected shape: routes without request
schemas must leave request transport/media metadata empty, query requests must
leave `requestMediaType` empty, and body requests must include
`requestMediaType`.
Direct `RouteManifestResponse` values must keep `has_body` and `media_type`
aligned with Foundry's status/schema response metadata rules.
Route id groups are camelCased for TypeScript property access, so
`admin.audit_logs.index` becomes `RouteIds.admin.auditLogs.index`. Frontend
tooling can import `RouteManifestEntry`, `RouteManifestResponse`,
`RouteHttpMethod`, `RouteHttpMethods`, `RouteRequestTransports`,
`RouteRequestMediaTypes`, `RouteResponseMediaTypes`, `isRouteHttpMethod()`,
`routeHttpMethodOrNull()`, `isRouteRequestTransport()`,
`routeRequestTransportOrNull()`, `isRouteRequestMediaType()`,
`routeRequestMediaTypeOrNull()`, `isRouteResponseMediaType()`,
`routeResponseMediaTypeOrNull()`, `routeNameOrNull()`, `routeEntries()`, `routeManifestEntry()`,
`routeManifestEntryOrNull()`,
`routeNames()`, `routeCount()`, `routeHasEntries()`, `routeFirstEntry()`,
`routeFirstEntryOrNull()`, `routeFirstName()`, `routeFirstNameOrNull()`,
`routePath()`, `routePathOrNull()`, `routeMethod()`,
`routeMethodOrNull()`, `routeHasMethod()`,
`routesWithMethod()`, `routeNamesWithMethod()`, `routesWithoutMethod()`,
`routeNamesWithoutMethod()`, method-route count/presence/first/`OrNull`
selectors,
`routeParamNames()`, `routeParamNamesOrNull()`, `routeParamCount()`,
`routeHasParams()`, `routeHasParam()`,
`routeFirstParamName()`, `routeFirstParamNameOrNull()`, `routesWithParams()`, `routeNamesWithParams()`,
`routesWithoutParams()`, `routeNamesWithoutParams()`, `routesWithParam()`,
`routeNamesWithParam()`, `routesWithoutParam()`, `routeNamesWithoutParam()`,
param-route count/presence/first/`OrNull` selectors,
`routeClientExported()`, `routeClientExportedOrNull()`, `routeGuard()`,
`routeGuardOrNull()`, `routeHasGuard()`, `routeHasAuthorizeCallback()`,
`routeHasAuthorizeCallbackOrNull()`, `routeAllowsMfaPendingToken()`,
`routeAllowsMfaPendingTokenOrNull()`, `routeRejectsMfaPendingToken()`, `routeAuditArea()`,
`routeAuditAreaOrNull()`, `routeHasAuditArea()`, `routeRateLimits()`,
`routeRateLimitsOrNull()`, `routeRateLimitCount()`, `routeHasRateLimits()`,
`routeHasRateLimitBy()`, `routeFirstRateLimit()`,
`routeFirstRateLimitOrNull()`, `routePermissions()`,
`routePermissionsOrNull()`, `routePermissionCount()`, `routeHasPermissions()`,
`routeFirstPermission()`, `routeFirstPermissionOrNull()`, `routeTags()`, `routeTagsOrNull()`,
`routeTagCount()`, `routeHasTags()`, `routeFirstTag()`, `routeFirstTagOrNull()`,
`routesRequiringAuth()`,
`routeNamesRequiringAuth()`, `authRequiredRouteCount()`,
`routeHasAuthRequiredRoutes()`, `firstAuthRequiredRoute()`,
`firstAuthRequiredRouteOrNull()`, `firstAuthRequiredRouteName()`,
`firstAuthRequiredRouteNameOrNull()`, `publicRoutes()`, `publicRouteNames()`,
`publicRouteCount()`, `routeHasPublicRoutes()`, `firstPublicRoute()`,
`firstPublicRouteOrNull()`, `firstPublicRouteName()`,
`firstPublicRouteNameOrNull()`, `clientExportedRoutes()`,
`clientExportedRouteNames()`, `clientExportedRouteCount()`,
`routeHasClientExportedRoutes()`, `firstClientExportedRoute()`,
`firstClientExportedRouteOrNull()`, `firstClientExportedRouteName()`,
`firstClientExportedRouteNameOrNull()`, `nonClientExportedRoutes()`,
`nonClientExportedRouteNames()`, `nonClientExportedRouteCount()`,
`routeHasNonClientExportedRoutes()`, `firstNonClientExportedRoute()`,
`firstNonClientExportedRouteOrNull()`, `firstNonClientExportedRouteName()`,
`firstNonClientExportedRouteNameOrNull()`,
`routesAllowingMfaPendingToken()`, `routeNamesAllowingMfaPendingToken()`,
`routesRejectingMfaPendingToken()`, `routeNamesRejectingMfaPendingToken()`,
MFA pending-token route count/presence/first/`OrNull` selectors,
`routesWithAuthorizeCallback()`, `routeNamesWithAuthorizeCallback()`,
`routesWithoutAuthorizeCallback()`, `routeNamesWithoutAuthorizeCallback()`,
authorize-callback route count/presence/first/`OrNull` selectors,
`routesByMethod()`, `routeNamesByMethod()`, `routesWithGuard()`,
`routeNamesWithGuard()`, `routesWithAnyGuard()`, `routeNamesWithAnyGuard()`,
`routesWithoutGuard()`, `routeNamesWithoutGuard()`, guard-route
count/presence/first/`OrNull` selectors, `routesWithAuditArea()`,
`routeNamesWithAuditArea()`, `routesWithAnyAuditArea()`,
`routeNamesWithAnyAuditArea()`, `routesWithoutAuditArea()`,
`routeNamesWithoutAuditArea()`, audit-area route count/presence/first/`OrNull`
selectors, `routesWithRateLimits()`, `routeNamesWithRateLimits()`,
`routesWithoutRateLimits()`, `routeNamesWithoutRateLimits()`,
`routesWithRateLimitBy()`, `routeNamesWithRateLimitBy()`, rate-limit route
count/presence/first/`OrNull` selectors, `routesWithPermission()`,
`routeNamesWithPermission()`, `routesWithoutPermission()`,
`routeNamesWithoutPermission()`, permission-route count/presence/first/`OrNull`
selectors, `routesWithTag()`, `routeNamesWithTag()`, `routesWithoutTag()`,
`routeNamesWithoutTag()`, tag-route count/presence/first/`OrNull` selectors,
`routeOperationId()`, `routeOperationIdOrNull()`, `routeHasOperationId()`,
`routeSummary()`, `routeSummaryOrNull()`, `routeHasSummary()`,
`routeDescription()`, `routeDescriptionOrNull()`, `routeHasDescription()`,
`routeIsDeprecated()`, `routeIsDeprecatedOrNull()`, `routeHasDocumentMetadata()`,
`deprecatedRoutes()`,
`deprecatedRouteNames()`, `deprecatedRouteCount()`,
`routeHasDeprecatedRoutes()`, `firstDeprecatedRoute()`,
`firstDeprecatedRouteOrNull()`, `firstDeprecatedRouteName()`,
`firstDeprecatedRouteNameOrNull()`, `nonDeprecatedRoutes()`,
`nonDeprecatedRouteNames()`, `nonDeprecatedRouteCount()`,
`routeHasNonDeprecatedRoutes()`, `firstNonDeprecatedRoute()`,
`firstNonDeprecatedRouteOrNull()`, `firstNonDeprecatedRouteName()`,
`firstNonDeprecatedRouteNameOrNull()`, `routesWithDocumentMetadata()`,
`routeNamesWithDocumentMetadata()`, `firstRouteWithDocumentMetadataOrNull()`,
`firstRouteNameWithDocumentMetadataOrNull()`, `routesWithoutDocumentMetadata()`,
`routeNamesWithoutDocumentMetadata()`,
`firstRouteWithoutDocumentMetadataOrNull()`,
`firstRouteNameWithoutDocumentMetadataOrNull()`,
`routesWithOperationIdMetadata()`, `routeNamesWithOperationIdMetadata()`,
`firstRouteWithOperationIdMetadataOrNull()`,
`firstRouteNameWithOperationIdMetadataOrNull()`,
`routesWithoutOperationIdMetadata()`, `routeNamesWithoutOperationIdMetadata()`,
`firstRouteWithoutOperationIdMetadataOrNull()`,
`firstRouteNameWithoutOperationIdMetadataOrNull()`, `routesWithOperationId()`,
`routeNamesWithOperationId()`, `firstRouteWithOperationIdOrNull(...)`,
`firstRouteNameWithOperationIdOrNull(...)`, `routesWithoutOperationId()`,
`routeNamesWithoutOperationId()`, `firstRouteWithoutOperationIdOrNull(...)`,
`firstRouteNameWithoutOperationIdOrNull(...)`, `routesWithSummary()`,
`routeNamesWithSummary()`, `firstRouteWithSummaryOrNull()`,
`firstRouteNameWithSummaryOrNull()`, `routesWithoutSummary()`,
`routeNamesWithoutSummary()`, `firstRouteWithoutSummaryOrNull()`,
`firstRouteNameWithoutSummaryOrNull()`, `routesWithDescription()`,
`routeNamesWithDescription()`, `firstRouteWithDescriptionOrNull()`,
`firstRouteNameWithDescriptionOrNull()`, `routesWithoutDescription()`,
`routeNamesWithoutDescription()`, `firstRouteWithoutDescriptionOrNull()`,
`firstRouteNameWithoutDescriptionOrNull()`, `routeRequestSchema()`,
`routeRequestSchemaOrNull()`, `routeHasRequestSchema()`,
`routeRequestTransport()`, `routeRequestTransportForRouteOrNull()`,
`routeHasRequestTransport()`, `routeRequestMediaType()`,
`routeRequestMediaTypeForRouteOrNull()`, `routeHasRequestMediaType()`,
`routeHasRequestMetadata()`, `routeResponseSchemas()`,
`routeResponseStatuses()`, `routeResponseMediaTypes()`, `routeResponseBodyFlags()`,
`routeResponseByStatus()`, `routeResponseByStatusOrNull()`,
`routeHasResponseSchema()`, `routeHasResponseStatus()`,
`routeResponses()`, `routeResponsesOrNull()`, `routeResponsesWithSchema()`, `routeResponsesWithMediaType()`,
`routeResponsesWithBody()`, `routeHasResponseMediaType()`, `routeHasResponseBody()`,
`routeResponseCount()`, `routeHasResponses()`, `routeFirstResponse()`,
`routeFirstResponseOrNull()`, and the response status/schema/media/body count,
presence, first-value, and `OrNull` first-value selectors,
`routesWithRequestMetadata()`, `routeNamesWithRequestMetadata()`,
`routesWithoutRequestMetadata()`, `routeNamesWithoutRequestMetadata()`,
`routesWithRequestSchema()`, `routeNamesWithRequestSchema()`,
`routesWithoutRequestSchema()`, `routeNamesWithoutRequestSchema()`,
`routesWithRequestTransport()`, `routeNamesWithRequestTransport()`,
`routesWithoutRequestTransport()`, `routeNamesWithoutRequestTransport()`,
`routesWithRequestMediaType()`, `routeNamesWithRequestMediaType()`,
`routesWithoutRequestMediaType()`, `routeNamesWithoutRequestMediaType()`,
request group summaries such as `routeCountWithRequestMetadata()`,
`routeHasRoutesWithRequestSchema(...)`,
`firstRouteWithRequestTransport(...)`, and
`firstRouteNameWithRequestMediaType(...)`, including matching request group
`OrNull` first selectors,
`routesWithResponses()`, `routeNamesWithResponses()`, `routesWithoutResponses()`,
`routeNamesWithoutResponses()`,
`routesWithResponseSchema()`, `routeNamesWithResponseSchema()`,
`routesWithoutResponseSchema()`, `routeNamesWithoutResponseSchema()`,
`routesWithResponseBody()`, `routeNamesWithResponseBody()`,
`routesWithoutResponseBody()`, `routeNamesWithoutResponseBody()`,
`routesWithResponseMediaType()`, `routeNamesWithResponseMediaType()`,
`routesWithoutResponseMediaType()`, `routeNamesWithoutResponseMediaType()`,
`routesWithResponseStatus()`, `routeNamesWithResponseStatus()`,
`routesWithoutResponseStatus()`, `routeNamesWithoutResponseStatus()`, and response
group summaries such as `routeCountWithResponseStatus(...)`,
`routeHasRoutesWithResponseSchema(...)`, `firstRouteWithResponseBody(...)`, and
`firstRouteNameWithResponses()`, including matching response group `OrNull`
first selectors, plus document/guard/audit-area/rate-limit/middleware group summaries such as
`routeCountWithDocumentMetadata()`, `routeHasRoutesWithGuard(...)`,
`firstRouteWithoutGuard()`, `routesWithAuditArea(...)`,
`routesWithRateLimitBy(...)`,
`routesWithMiddlewareGroup(...)`,
`firstRouteNameWithPermission(...)`, and `routeHasRoutesWithTag(...)` to type
route guards, menu builders, route docs, schema explorers, and route-audit
helpers against the generated manifest shape.
Use route-local `OrNull` first-value helpers such as
`routeFirstParamNameOrNull(...)`, `routeFirstResponseSchemaOrNull(...)`,
`routeFirstResponseWithSchemaOrNull(...)`, and
`routeFirstResponseWithBodyOrNull(...)` when UI stores model missing first
route metadata as `null`.
Use core route-group `OrNull` first-value helpers such as
`firstRouteByMethodOrNull(...)`, `firstRouteWithParamsOrNull()`,
`firstAuthRequiredRouteOrNull()`, `firstPublicRouteOrNull()`, and
`firstDeprecatedRouteOrNull()` for nullable first matches from backend-owned
route groups.
Use document metadata route-group `OrNull` helpers such as
`firstRouteWithDocumentMetadataOrNull()`,
`firstRouteWithOperationIdOrNull(...)`, `firstRouteWithSummaryOrNull()`, and
`firstRouteWithDescriptionOrNull()` when route docs or audits model missing
first matches as `null`.
Use guard, audit area, rate-limit, middleware group, permission, tag, and named-param route-group `OrNull` helpers such as
`firstRouteWithGuardOrNull(...)`, `firstRouteWithAuditAreaOrNull(...)`,
`firstRouteWithRateLimitByOrNull(...)`,
`firstRouteWithPermissionOrNull(...)`,
`firstRouteWithMiddlewareGroupOrNull(...)`, `firstRouteWithParamOrNull(...)`,
and `firstRouteWithTagOrNull(...)` when navigation, policy menus, and docs model
missing grouped matches as `null`.
Use request and response route-group `OrNull` helpers such as
`firstRouteWithRequestSchemaOrNull(...)`,
`firstRouteWithRequestTransportOrNull(...)`,
`firstRouteWithResponseSchemaOrNull(...)`, and
`firstRouteWithResponseStatusOrNull(...)` when schema explorers or generated
client dashboards model missing grouped matches as `null`.
The generated `RouteManifest` object and `RouteIds` tree are frozen at
runtime, including nested params, permissions, tags, and response metadata
arrays, so direct mutation cannot change backend-owned route metadata for every
importer. Route selector helpers clone entries, params, permissions, tags, and
response metadata before returning them, so menus, docs, and guards can annotate
selector results locally without mutating the generated manifest.

`types:export` also writes `HttpManifest.ts` from safe HTTP runtime
configuration. It includes body-size and request-timeout limits, security-header
flags, CORS methods/headers, CSRF cookie/header names, and global rate-limit
shape while omitting trusted-proxy CIDRs, rate-limit key prefixes, and raw CSP
policy text. Browser clients can import `HttpCsrfCookieName`,
`HttpCsrfHeaderName`, `httpCsrfEnabled()`, `httpCsrfCookieName()`,
`httpCsrfHeaderName()`, `httpCsrfCookieSecure()`, `httpCsrfCookiePath()`,
`httpCsrfCookieSameSite()`, `httpCsrfTokenFromCookie()`, `httpCsrfHeaders()`,
`httpCorsAllowedMethods()`,
`httpCorsAllowedMethodCount()`, `httpCorsHasAllowedMethods()`,
`httpCorsFirstAllowedMethod()`, `httpCorsFirstAllowedMethodOrNull()`,
`httpCorsAllowedHeaders()`,
`httpCorsAllowedHeaderCount()`, `httpCorsHasAllowedHeaders()`,
`httpCorsFirstAllowedHeader()`, `httpCorsFirstAllowedHeaderOrNull()`,
`httpCorsAllowsMethod()`,
`httpCorsAllowsHeader()`, `httpSecurityHeadersEnabled()`,
`httpRateLimitMaxRequests()`, `httpRateLimitWindowSeconds()`, and
`HttpRateLimitByValues`, `isHttpRateLimitBy()`, `httpRateLimitByOrNull()`,
`httpRateLimitBy()`, and `httpRateLimitByValues()`, `httpRateLimitByValueCount()`,
`httpRateLimitHasByValues()`, `httpRateLimitFirstByValue()`, and
`httpRateLimitFirstByValueOrNull()` instead of
duplicating HTTP integration constants, CORS allow-list checks, rate-limit mode
guards, list summary wrappers, first selector nullable wrappers, or cookie
parsing from config.
Generated HTTP runtime metadata and rate-limit mode lists are frozen at runtime,
so direct mutation cannot change backend-owned HTTP config. HTTP selector
helpers clone security-header, CORS, CSRF, rate-limit, and CORS allow-list
metadata before returning them, so diagnostics UI can apply local display state
to selector results.
`HttpManifest.ts` preserves the documented `0` sentinels for disabled global
body-size caps and request timeouts, exports backend-effective CSRF header names
and SameSite values, and rejects invalid CSRF cookie/header settings, wildcard
CORS origins combined with credentials, duplicate CORS allow-list values,
enabled global rate-limit zero values, global `by = "actor"` rate limits, and
HTTP numbers above JavaScript's safe integer range before helpers are written.

`types:export` also writes `AppManifest.ts` from frontend-safe `AppConfig`
fields. It includes the app name, environment label, environment kind and
derived booleans, configured timezone, and background shutdown timeout while
omitting the signing key. Dashboards, shell layouts, and generated clients can
import `ApplicationName`, `ApplicationEnvironment`,
`ApplicationEnvironmentKinds`, `ApplicationEnvironmentKind`,
`ApplicationTimezone`, `ApplicationBackgroundShutdownTimeoutMs`, `appName()`,
`appEnvironment()`, `isAppEnvironmentKind()`, `appEnvironmentKindOrNull()`,
`appEnvironmentKinds()`, `appEnvironmentKindCount()`, `appHasEnvironmentKinds()`,
`appFirstEnvironmentKind()`, `appFirstEnvironmentKindOrNull()`,
`appEnvironmentKind()`, `appEnvironmentKindIs()`, `appIsCustom()`,
`appTimezone()`, `appBackgroundShutdownTimeoutMs()`, and
`appIsProductionLike()` instead of copying app identity, environment kind lists,
environment checks, environment-list summary wrappers, first selector nullable
wrappers, or shutdown checks.
Generated app metadata and environment-kind lists are frozen at runtime, so
direct mutation cannot change backend-owned app metadata. `appEnvironmentKinds()`
returns a fresh list, so shells can add UI-only labels or grouping to local
derived state without mutating generated constants.
`AppManifest.ts` rejects blank or padded app names/environments and background
shutdown timeouts above JavaScript's safe integer range before helpers are
written. `background_shutdown_timeout_ms = 0` remains the documented immediate
shutdown sentinel.

When auth guards, policies, and authenticatable models are registered,
`types:export` writes `AuthManifest.ts`. It includes registered guard ids,
guard driver kind (`"bearer"` or `"session"`), the default guard when it is
registered, policy ids, and whether each guard has a registered
`Authenticatable` resolver. The same file also exports `AuthRuntimeManifest`
from safe auth config: token/session TTLs, session cookie integration metadata,
password-reset and email-verification expiry, lockout policy, and MFA settings,
without exporting token lengths, pruning internals, or raw cookie domains.
Frontend route guards, admin panels, and auth-state tooling can import
`AuthGuardIds`, `AuthPolicyIds`, `DefaultAuthGuard`,
`ConfiguredDefaultAuthGuard`, `AuthSessionCookieName`,
`AuthMfaPendingTokenTtlMinutes`, `AuthGuardKinds`, `isAuthGuardKind()`,
`isAuthGuardName()`, `isAuthPolicyName()`, `authGuardKindOrNull()`,
`authGuardNameOrNull()`, `authPolicyNameOrNull()`,
`authTokenGuardNameOrNull()`, `authGuardKinds()`, `authGuardNames()`,
`authPolicyNames()`, `authGuards()`,
`authPolicies()`, `authGuardKindCount()`, `authFirstGuardKind()`,
`authFirstGuardKindOrNull()`, `authGuardCount()`, `authHasGuards()`,
`authFirstGuard()`, `authFirstGuardOrNull()`, `authFirstGuardName()`,
`authFirstGuardNameOrNull()`, `authPolicyCount()`, `authHasPolicies()`,
`authFirstPolicy()`, `authFirstPolicyOrNull()`, `authFirstPolicyName()`,
`authFirstPolicyNameOrNull()`,
`authGuardManifestEntry()`, `authGuardManifestEntryOrNull()`,
`authPolicyManifestEntry()`, `authPolicyManifestEntryOrNull()`,
`authDefaultGuardName()`, `authConfiguredDefaultGuardName()`,
`authDefaultGuardManifestEntry()`, `authHasDefaultGuard()`,
`authNonDefaultGuardNames()`, `authNonDefaultGuards()`,
`authNonDefaultGuardCount()`, `authHasNonDefaultGuards()`,
`authFirstNonDefaultGuard()`, `authFirstNonDefaultGuardOrNull()`,
`authFirstNonDefaultGuardName()`, `authFirstNonDefaultGuardNameOrNull()`,
`authGuardNamesByKind()`,
`authGuardsByKind()`, `authGuardCountByKind()`,
`authHasGuardsByKind()`, `authFirstGuardByKind()`,
`authFirstGuardByKindOrNull()`, `authFirstGuardNameByKind()`,
`authFirstGuardNameByKindOrNull()`, `authGuardNamesWithoutKind()`,
`authGuardsWithoutKind()`, `authGuardCountWithoutKind()`,
`authHasGuardsWithoutKind()`, `authFirstGuardWithoutKind()`,
`authFirstGuardWithoutKindOrNull()`, `authFirstGuardNameWithoutKind()`,
`authFirstGuardNameWithoutKindOrNull()`, `authAuthenticatableGuardNames()`,
`authAuthenticatableGuards()`, `authAuthenticatableGuardCount()`,
`authHasAuthenticatableGuards()`, `authFirstAuthenticatableGuard()`,
`authFirstAuthenticatableGuardOrNull()`,
`authFirstAuthenticatableGuardName()`,
`authFirstAuthenticatableGuardNameOrNull()`,
`authNonAuthenticatableGuardNames()`,
`authNonAuthenticatableGuards()`, `authNonAuthenticatableGuardCount()`,
`authHasNonAuthenticatableGuards()`, `authFirstNonAuthenticatableGuard()`,
`authFirstNonAuthenticatableGuardOrNull()`,
`authFirstNonAuthenticatableGuardName()`,
`authFirstNonAuthenticatableGuardNameOrNull()`, `authGuardKind()`, and
`authGuardHasAuthenticatable()`, plus runtime helpers such as
`authBearerPrefix()`, `isAuthTokenGuardName()`,
`authTokenGuardManifestEntry()`, `authTokenGuardNames()`,
`authTokenGuardEntries()`, `authTokenGuardCount()`,
`authHasTokenGuards()`, `authFirstTokenGuard()`,
`authFirstTokenGuardOrNull()`, `authFirstTokenGuardName()`,
`authFirstTokenGuardNameOrNull()`,
`authAccessTokenTtlMinutes()`, `authSessionCookieName()`,
`authSessionCookiePath()`, `authSessionCookieSameSite()`,
`authSessionSlidingExpiry()`,
`authLockoutMaxFailures()`, `authLockoutWindowMinutes()`, `authMfaIssuer()`,
`authMfaGuardsRequiringRoles()`, `authMfaGuardCountRequiringRoles()`,
`authMfaTotalRequiredRoleCount()`, `authHasMfaGuardsRequiringRoles()`,
`authFirstMfaGuardRequiringRoles()`,
`authFirstMfaGuardRequiringRolesOrNull()`, `authMfaRequiredRolesForGuard()`,
`authMfaRequiredRoleCountForGuard()`, `authMfaFirstRequiredRoleForGuard()`,
and `authMfaFirstRequiredRoleForGuardOrNull()`
instead of
copying guard, policy, default-guard lookup, guard-kind lists, guard filters,
token guard names, token guard summaries,
guard-kind summary wrappers, first selector nullable wrappers, session-cookie
names, or auth-flow settings from Rust.
Generated auth manifests, runtime config, grouped guard/policy ids, and guard
kind lists are frozen at runtime, so direct mutation cannot change backend-owned
auth metadata for every importer. Auth selector helpers clone guard, policy,
token, session, lockout, MFA, and required-role metadata before returning it, so
auth UIs can annotate selector results locally without mutating generated
contracts.
`AuthRuntimeManifest` export uses backend-effective config for trimmed bearer
prefixes, lowercase cookie SameSite values, lockout clamps, MFA pending-token
TTL clamps, recovery-code count clamps, and blank MFA issuer fallback to
`app.name`. Direct auth runtime descriptors must keep exported strings trimmed,
session cookie settings valid, TTLs positive, and numeric values within
JavaScript's safe integer range before auth helpers are generated.
Guards generated with `make:guard` include the backend
`GuardId` constant and registration helper; policies generated with
`make:policy` do the same for `PolicyId`. Regenerate frontend types after
registering one so `AuthGuardIds` and `AuthPolicyIds` stay the auth SSOT.
Authenticatable metadata must point at a registered guard, so `types:export`
fails if a backend resolver is stale.
The `make:ids` scaffold starts the same auth ID module with `FoundryId` guard
ids and an `AppEnum` ability enum backed by `PermissionId`, so permission values
can be imported as generated TypeScript enum metadata instead of being copied
into forms or route guards.

`types:export` also writes `AuditManifest.ts` from `AuditConfig` and the
built-in audit event constants. It includes audit event types, the redacted
marker, sensitive field names, backend sensitive-name segment heuristics, and
the generated `auditFieldIsSensitive()` / `normalizeAuditFieldName()` helpers.
Audit-log dashboards can import `AuditEventTypes`, `AuditRedactedValue`, and
`auditRedactsSensitiveFields()`, `auditSensitiveFieldRedactionDisabled()`,
`auditEventTypes()`, `auditEventTypeOrNull()`, `auditEventTypeCount()`,
`auditHasEventTypes()`,
`auditFirstEventType()`, `auditFirstEventTypeOrNull()`, `auditRedactedValue()`,
`auditFieldIsConfiguredSensitive()`, `auditFieldMatchesSensitiveSegment()`,
`auditSensitiveFields()`, `auditSensitiveFieldCount()`,
`auditHasSensitiveFields()`, `auditFirstSensitiveField()`,
`auditFirstSensitiveFieldOrNull()`,
`auditSensitiveFieldSegments()`, `auditSensitiveFieldSegmentCount()`,
`auditHasSensitiveFieldSegments()`, `auditFirstSensitiveFieldSegment()`,
`auditFirstSensitiveFieldSegmentOrNull()`,
`isAuditSensitiveFieldName()`, `auditSensitiveFieldNameOrNull()`,
`isAuditSensitiveFieldSegment()`, `auditSensitiveFieldSegmentOrNull()`, and
`auditFieldIsSensitive()` instead of copying audit event strings, segment
heuristics, redaction rules, redaction-disabled checks, list summary wrappers, or
first selector nullable wrappers.
Generated audit metadata and redaction lists are frozen at runtime, so dashboards
cannot mutate backend-owned audit policy through direct constants. Audit
selector helpers clone event, sensitive-field, and sensitive-segment lists
before returning them, so dashboards can attach local redaction explanations to
selector results.
Audit sensitive fields are exported after the same backend normalization used by
the Rust redaction policy, so values such as `cardToken` become `card_token` and
duplicates collapse before helpers are written. Direct manifest descriptors must
use non-blank backend-normalized sensitive field names.

`types:export` also writes `LoggingManifest.ts` from frontend-safe
`LoggingConfig` fields. It includes the configured log level, output format,
log directory, and retention days while reusing the generated `LogLevel` union.
Admin dashboards and dev tooling can import `LoggingManifest`,
`LoggingFormats`, `LoggingLogDir`, `LoggingRetentionDays`,
`isLoggingFormat()`, `loggingFormatOrNull()`, `loggingFormats()`, `loggingFormatCount()`,
`loggingHasFormats()`, `loggingFirstFormat()`, `loggingFirstFormatOrNull()`,
`loggingUsesJson()`, `loggingUsesText()`, `loggingLogDirectory()`,
`loggingHasLogDirectory()`, `loggingWritesFiles()`,
`loggingFileOutputDisabled()`, `loggingRetentionEnabled()`, and
`loggingRetentionDisabled()` instead of copying logging config, log-directory
constants, file-output disabled checks, format guards, format-list summary
wrappers, or first selector nullable wrappers.
Generated logging metadata and format lists are frozen at runtime, so tooling
cannot mutate backend-owned logging config through direct constants.
`loggingFormats()` returns a fresh list, so tooling can add local presentation
metadata to selector results.
`logging.log_dir = ""` remains the stdout-only sentinel, and
`logging.retention_days = 0` remains the keep-forever sentinel. Non-empty log
directories must be trimmed and free of control characters before logging
helpers are generated.

When storage disks are configured, `types:export` writes `StorageManifest.ts`.
It includes each logical disk name, driver key, visibility, and the default disk
flag without exposing roots, buckets, endpoints, or credentials. The same file
also exports `StorageRuntimeManifest` from browser-safe storage config:
configured default disk, upload caps, image decode caps, temp-upload pruning
settings, and attachment-orphan policy. Upload forms, storage browsers, and
admin tooling can import `StorageDiskIds`, `DefaultStorageDisk`,
`ConfiguredDefaultStorageDisk`, `StorageMaxUploadFileSizeBytes`,
`isStorageDiskName()`, `isStorageDiskDriverName()`, `isStorageDiskVisibility()`,
`storageDiskNameOrNull()`, `storageDiskDriverNameOrNull()`,
`storageDiskVisibilityOrNull()`, `storageDiskManifestEntry()`,
`storageDiskManifestEntryOrNull()`, `storageDiskNames()`, `storageDisks()`, `storageDiskCount()`,
`storageDiskHasEntries()`, `storageDiskFirstEntry()`,
`storageDiskFirstEntryOrNull()`, `storageDiskFirstName()`,
`storageDiskFirstNameOrNull()`,
`storageDefaultDiskName()`, `storageConfiguredDefaultDiskName()`,
`storageDefaultDiskManifestEntry()`, `storageHasDefaultDisk()`,
`storageNonDefaultDiskNames()`, `storageNonDefaultDisks()`,
`storageNonDefaultDiskCount()`, `storageHasNonDefaultDisks()`,
`storageFirstNonDefaultDisk()`, `storageFirstNonDefaultDiskOrNull()`,
`storageFirstNonDefaultDiskName()`, `storageFirstNonDefaultDiskNameOrNull()`,
`storageDiskDriverNames()`, `isStorageDiskDriverName()`,
`storageDiskDriverNameCount()`, `storageHasDiskDriverNames()`,
`storageFirstDiskDriverName()`, `storageFirstDiskDriverNameOrNull()`,
`storageDiskNamesByDriver()`,
`storageDisksByDriver()`, `storageDiskCountByDriver()`,
`storageHasDisksByDriver()`, `storageFirstDiskByDriver()`,
`storageFirstDiskByDriverOrNull()`, `storageFirstDiskNameByDriver()`,
`storageFirstDiskNameByDriverOrNull()`, `storageDiskNamesWithoutDriver()`,
`storageDisksWithoutDriver()`, `storageDiskCountWithoutDriver()`,
`storageHasDisksWithoutDriver()`, `storageFirstDiskWithoutDriver()`,
`storageFirstDiskWithoutDriverOrNull()`,
`storageFirstDiskNameWithoutDriver()`, `storageFirstDiskNameWithoutDriverOrNull()`,
`storageDiskVisibilityNames()`, `isStorageDiskVisibility()`,
`storageDiskVisibilityNameCount()`, `storageHasDiskVisibilityNames()`,
`storageFirstDiskVisibilityName()`, `storageFirstDiskVisibilityNameOrNull()`,
`storageDiskNamesByVisibility()`,
`storageDiskCountByVisibility()`, `storageHasDisksByVisibility()`,
`storageFirstDiskByVisibility()`, `storageFirstDiskByVisibilityOrNull()`,
`storageFirstDiskNameByVisibility()`, `storageFirstDiskNameByVisibilityOrNull()`,
`storagePublicDiskNames()`, `storagePublicDisks()`,
`storagePublicDiskCount()`, `storageHasPublicDisks()`,
`storageFirstPublicDisk()`, `storageFirstPublicDiskOrNull()`,
`storageFirstPublicDiskName()`, `storageFirstPublicDiskNameOrNull()`,
`storagePrivateDiskNames()`, `storagePrivateDisks()`,
`storagePrivateDiskCount()`, `storageHasPrivateDisks()`,
`storageFirstPrivateDisk()`, `storageFirstPrivateDiskOrNull()`,
`storageFirstPrivateDiskName()`, `storageFirstPrivateDiskNameOrNull()`,
`storageMaxUploadSizeBytes()`, `storageMaxUploadFileSizeBytes()`,
`storageMaxUploadFiles()`, `storageUploadLimits()`, `storageImageLimits()`, and
`storageDiskVisibility()`
instead of hard-coding configured disk names, default disk lookups, driver
filters, visibility filters, filtered-list summary wrappers, first selector
nullable wrappers, or upload limits.
Runtime manifest export requires a non-empty trimmed configured default disk,
matching default disk metadata when disk descriptors are present, upload caps
that stay within JavaScript's safe integer range, a per-file upload cap no
larger than the total cap when both are enabled, positive prune intervals when
temp/orphan maintenance is enabled, and a safe relative `attachment_orphan_prefix`.
Documented `0` sentinels for upload caps, temp retention, and image decode caps
remain valid.
Generated storage manifests, runtime config, and disk id trees are frozen at
runtime, so direct mutation cannot change backend-owned storage metadata.
Storage selector helpers clone disk entries and upload/image/orphan runtime
config before returning them, so storage browsers can attach local upload or
presentation state to selector results.

When email mailers are configured, `types:export` writes `EmailManifest.ts`. It
includes each logical mailer name, driver key, and the default mailer flag
without exposing provider endpoints, sender settings, templates, or
credentials. The same file also exports `EmailRuntimeManifest` from browser-safe
email config: configured default mailer, outbound queue, and attachment byte
limits. Admin tooling and preview/test-send forms can import `EmailMailerIds`,
`DefaultEmailMailer`, `ConfiguredDefaultEmailMailer`, `EmailDefaultQueue`,
`isEmailMailerName()`, `emailMailerNameOrNull()`, `emailMailerManifestEntry()`,
`emailMailerManifestEntryOrNull()`, `emailMailerNames()`, `emailMailers()`, `emailMailerCount()`,
`emailMailerHasEntries()`, `emailMailerFirstEntry()`,
`emailMailerFirstEntryOrNull()`, `emailMailerFirstName()`,
`emailMailerFirstNameOrNull()`,
`emailDefaultMailerName()`, `emailConfiguredDefaultMailerName()`,
`emailDefaultMailerManifestEntry()`, `emailHasDefaultMailer()`,
`emailNonDefaultMailerNames()`, `emailNonDefaultMailers()`,
`emailNonDefaultMailerCount()`, `emailHasNonDefaultMailers()`,
`emailFirstNonDefaultMailer()`, `emailFirstNonDefaultMailerOrNull()`,
`emailFirstNonDefaultMailerName()`, `emailFirstNonDefaultMailerNameOrNull()`,
`emailMailerDriverNames()`, `isEmailMailerDriverName()`,
`emailMailerDriverNameOrNull()`,
`emailMailerDriverNameCount()`, `emailHasMailerDriverNames()`,
`emailFirstMailerDriverName()`, `emailFirstMailerDriverNameOrNull()`,
`emailMailerNamesByDriver()`,
`emailMailersByDriver()`, `emailMailerCountByDriver()`,
`emailHasMailersByDriver()`, `emailFirstMailerByDriver()`,
`emailFirstMailerByDriverOrNull()`, `emailFirstMailerNameByDriver()`,
`emailFirstMailerNameByDriverOrNull()`, `emailMailerNamesWithoutDriver()`,
`emailMailersWithoutDriver()`, `emailMailerCountWithoutDriver()`,
`emailHasMailersWithoutDriver()`, `emailFirstMailerWithoutDriver()`,
`emailFirstMailerWithoutDriverOrNull()`,
`emailFirstMailerNameWithoutDriver()`, `emailFirstMailerNameWithoutDriverOrNull()`,
`emailMailerDriver()`,
`emailDefaultQueue()`, `emailMaxAttachmentBytes()`,
`emailMaxTotalAttachmentBytes()`, and `emailAttachmentLimits()` instead of
copying configured mailer names, default-mailer lookups, queue names, driver
guards, driver filters, driver-list summary wrappers, first selector nullable
wrappers, or attachment caps from Rust.
Runtime manifest export requires a non-empty trimmed configured default mailer,
a non-empty trimmed queue, matching default mailer metadata when mailer
descriptors are present, attachment caps within JavaScript's safe integer range,
and a per-file attachment cap no larger than the total attachment cap when both
are enabled. `max_attachment_bytes = 0` and `max_total_attachment_bytes = 0`
remain valid no-cap sentinels.
Generated email manifests, runtime config, and mailer id trees are frozen at
runtime, so direct mutation cannot change backend-owned mailer metadata. Email
selector helpers clone mailer entries and attachment-limit config before
returning them, so mailer admin tools can add local preview or delivery-state
annotations to selector results.

When i18n catalogs are loaded, `types:export` writes `I18nManifest.ts`. It
includes the configured default locale, fallback locale, and loaded locale names
without exporting translation keys or catalog text. Locale switchers, admin
preview tools, and frontend locale guards can import `I18nLocaleIds`,
`I18nDefaultLocale`, `I18nFallbackLocale`, `I18nLocaleMap`,
`isI18nLocaleName()`, `i18nLocaleNameOrNull()`, `i18nLocaleManifestEntry()`,
`i18nLocaleManifestEntryOrNull()`, `i18nLocaleMap()`, `i18nDefaultLocale()`,
`i18nFallbackLocale()`, `i18nLocaleNames()`, `i18nLocales()`,
`i18nLocaleCount()`, `i18nLocaleHasEntries()`, `i18nLocaleFirstEntry()`,
`i18nLocaleFirstEntryOrNull()`, `i18nLocaleFirstName()`,
`i18nLocaleFirstNameOrNull()`,
`i18nDefaultLocaleManifestEntry()`, `i18nFallbackLocaleManifestEntry()`,
`i18nDefaultLocaleLoaded()`, `i18nFallbackLocaleLoaded()`,
`i18nNonDefaultLocaleNames()`, `i18nNonDefaultLocales()`,
`i18nNonDefaultLocaleCount()`, `i18nHasNonDefaultLocales()`,
`i18nFirstNonDefaultLocale()`, `i18nFirstNonDefaultLocaleOrNull()`,
`i18nFirstNonDefaultLocaleName()`, `i18nFirstNonDefaultLocaleNameOrNull()`,
`i18nNonFallbackLocaleNames()`, `i18nNonFallbackLocales()`,
`i18nNonFallbackLocaleCount()`, `i18nHasNonFallbackLocales()`,
`i18nFirstNonFallbackLocale()`, `i18nFirstNonFallbackLocaleOrNull()`,
`i18nFirstNonFallbackLocaleName()`, `i18nFirstNonFallbackLocaleNameOrNull()`,
and `i18nResolveLocale()` instead of copying locale maps, locale lists,
default/fallback names, loaded checks, default/fallback exclusion filters, first
selector nullable wrappers, or Accept-Language fallback logic from config.
Generated i18n locale maps, manifests, and locale id trees are frozen at
runtime, so direct mutation cannot change backend-owned locale metadata. i18n
selector helpers clone locale entries and locale lists before returning them,
so locale switchers can add local labels, sort preferences, or preview state to
selector results.

When readiness checks are registered, `types:export` writes
`ReadinessManifest.ts`. It includes each probe id and whether the probe is a
Foundry built-in, without exporting current health state or error messages.
Operational dashboards can import `ReadinessProbeIds`,
`isReadinessProbeName()`, `readinessProbeNameOrNull()`,
`isBuiltInReadinessProbeName()`, `builtInReadinessProbeNameOrNull()`,
`isCustomReadinessProbeName()`, `customReadinessProbeNameOrNull()`, `readinessProbeManifestEntry()`,
`readinessProbeManifestEntryOrNull()`,
`readinessProbeNames()`, `readinessProbeEntries()`,
`readinessProbeCount()`, `readinessProbeHasEntries()`,
`readinessProbeFirstEntry()`, `readinessProbeFirstEntryOrNull()`,
`readinessProbeFirstName()`, `readinessProbeFirstNameOrNull()`,
`readinessProbeIsBuiltIn()`, `readinessProbeIsCustom()`,
`builtInReadinessProbeNames()`, `builtInReadinessProbes()`,
`builtInReadinessProbeCount()`, `readinessHasBuiltInProbes()`,
`firstBuiltInReadinessProbe()`, `firstBuiltInReadinessProbeOrNull()`,
`firstBuiltInReadinessProbeName()`, `firstBuiltInReadinessProbeNameOrNull()`,
`customReadinessProbeNames()`, `customReadinessProbes()`,
`customReadinessProbeCount()`, `readinessHasCustomProbes()`,
`firstCustomReadinessProbe()`, `firstCustomReadinessProbeOrNull()`,
`firstCustomReadinessProbeName()`, and
`firstCustomReadinessProbeNameOrNull()` instead of copying probe ids,
built-in/custom filters, built-in/custom summary wrappers, or first selector
nullable wrappers from providers or plugins.
Generated readiness manifests and probe id trees are frozen at runtime, so
direct mutation cannot change backend-owned probe metadata. Readiness selector
helpers clone probe entries before returning them, so dashboards can add live
health results or local display state to selector results.

When settings are registered with `SettingDefinition`, `types:export` writes
`SettingManifest.ts`. It includes setting keys, widget type, group, label,
description, sort order, public/private status, and parameter metadata without
exporting current setting values. Admin forms and public-settings clients can
import `SettingIds`, `SettingGroups`, `SettingManifest`, `settingsInGroup()`,
`isSettingName()`, `settingNameOrNull()`, `isSettingGroupName()`,
`settingGroupNameOrNull()`, `settingNames()`, `settingGroupNames()`,
`settingEntries()`, `settingCount()`, `settingHasEntries()`,
`settingFirstEntry()`, `settingFirstEntryOrNull()`, `settingFirstName()`,
`settingFirstNameOrNull()`, `settingNamesInGroup()`,
`settingGroupCount()`, `settingHasGroups()`, `settingFirstGroupName()`,
`settingFirstGroupNameOrNull()`,
`settingCountInGroup()`, `settingHasGroupEntries()`,
`settingFirstEntryInGroup()`, `settingFirstEntryInGroupOrNull()`,
`settingFirstNameInGroup()`, `settingFirstNameInGroupOrNull()`,
`settingNamesWithoutGroup()`, `settingsWithoutGroup()`,
`settingCountWithoutGroup()`, `settingHasSettingsWithoutGroup()`,
`firstSettingWithoutGroup()`, `firstSettingWithoutGroupOrNull()`,
`firstSettingNameWithoutGroup()`, `firstSettingNameWithoutGroupOrNull()`,
`publicSettingNames()`, `publicSettings()`, `publicSettingCount()`,
`settingHasPublicSettings()`, `firstPublicSetting()`,
`firstPublicSettingOrNull()`, `firstPublicSettingName()`,
`firstPublicSettingNameOrNull()`, `privateSettingNames()`, `privateSettings()`,
`privateSettingCount()`, `settingHasPrivateSettings()`,
`firstPrivateSetting()`, `firstPrivateSettingOrNull()`,
`firstPrivateSettingName()`, `firstPrivateSettingNameOrNull()`,
`settingNamesByType()`, `settingsByType()`, `settingCountByType()`,
`settingHasTypeEntries()`, `firstSettingByType()`,
`firstSettingByTypeOrNull()`, `firstSettingNameByType()`,
`firstSettingNameByTypeOrNull()`, `settingNamesWithoutType()`,
`settingsWithoutType()`, `settingCountWithoutType()`,
`settingHasSettingsWithoutType()`, `firstSettingWithoutType()`,
`firstSettingWithoutTypeOrNull()`, `firstSettingNameWithoutType()`,
`firstSettingNameWithoutTypeOrNull()`, `settingManifestEntry()`,
`settingManifestEntryOrNull()`, `settingLabel()`,
`settingDescription()`, `settingParameters()`, `settingParameterNames()`,
`settingParameterNameCount()`, `settingTotalParameterNameCount()`,
`settingHasParameters()`,
`settingFirstParameterName()`, `settingFirstParameterNameOrNull()`,
`settingOptions()`, `settingOptionValues()`, `settingOptionCount()`,
`settingTotalOptionCount()`, `settingHasOptions()`,
`settingFirstOption()`, `settingFirstOptionOrNull()`,
`settingFirstOptionValue()`, `settingFirstOptionValueOrNull()`,
`settingOptionLabels()`, `settingFirstOptionLabel()`,
`settingFirstOptionLabelOrNull()`, `settingOptionForValue()`,
`settingOptionLabelForValue()`, `settingHasOptionValue()`,
`settingsWithOptions()`,
`settingNamesWithOptions()`, `settingCountWithOptions()`,
`settingHasSettingsWithOptions()`, `firstSettingWithOptions()`,
`firstSettingWithOptionsOrNull()`, `firstSettingNameWithOptions()`,
`firstSettingNameWithOptionsOrNull()`, `settingsWithoutOptions()`,
`settingNamesWithoutOptions()`, `settingCountWithoutOptions()`,
`settingHasSettingsWithoutOptions()`, `firstSettingWithoutOptions()`,
`firstSettingWithoutOptionsOrNull()`, `firstSettingNameWithoutOptions()`,
`firstSettingNameWithoutOptionsOrNull()`, `settingSortOrder()`, and
`SettingValueFor<...>` instead of copying setting key strings, group tabs,
public/private/type filters, filtered-list summary wrappers, parameter names,
select/multiselect option lists, no-option filters, option-value lookups, first
selector nullable wrappers, or form metadata from seeders.
Generated setting manifests, setting id trees, and setting groups are frozen at
runtime, so direct mutation cannot change backend-owned setting metadata.
Setting selector helpers clone entries and deep-clone JSON parameter metadata
before returning them, so admin UIs can add local field state, draft values, or
layout overrides to selector results.
`SettingManifest.ts` validates backend-owned option metadata before helpers are
written. Select and multiselect definitions must declare a non-empty
`parameters.options` array, each option must be an object with a `value` field
and a non-blank trimmed string `label`, and duplicate option values are rejected
so `settingOptionForValue()` remains unambiguous.

`types:export` also writes `CacheManifest.ts` from the resolved cache and Redis
configuration. It includes the cache driver, error mode, prefix, TTL/key limits,
remember-lock settings, whether Redis is configured, and the resolved Redis
namespace, without exporting the Redis URL. Admin dashboards and dev tools can
import `CacheManifest`, `CacheUsesRedis`, `CacheRedisNamespace`,
`CacheDrivers`, `CacheErrorModes`, `isCacheDriver()`, `isCacheErrorMode()`,
`cacheDriverOrNull()`, `cacheErrorModeOrNull()`, `cacheDrivers()`,
`cacheDriverCount()`, `cacheHasDrivers()`,
`cacheFirstDriver()`, `cacheFirstDriverOrNull()`, `cacheErrorModes()`,
`cacheErrorModeCount()`,
`cacheHasErrorModes()`, `cacheFirstErrorMode()`, `cacheFirstErrorModeOrNull()`,
`cacheDriver()`, `cacheErrorMode()`,
`cacheUsesRedis()`, `cacheUsesMemory()`, `cacheRedisConfigured()`,
`cacheRedisNamespace()`, `cacheHasRedisNamespace()`, `cacheMaxEntries()`,
`cacheKeyMaxLength()`, `cacheKeyPrefix()`, `cacheHasKeyPrefix()`, `cacheIsStrict()`,
`cacheIsFailOpen()`, `cacheRememberLockTtlMs()`,
`cacheRememberLockWaitTimeoutMs()`, `cacheRememberLockPollMs()`, and
`cacheRememberLockTiming()` instead of duplicating cache config, driver lists,
error-mode guards, key-prefix presence checks, remember-lock timing values, or
list summary wrappers and first selector nullable wrappers.
Generated cache metadata, driver lists, and error-mode lists are frozen at
runtime, so direct mutation cannot change backend-owned cache config.
`cacheDrivers()`, `cacheErrorModes()`, and `cacheRememberLockTiming()` return
fresh values, so dashboards can add local annotations to selector results.
`CacheManifest.ts` keeps `key_max_length = 0` as the documented no-cap sentinel
and keeps disabled distributed-lock timing values as configured. When
distributed remember locks are enabled, exported lock TTL and poll intervals use
the same positive clamp as the backend. Cache manifest export rejects memory
cache `max_entries = 0` and cache timing/limit values above JavaScript's safe
integer range before frontend cache helpers are generated.

`types:export` also writes `DatabaseManifest.ts` from browser-safe database
configuration. It includes pagination defaults used by the direct
`Pagination` extractor while omitting database URLs, schema names, migration
tables/paths, pool settings, and SQL observability internals. Frontend list
screens and generated clients can import `DatabaseDefaultPerPage`,
`databaseDefaultPage()`, `databaseDefaultPerPage()`,
`databaseDefaultPagination()`, `databaseNormalizePagination()`,
`databasePaginationWithDefaults()`, `DatabasePaginationQueryParamNames`,
`databasePaginationQueryParamNameMap()`,
`databasePaginationPageQueryParamName()`,
`databasePaginationPerPageQueryParamName()`,
`databasePaginationQueryParamNames()`,
`databasePaginationQueryParamNameCount()`,
`databaseHasPaginationQueryParamNames()`,
`databaseHasPaginationQueryParamName()`,
`databasePaginationQueryParamNameOrNull()`,
`databaseFirstPaginationQueryParamName()`,
`databaseFirstPaginationQueryParamNameOrNull()`,
`databasePaginationQueryParams()`,
and `databasePaginationFromQueryParams()` instead of copying
`database.default_per_page`, hand-serializing `page` / `per_page` query
strings, local query-param summary wrappers, or hand-parsing pagination state
from URLs.
Generated safe database pagination metadata and query-param names are frozen at
runtime, so direct mutation cannot change backend-owned pagination defaults.
Database pagination helpers return fresh pagination/query-name values, so list
builders can add UI-only labels to their local derived state.
`DatabaseManifest.ts` exports the effective backend pagination defaults after
the same positive clamp used by the direct `Pagination` extractor, so a
configured `database.default_per_page = 0` is emitted as `1`. Direct manifest
descriptors must use positive defaults that fit within JavaScript's safe integer
range before frontend pagination helpers are generated.

`types:export` also writes `ObservabilityManifest.ts` from
`ObservabilityConfig`. It includes the `/_foundry` base path, enable/capture
flags, HTTP sample and WebSocket channel retention sizes, tracing flag, service
name, and whether WebSocket history includes payloads. It intentionally omits
the OTLP endpoint so frontend dashboards and dev tools can import
`ObservabilityManifest`, `ObservabilityBasePath`,
`ObservabilityStaticEndpointNames`, `ObservabilityChannelEndpointNames`,
`ObservabilityEndpointNames`, `ObservabilityEndpointPaths`,
`isObservabilityStaticEndpointName()`, `isObservabilityChannelEndpointName()`,
`isObservabilityEndpointName()`, `observabilityStaticEndpointNameOrNull()`,
`observabilityChannelEndpointNameOrNull()`, `observabilityEndpointNameOrNull()`,
`observabilityServiceName()`,
`observabilityEnabled()`, `observabilityDisabled()`,
`observabilityCaptureEnabled()`, `observabilityCaptureDisabled()`,
`observabilityTracingEnabled()`, `observabilityTracingDisabled()`,
`observabilityEndpointNames()`,
`observabilityStaticEndpointNameCount()`,
`observabilityFirstStaticEndpointName()`,
`observabilityFirstStaticEndpointNameOrNull()`,
`observabilityChannelEndpointNameCount()`,
`observabilityFirstChannelEndpointName()`,
`observabilityFirstChannelEndpointNameOrNull()`, `observabilityEndpointNameCount()`,
`observabilityFirstEndpointName()`, `observabilityFirstEndpointNameOrNull()`,
`observabilityPathTemplate()`,
`observabilityPathTemplateOrNull()`, `observabilityPath()`,
`observabilityPathOrNull()`, `observabilityWebSocketPayloadsIncluded()`, and
`observabilityWebSocketPayloadsExcluded()` without
copying backend-only collector config, endpoint-name guards, endpoint-list
summary wrappers, first selector nullable wrappers, enabled/disabled checks,
payload-visibility checks, or hardcoding built-in `/_foundry/*` route strings.
`observabilityPathTemplateOrNull()` returns `null` for unknown runtime endpoint
names, and `observabilityPathOrNull()` also returns `null` when a channel
endpoint is missing `params.channel`.
Generated observability metadata, endpoint-name lists, and endpoint path maps are
frozen at runtime, so direct mutation cannot change backend-owned operations
metadata. Observability selector helpers clone endpoint-name lists and endpoint
path maps before returning them, so dashboards can decorate endpoint rows
locally.
The generated manifest exports the backend-normalized observability base path:
missing leading slashes are added and trailing slashes are removed before
endpoint helpers are written. Runtime retention values must fit within
JavaScript's safe integer range, while `0` remains the documented sentinel for
disabling retained HTTP samples or retained per-channel WebSocket counters.
Direct observability manifest descriptors must also use literal normalized base
paths and non-blank, trimmed service names before frontend helpers are
generated.

When the app registers WebSocket routes, `types:export` also writes
`WebSocketChannelManifest.ts`. It includes each channel id, presence setting,
replay count, client-event setting, guard, and permissions, plus
`WebSocketChannelIds` and typed frame builders such as `subscribeToChannel()`
and `messageToChannel()` for frontend clients that should not duplicate backend
channel strings or hand-build protocol JSON. The same file includes
frontend-safe runtime config from `WebSocketConfig` via
`WebSocketRuntimeManifest`, `WebSocketPath`, heartbeat timing constants,
query-token metadata, client-facing limits, history caps, `webSocketPath()`,
`webSocketHeartbeatIntervalSeconds()`, `webSocketHeartbeatTimeoutSeconds()`,
`webSocketHeartbeat()`, `webSocketLimits()`, `webSocketQueryToken()`,
`webSocketQueryTokenName()`,
`webSocketHistory()`, and `webSocketUrl()`.
It intentionally omits deployment and transport internals such as bind
host/port, allowed origins, frame/write-buffer limits, and outbound buffer
size. Runtime manifest export requires a non-empty trimmed WebSocket path that
starts with `/`, positive heartbeat timings, a positive history buffer size, and
a valid query-token name when query tokens are enabled. Exported numeric runtime
values must also fit within JavaScript's safe integer range before frontend
socket helpers are generated. The generated `ClientAction`
AppEnum owns the broad WebSocket client action union, `WebSocketClientAction`
aliases it from the channel manifest, and `WebSocketErrorPayload` aliases the
generated `ErrorResponse` contract. ACK and presence protocol payloads are also
backend-owned via `WebSocketAckPayload`, `WebSocketAckStatus`,
`WebSocketPresenceJoinPayload`, and `WebSocketPresenceLeavePayload`. Raw
protocol and metadata records (`ClientMessage`, `ServerMessage`,
`PresenceInfo`, and `WebSocketChannelDescriptor`) are also exported for tooling
and custom clients.
Like `RouteIds`, dotted channel ids are grouped and camelCased for property access. The generated
`webSocketChannelManifestEntry()`, `webSocketChannelManifestEntryOrNull()`,
`webSocketChannelNameOrNull()`, `webSocketPresenceChannelNameOrNull()`,
`webSocketClientEventChannelNameOrNull()`,
`webSocketChannelNames()`, `webSocketChannelCount()`,
`webSocketHasChannels()`, `webSocketFirstChannel()`,
`webSocketFirstChannelOrNull()`, `webSocketFirstChannelName()`,
`webSocketFirstChannelNameOrNull()`, `webSocketPresenceChannelNames()`,
`webSocketNonPresenceChannelNames()`, `webSocketClientEventChannelNames()`,
`webSocketChannelsAllowingClientEvents()`, `webSocketChannelNamesAllowingClientEvents()`,
`webSocketChannelsDisallowingClientEvents()`,
`webSocketChannelNamesDisallowingClientEvents()`, `webSocketChannelsWithPermission()`,
`webSocketChannelPermissionCount()`, `webSocketChannelHasPermissions()`,
`webSocketChannelTotalPermissionCount()`, `webSocketChannelFirstPermission()`,
`webSocketChannelFirstPermissionOrNull()`,
`webSocketChannelsWithPermissions()`, `webSocketChannelNamesWithPermissions()`,
`webSocketChannelsWithoutPermissions()`, `webSocketChannelNamesWithoutPermissions()`,
`webSocketChannelGuard()`, `webSocketChannelHasGuard()`,
`webSocketChannelsWithGuard()`, `webSocketChannelNamesWithGuard()`,
`webSocketChannelsWithAnyGuard()`, `webSocketChannelNamesWithAnyGuard()`,
`webSocketChannelsWithoutGuard()`, `webSocketChannelNamesWithoutGuard()`,
`webSocketChannelHasReplay()`, `webSocketChannelsWithReplay()`,
`webSocketChannelNamesWithReplay()`, `webSocketChannelsWithoutReplay()`,
`webSocketChannelNamesWithoutReplay()`, `webSocketChannelTotalReplayCount()`,
`webSocketChannelClientEventCount()`, `webSocketChannelHasClientEvents()`,
`webSocketChannelTotalClientEventCount()`, `webSocketChannelFirstClientEvent()`,
`webSocketChannelFirstClientEventOrNull()`,
`webSocketChannelsWithClientEvents()`, `webSocketChannelNamesWithClientEvents()`,
`webSocketChannelsWithoutClientEvents()`,
`webSocketChannelNamesWithoutClientEvents()`, `webSocketChannelServerEventCount()`,
`webSocketChannelTotalServerEventCount()`,
`webSocketChannelHasServerEvents()`, `webSocketChannelFirstServerEvent()`,
`webSocketChannelFirstServerEventOrNull()`,
`webSocketChannelsWithServerEvents()`, `webSocketChannelNamesWithServerEvents()`,
`webSocketChannelsWithoutServerEvents()`, `webSocketChannelNamesWithoutServerEvents()`,
`webSocketChannelAllowsClientEvent()`, and
`webSocketChannelAllowsServerEvent()` helpers let clients and dashboards filter
backend-owned channel metadata without hand-scanning `WebSocketChannelManifest`.
Channel-group count/presence/first selectors are also generated for
presence/non-presence, client-event-capable, auth-required/public, guard, replay,
permission, client-event, and server-event groups, so dashboards can summarize
those filtered groups without local `.length`, `.length > 0`, or `[0]` wrappers.
Nullable first-selector companions such as `webSocketFirstPresenceChannelOrNull()`,
`webSocketFirstChannelWithGuardOrNull()`, `webSocketChannelFirstPermissionOrNull()`,
`webSocketChannelFirstClientEventOrNull()`, and `webSocketChannelFirstServerEventOrNull()`
return explicit `null` for empty backend-owned channel metadata lists.
Generated WebSocket channel, payload, protocol, runtime, and channel-id
metadata are frozen at runtime, so direct mutation cannot change backend-owned
realtime metadata for every importer. WebSocket selector helpers clone channel
entries, event/permission arrays, and runtime sub-config objects before
returning them, so socket clients and dashboards can annotate selector results
locally without mutating generated contracts.
The generated `WebSocketClientEventChannelName` type is derived from backend
`allow_client_events(true)` settings, `isWebSocketClientEventChannelName()`
narrows unknown channel strings, and `clientEventToChannel()` only accepts those
channels. When a channel registers `client_event(...)` /
`client_events(...)`, the generated `WebSocketClientEventName` type narrows the
event argument to that backend-owned allowlist; channels that only use
`allow_client_events(true)` remain open to any event string.
`webSocketClientEventNameOrNull()` safely normalizes runtime client-event
strings for a client-event-capable channel. Channels can also
register `server_event(...)` / `server_events(...)` so
`WebSocketServerEventName` narrows incoming server-frame event names for
registered channels, and `webSocketServerEventNameOrNull()` safely narrows
runtime server-event strings by channel. Declared server events are also enforced by
`WebSocketPublisher` in kernels booted with registered WebSocket routes,
including queued jobs and scheduler tasks; channels without declarations remain
open to arbitrary backend-published event strings.
Apps can register app-event payload DTOs with `TsWebSocketPayload::server(...)`
or `TsWebSocketPayload::client_event(...)`. `types:export` adds those contracts
to `WebSocketPayloadManifest`, `WebSocketServerPayloadMap`, and
`WebSocketClientEventPayloadMap`, and exports typed aliases such as
`TypedWebSocketAppServerFrame<"chat.rooms", "message">` plus
`typedClientEventToChannel()` for payload-checked client events. Payload
metadata must point at a registered channel and must agree with explicit
`client_events` / `server_events` allowlists when a channel uses them.
Payload tooling can use `webSocketPayloadEntries()`, `webSocketPayloadNames()`,
`webSocketPayloadCount()`, `webSocketHasPayloads()`,
`webSocketFirstPayload()`, `webSocketFirstPayloadOrNull()`,
`webSocketFirstPayloadName()`, `webSocketFirstPayloadNameOrNull()`,
`webSocketPayloadNameOrNull()`,
`webSocketPayloadManifestEntryByName()`,
`webSocketPayloadManifestEntryByNameOrNull()`,
`webSocketPayloadsByDirection()`, `webSocketServerPayloads()`,
`webSocketClientEventPayloads()`, `webSocketPayloadsForChannel()`,
`webSocketPayloadsForEvent()`, `webSocketPayloadsForChannelEvent()`,
direction/server/client/channel/event/channel-event payload count/presence/first
selectors, nullable first selector wrappers, `webSocketPayloadSchema()`, and
matching name helpers instead of scanning `WebSocketPayloadManifest` directly or
rebuilding manifest keys locally.
For inbound frames parsed from the socket, the generated manifest also exports
`WebSocketProtocol`, `isWebSocketChannelName()`,
`isWebSocketChannelProtocolEventName()`, `isWebSocketProtocolEventName()`,
`webSocketChannelProtocolEventNameOrNull()`,
`webSocketSystemEventNameOrNull()`, `webSocketProtocolEventNameOrNull()`,
`isWebSocketProtocolFrame()`, `isWebSocketServerEventName()`,
`isWebSocketServerFrame()`, and `parseWebSocketServerFrame()`. These guards check
unknown JSON against registered channels, Foundry protocol events
(`system/error`, `system/ack`, subscription confirmations, and presence
join/leave frames), and declared server-event allowlists before frontend code
dispatches the payload, so clients do not need a second hand-written
channel/event registry.
Foundry reserves channel protocol event names (`subscribed`, `unsubscribed`,
`presence:join`, and `presence:leave`) for framework frames. Prefer
domain-specific event names for backend-published app frames so frontend
dispatch stays unambiguous. Directly constructed `WebSocketChannelDescriptor`
values must follow the same registry shape: channel ids, guard ids, permission
ids, and declared event ids must be non-empty and trimmed; permissions must be
unique per channel; guard or permission metadata requires `requires_auth: true`;
`client_events` requires `allow_client_events: true`; and declared client/server
events must be unique domain event ids rather than reserved protocol events.

When jobs are registered in the app, `types:export` writes `JobManifest.ts`
from the runtime job registry. It includes each job id, resolved queue,
resolved queue priority, `JobIds`, `isJobName()`, `jobNameOrNull()`,
`jobManifestEntry()`, `jobManifestEntryOrNull()`, `isJobQueueName()`,
`jobQueueNameOrNull()`, `isJobPayloadName()`, `jobPayloadNameOrNull()`, `jobNames()`, `jobEntries()`,
`jobCount()`, `jobHasEntries()`, `jobFirstEntry()`,
`jobFirstEntryOrNull()`, `jobFirstName()`, `jobFirstNameOrNull()`,
`jobQueue()`, `jobQueueNames()`, `jobQueueNameCount()`,
`jobHasQueueNames()`, `jobFirstQueueName()`,
`jobFirstQueueNameOrNull()`, `jobNamesInQueue()`,
`jobUsesQueue()`, `jobsInQueue()`, `jobCountInQueue()`,
`jobHasJobsInQueue()`, `firstJobInQueue()`, `firstJobInQueueOrNull()`,
`firstJobNameInQueue()`, `firstJobNameInQueueOrNull()`,
`jobPriority()`, `jobsWithQueuePriority()`, `jobCountWithQueuePriority()`,
`jobHasJobsWithQueuePriority()`, `firstJobWithQueuePriority()`,
`firstJobWithQueuePriorityOrNull()`, `firstJobNameWithQueuePriority()`,
`firstJobNameWithQueuePriorityOrNull()`, `jobPriorityQueueNames()`,
`jobPriorityQueueNameCount()`, `jobHasPriorityQueueNames()`,
`jobFirstPriorityQueueName()`, `jobFirstPriorityQueueNameOrNull()`,
`jobPayloadName()`, `jobPayloadNames()`, `jobPayloadNameCount()`,
`jobHasPayloadNames()`, `jobFirstPayloadName()`, `jobFirstPayloadNameOrNull()`,
`jobHasPayload()`, `jobNamesWithPayload()`, `jobUsesPayload()`,
`jobsWithPayload()`, `jobCountWithPayload()`, `jobHasJobsWithPayload()`,
`firstJobWithPayload()`, `firstJobWithPayloadOrNull()`,
`firstJobNameWithPayload()`, `firstJobNameWithPayloadOrNull()`,
`jobsWithoutPayload()`, `jobNamesWithoutPayload()`, `jobCountWithoutPayload()`,
`jobHasJobsWithoutPayload()`, `firstJobWithoutPayload()`,
`firstJobWithoutPayloadOrNull()`, `firstJobNameWithoutPayload()`, and
`firstJobNameWithoutPayloadOrNull()` so admin tooling can inspect
backend-owned job names, queues, configured priority queues, priorities, and
payload contracts without
duplicating strings, config joins, or local first selector `... ?? null`
wrappers. Apps can register payload DTOs with
`TsJobPayload::new(JobId::new("..."), "DtoName")` to add `JobPayloadMap` and
`JobPayload<Name>` entries for dashboard/tooling contracts. `make:job`
scaffolds an exportable payload type, but the `TsJobPayload` entry should be
added only after the job is registered so the manifest cannot point at stale job
ids. The same manifest includes
frontend-safe worker policy metadata via
`JobRuntimeManifest`, `JobDefaultQueue`, `JobMaxRetries`,
`jobDefaultQueueName()`, `jobMaxRetries()`, `jobMaxConcurrentJobs()`,
`jobTimeoutSeconds()`, `jobHistoryRetentionDays()`,
`jobPriorityQueueNames()`, `jobPriorityQueueNameCount()`,
`jobHasPriorityQueueNames()`, `jobFirstPriorityQueueName()`,
`jobQueuePriority()`, and `jobHistoryTracked()`. It omits polling, lease, shutdown, requeue, and
history-prune internals that only workers need. Runtime manifest export requires
`jobs.queue` and `jobs.queue_priorities` keys to be non-empty trimmed queue ids,
with `jobs.max_retries` and `jobs.timeout_seconds` greater than `0`;
exported timeout and concurrency numbers must fit within JavaScript's safe
integer range. `max_concurrent_jobs = 0` and `history_retention_days = 0` keep
their documented unlimited/keep-forever meanings. The manifest is read-only
metadata; frontend clients should not enqueue jobs directly unless an
application route intentionally exposes that workflow. Generated job manifest,
runtime, and id metadata are frozen at runtime, so local dashboard annotations or
sorting state cannot mutate generated constants. Job selector helpers clone
manifest entries before returning them, so dashboards can annotate selector
results locally without touching the backend-owned manifest.

When CLI commands are registered in the app, `types:export` writes
`CommandManifest.ts` from the command registry snapshot used by the running CLI
kernel. It includes each command id, command name, short and long descriptions,
argument names, argument metadata (`kind`, `required`, `repeatable`, and
`defaultValues`, plus `help`, `longHelp`, `valueNames`, `valueHint`, and
`possibleValues`), split positional argument names, non-positional option names,
value-taking option names, flag names, subcommand names, counts, `CommandIds`,
`isCommandName()`, `commandNameOrNull()`,
`isCommandArgumentName()`, `commandArgumentNameOrNull()`,
`isCommandPositionalArgumentName()`, `commandPositionalArgumentNameOrNull()`,
`isCommandOptionName()`, `commandOptionNameOrNull()`,
`isCommandValueOptionName()`, `commandValueOptionNameOrNull()`,
`isCommandFlagName()`, `commandFlagNameOrNull()`,
`isCommandSubcommandName()`, `commandSubcommandNameOrNull()`,
`commandManifestEntry()`, `commandManifestEntryOrNull()`, `commandNames()`, `commandEntries()`,
`commandCount()`, `commandHasEntries()`, `commandFirstEntry()`,
`commandFirstEntryOrNull()`, `commandFirstName()`, `commandFirstNameOrNull()`,
`commandArguments()`, `commandArgumentMetadata()`,
`commandArgumentMetadataForArgument()`,
`commandArgumentMetadataForArgumentOrNull()`,
`commandArgumentHelp()`, `commandArgumentLongHelp()`,
`commandArgumentValueNames()`, `commandArgumentValueHint()`,
`commandArgumentDefaultValues()`, `commandArgumentPossibleValues()`,
`commandVisibleArgumentPossibleValues()`,
`commandArgumentPossibleValueNames()`,
`commandVisibleArgumentPossibleValueNames()`, `commandRequiredArguments()`,
`commandRequiredArgumentNames()`, `commandRepeatableArguments()`,
`commandRepeatableArgumentNames()`, `commandDefaultedArguments()`,
`commandDefaultedArgumentNames()`, `commandArgumentsWithPossibleValues()`,
`commandArgumentNamesWithPossibleValues()`, `commandArgumentNames()`,
`commandArgumentNameCount()`, `commandTotalArgumentCount()`,
`commandHasArgumentNames()`, `commandFirstArgumentName()`,
`commandFirstArgumentNameOrNull()`,
`commandPositionalArguments()`, `commandPositionalArgumentNames()`,
`commandPositionalArgumentNameCount()`, `commandTotalPositionalArgumentCount()`,
`commandHasPositionalArgumentNames()`, `commandFirstPositionalArgumentName()`,
`commandFirstPositionalArgumentNameOrNull()`, `commandOptions()`,
`commandOptionNames()`, `commandOptionNameCount()`, `commandTotalOptionCount()`,
`commandHasOptionNames()`, `commandFirstOptionName()`,
`commandFirstOptionNameOrNull()`, `commandValueOptions()`,
`commandOptionSwitches()`, `commandOptionSwitch()`,
`commandOptionSwitchOrNull()`, `commandOptionTokens()`,
`commandPreferredOptionToken()`, `commandPreferredOptionTokenOrNull()`,
`commandValueOptionNames()`, `commandValueOptionNameCount()`,
`commandTotalValueOptionCount()`, `commandHasValueOptionNames()`,
`commandFirstValueOptionName()`, `commandFirstValueOptionNameOrNull()`,
`commandFlags()`, `commandFlagNames()`, `commandFlagNameCount()`,
`commandTotalFlagCount()`, `commandHasFlagNames()`, `commandFirstFlagName()`,
`commandFirstFlagNameOrNull()`, `commandSubcommands()`,
`commandSubcommandNames()`, `commandSubcommandNameCount()`,
`commandTotalSubcommandCount()`, `commandHasSubcommandNames()`,
`commandFirstSubcommandName()`, `commandFirstSubcommandNameOrNull()`,
`commandHasArgument()`, `commandHasSubcommand()`, `commandNamesWithArgument()`,
`commandsWithArgument()`, `commandCountWithArgument()`,
`commandHasCommandsWithArgument()`, `firstCommandWithArgument()`,
`firstCommandWithArgumentOrNull()`, `firstCommandNameWithArgument()`,
`firstCommandNameWithArgumentOrNull()`, `commandNamesWithoutArgument()`,
`commandsWithoutArgument()`, `commandCountWithoutArgument()`,
`commandHasCommandsWithoutArgument()`, `firstCommandWithoutArgument()`,
`firstCommandWithoutArgumentOrNull()`, `firstCommandNameWithoutArgument()`,
`firstCommandNameWithoutArgumentOrNull()`,
`commandNamesWithPositionalArgument()`, `commandsWithPositionalArgument()`,
`commandCountWithPositionalArgument()`,
`commandHasCommandsWithPositionalArgument()`,
`firstCommandWithPositionalArgument()`,
`firstCommandWithPositionalArgumentOrNull()`,
`firstCommandNameWithPositionalArgument()`,
`firstCommandNameWithPositionalArgumentOrNull()`,
`commandNamesWithoutPositionalArgument()`, `commandsWithoutPositionalArgument()`,
`commandCountWithoutPositionalArgument()`,
`commandHasCommandsWithoutPositionalArgument()`,
`firstCommandWithoutPositionalArgument()`,
`firstCommandWithoutPositionalArgumentOrNull()`,
`firstCommandNameWithoutPositionalArgument()`,
`firstCommandNameWithoutPositionalArgumentOrNull()`, `commandNamesWithOption()`,
`commandsWithOption()`, `commandCountWithOption()`,
`commandHasCommandsWithOption()`, `firstCommandWithOption()`,
`firstCommandWithOptionOrNull()`, `firstCommandNameWithOption()`,
`firstCommandNameWithOptionOrNull()`, `commandNamesWithoutOption()`,
`commandsWithoutOption()`, `commandCountWithoutOption()`,
`commandHasCommandsWithoutOption()`, `firstCommandWithoutOption()`,
`firstCommandWithoutOptionOrNull()`, `firstCommandNameWithoutOption()`,
`firstCommandNameWithoutOptionOrNull()`, `commandNamesWithValueOption()`,
`commandsWithValueOption()`, `commandCountWithValueOption()`,
`commandHasCommandsWithValueOption()`, `firstCommandWithValueOption()`,
`firstCommandWithValueOptionOrNull()`, `firstCommandNameWithValueOption()`,
`firstCommandNameWithValueOptionOrNull()`, `commandNamesWithoutValueOption()`,
`commandsWithoutValueOption()`, `commandCountWithoutValueOption()`,
`commandHasCommandsWithoutValueOption()`, `firstCommandWithoutValueOption()`,
`firstCommandWithoutValueOptionOrNull()`,
`firstCommandNameWithoutValueOption()`,
`firstCommandNameWithoutValueOptionOrNull()`, `commandNamesWithFlag()`,
`commandsWithFlag()`, `commandCountWithFlag()`,
`commandHasCommandsWithFlag()`, `firstCommandWithFlag()`,
`firstCommandWithFlagOrNull()`, `firstCommandNameWithFlag()`,
`firstCommandNameWithFlagOrNull()`, `commandNamesWithoutFlag()`,
`commandsWithoutFlag()`, `commandCountWithoutFlag()`,
`commandHasCommandsWithoutFlag()`, `firstCommandWithoutFlag()`,
`firstCommandWithoutFlagOrNull()`, `firstCommandNameWithoutFlag()`,
`firstCommandNameWithoutFlagOrNull()`, `commandNamesWithSubcommand()`,
`commandsWithSubcommand()`, `commandCountWithSubcommand()`,
`commandHasCommandsWithSubcommand()`, `firstCommandWithSubcommand()`,
`firstCommandWithSubcommandOrNull()`, `firstCommandNameWithSubcommand()`,
`firstCommandNameWithSubcommandOrNull()`, `commandNamesWithoutSubcommand()`,
`commandsWithoutSubcommand()`, `commandCountWithoutSubcommand()`,
`commandHasCommandsWithoutSubcommand()`, `firstCommandWithoutSubcommand()`,
`firstCommandWithoutSubcommandOrNull()`, `firstCommandNameWithoutSubcommand()`,
`firstCommandNameWithoutSubcommandOrNull()`, `commandsWithArguments()`,
`commandCountWithArguments()`, `commandHasCommandsWithArguments()`,
`firstCommandWithArguments()`, `firstCommandWithArgumentsOrNull()`,
`firstCommandNameWithArguments()`, `firstCommandNameWithArgumentsOrNull()`,
`commandNamesWithoutArguments()`, `commandsWithoutArguments()`,
`commandCountWithoutArguments()`, `commandHasCommandsWithoutArguments()`,
`firstCommandWithoutArguments()`, `firstCommandWithoutArgumentsOrNull()`,
`firstCommandNameWithoutArguments()`, `firstCommandNameWithoutArgumentsOrNull()`,
`commandsWithPositionalArguments()`, `commandCountWithPositionalArguments()`,
`commandHasCommandsWithPositionalArguments()`,
`firstCommandWithPositionalArguments()`,
`firstCommandWithPositionalArgumentsOrNull()`,
`firstCommandNameWithPositionalArguments()`,
`firstCommandNameWithPositionalArgumentsOrNull()`,
`commandNamesWithoutPositionalArguments()`,
`commandsWithoutPositionalArguments()`,
`commandCountWithoutPositionalArguments()`,
`commandHasCommandsWithoutPositionalArguments()`,
`firstCommandWithoutPositionalArguments()`,
`firstCommandWithoutPositionalArgumentsOrNull()`,
`firstCommandNameWithoutPositionalArguments()`,
`firstCommandNameWithoutPositionalArgumentsOrNull()`, `commandsWithOptions()`,
`commandCountWithOptions()`, `commandHasCommandsWithOptions()`,
`firstCommandWithOptions()`, `firstCommandWithOptionsOrNull()`,
`firstCommandNameWithOptions()`, `firstCommandNameWithOptionsOrNull()`,
`commandNamesWithoutOptions()`, `commandsWithoutOptions()`,
`commandCountWithoutOptions()`, `commandHasCommandsWithoutOptions()`,
`firstCommandWithoutOptions()`, `firstCommandWithoutOptionsOrNull()`,
`firstCommandNameWithoutOptions()`, `firstCommandNameWithoutOptionsOrNull()`,
`commandsWithValueOptions()`, `commandCountWithValueOptions()`,
`commandHasCommandsWithValueOptions()`, `firstCommandWithValueOptions()`,
`firstCommandWithValueOptionsOrNull()`, `firstCommandNameWithValueOptions()`,
`firstCommandNameWithValueOptionsOrNull()`,
`commandNamesWithoutValueOptions()`, `commandsWithoutValueOptions()`,
`commandCountWithoutValueOptions()`,
`commandHasCommandsWithoutValueOptions()`,
`firstCommandWithoutValueOptions()`,
`firstCommandWithoutValueOptionsOrNull()`,
`firstCommandNameWithoutValueOptions()`,
`firstCommandNameWithoutValueOptionsOrNull()`, `commandsWithFlags()`,
`commandCountWithFlags()`, `commandHasCommandsWithFlags()`,
`firstCommandWithFlags()`, `firstCommandWithFlagsOrNull()`,
`firstCommandNameWithFlags()`, `firstCommandNameWithFlagsOrNull()`,
`commandNamesWithoutFlags()`, `commandsWithoutFlags()`,
`commandCountWithoutFlags()`, `commandHasCommandsWithoutFlags()`,
`firstCommandWithoutFlags()`, `firstCommandWithoutFlagsOrNull()`,
`firstCommandNameWithoutFlags()`, `firstCommandNameWithoutFlagsOrNull()`,
`commandsWithSubcommands()`, `commandCountWithSubcommands()`,
`commandHasCommandsWithSubcommands()`, `firstCommandWithSubcommands()`,
`firstCommandWithSubcommandsOrNull()`, `firstCommandNameWithSubcommands()`,
`firstCommandNameWithSubcommandsOrNull()`, `commandNamesWithoutSubcommands()`,
`commandsWithoutSubcommands()`, `commandCountWithoutSubcommands()`,
`commandHasCommandsWithoutSubcommands()`, `firstCommandWithoutSubcommands()`,
`firstCommandWithoutSubcommandsOrNull()`, `firstCommandNameWithoutSubcommands()`,
and `firstCommandNameWithoutSubcommandsOrNull()` so admin/dev tooling
can inspect backend-owned command names and shape without duplicating strings or
local first selector `... ?? null` wrappers.
`CommandManifest.ts` rejects command count metadata that does not match its
backend-owned arrays, and rejects count values above JavaScript's safe integer
range before emitting `argCount`, `optionCount`, `flagCount`, and related
numeric helpers.
Generated command manifest and id metadata are frozen at runtime, so developer
tools cannot mutate backend-owned command metadata through direct constants.
Command selector helpers clone command entries plus argument
metadata/default-value/possible-value arrays, positional argument, option,
option-switch, value-option, flag, and subcommand arrays before returning them,
so dev tools can decorate selector results locally.

When plugins are registered in the app, `types:export` writes
`PluginManifest.ts` from the plugin registry. It includes each plugin id,
version, Foundry version constraint, description, dependencies, contribution
counts, asset target paths/kinds, scaffold variables, and scaffold output paths,
plus `PluginIds`, `isPluginName()`, `pluginNameOrNull()`,
`isPluginAssetKind()`, `pluginAssetKindOrNull()`,
`isPluginContributionName()`, `pluginContributionNameOrNull()`, `pluginManifestEntry()`,
`pluginManifestEntryOrNull()`, `pluginNames()`,
`pluginEntries()`, `pluginCount()`, `pluginHasEntries()`,
`pluginFirstEntry()`, `pluginFirstEntryOrNull()`, `pluginFirstName()`,
`pluginFirstNameOrNull()`, `pluginDependencyCount()`,
`pluginTotalDependencyCount()`, `pluginHasDependencies()`,
`pluginFirstDependency()`, `pluginFirstDependencyOrNull()`,
`pluginFirstDependencyId()`, `pluginFirstDependencyIdOrNull()`, `pluginsWithDependencies()`,
`pluginNamesWithDependencies()`, `pluginCountWithDependencies()`,
`pluginHasPluginsWithDependencies()`, `pluginFirstPluginWithDependencies()`,
`pluginFirstPluginWithDependenciesOrNull()`,
`pluginFirstPluginNameWithDependencies()`,
`pluginFirstPluginNameWithDependenciesOrNull()`, `pluginsWithoutDependencies()`,
`pluginNamesWithoutDependencies()`, `pluginCountWithoutDependencies()`,
`pluginHasPluginsWithoutDependencies()`, `pluginFirstPluginWithoutDependencies()`,
`pluginFirstPluginWithoutDependenciesOrNull()`,
`pluginFirstPluginNameWithoutDependencies()`,
`pluginFirstPluginNameWithoutDependenciesOrNull()`, `pluginAssetKinds()`,
`pluginAssetKindCount()`, `pluginHasAssetKinds()`, `pluginFirstAssetKind()`,
`pluginFirstAssetKindOrNull()`,
`pluginAssetCount()`, `pluginTotalAssetCount()`, `pluginHasAssets()`,
`pluginFirstAsset()`, `pluginFirstAssetOrNull()`, `pluginFirstAssetId()`,
`pluginFirstAssetIdOrNull()`, `pluginsWithAssets()`,
`pluginNamesWithAssets()`, `pluginCountWithAssets()`,
`pluginHasPluginsWithAssets()`, `pluginFirstPluginWithAssets()`,
`pluginFirstPluginWithAssetsOrNull()`, `pluginFirstPluginNameWithAssets()`,
`pluginFirstPluginNameWithAssetsOrNull()`, `pluginsWithoutAssets()`,
`pluginNamesWithoutAssets()`, `pluginCountWithoutAssets()`,
`pluginHasPluginsWithoutAssets()`, `pluginFirstPluginWithoutAssets()`,
`pluginFirstPluginWithoutAssetsOrNull()`, `pluginFirstPluginNameWithoutAssets()`,
`pluginFirstPluginNameWithoutAssetsOrNull()`, `pluginScaffoldCount()`,
`pluginTotalScaffoldCount()`, `pluginHasScaffolds()`,
`pluginFirstScaffold()`, `pluginFirstScaffoldOrNull()`,
`pluginFirstScaffoldId()`, `pluginFirstScaffoldIdOrNull()`, `pluginsWithScaffolds()`,
`pluginNamesWithScaffolds()`, `pluginCountWithScaffolds()`,
`pluginHasPluginsWithScaffolds()`, `pluginFirstPluginWithScaffolds()`,
`pluginFirstPluginWithScaffoldsOrNull()`,
`pluginFirstPluginNameWithScaffolds()`,
`pluginFirstPluginNameWithScaffoldsOrNull()`, `pluginsWithoutScaffolds()`,
`pluginNamesWithoutScaffolds()`, `pluginCountWithoutScaffolds()`,
`pluginHasPluginsWithoutScaffolds()`, `pluginFirstPluginWithoutScaffolds()`,
`pluginFirstPluginWithoutScaffoldsOrNull()`,
`pluginFirstPluginNameWithoutScaffolds()`,
`pluginFirstPluginNameWithoutScaffoldsOrNull()`, `pluginContributionCount()`,
`pluginTotalContributionCount()`, `pluginHasContribution()`, `pluginContributionNames()`,
`pluginContributionNameCount()`, `pluginHasContributionNames()`,
`pluginFirstContributionName()`, `pluginFirstContributionNameOrNull()`,
`pluginsWithContribution()`,
`pluginNamesWithContribution()`, `pluginCountWithContribution()`,
`pluginHasPluginsWithContribution()`, `pluginFirstPluginWithContribution()`,
`pluginFirstPluginWithContributionOrNull()`,
`pluginFirstPluginNameWithContribution()`,
`pluginFirstPluginNameWithContributionOrNull()`, `pluginsWithoutContribution()`,
`pluginNamesWithoutContribution()`, `pluginCountWithoutContribution()`,
`pluginHasPluginsWithoutContribution()`, `pluginFirstPluginWithoutContribution()`,
`pluginFirstPluginWithoutContributionOrNull()`,
`pluginFirstPluginNameWithoutContribution()`, and
`pluginFirstPluginNameWithoutContributionOrNull()`. Asset and scaffold file
contents are not exported; the manifest is metadata for admin screens, plugin dashboards, and
developer tooling without local first selector `... ?? null` wrappers.
`PluginManifest.ts` rejects contribution counts and aggregate contribution
totals above JavaScript's safe integer range, and rejects `assets` /
`scaffolds` contribution counts that do not match the exported asset and
scaffold arrays. Generated plugin manifest and id metadata are frozen at
runtime, so plugin dashboards cannot mutate backend-owned plugin metadata
through direct constants. Plugin selector helpers clone entries, dependencies,
assets, scaffolds, scaffold variables/files, and contribution counts before
returning them, so dashboards can apply local annotations to selector results.

When schedules are registered in the app, `types:export` writes
`ScheduleManifest.ts` from the scheduler registry. It includes each schedule id,
cron expression or interval milliseconds, overlap-lock flag, environment
filters, lifecycle hook presence, `ScheduleIds`, `isScheduleName()`,
`scheduleNameOrNull()`, `isScheduleEnvironmentName()`,
`scheduleEnvironmentNameOrNull()`, `isScheduleHookName()`, `scheduleHookNameOrNull()`,
`scheduleManifestEntry()`, `scheduleManifestEntryOrNull()`, `scheduleNames()`, `scheduleEntries()`,
`scheduleCount()`, `scheduleHasEntries()`, `scheduleFirstEntry()`,
`scheduleFirstEntryOrNull()`, `scheduleFirstName()`, `scheduleFirstNameOrNull()`,
`cronScheduleNames()`, `cronSchedules()`,
`cronScheduleCount()`, `cronScheduleHasEntries()`,
`cronScheduleFirstEntry()`, `cronScheduleFirstEntryOrNull()`,
`cronScheduleFirstName()`, `cronScheduleFirstNameOrNull()`,
`intervalScheduleNames()`, `intervalSchedules()`, `intervalScheduleCount()`,
`intervalScheduleHasEntries()`, `intervalScheduleFirstEntry()`,
`intervalScheduleFirstEntryOrNull()`, `intervalScheduleFirstName()`,
`intervalScheduleFirstNameOrNull()`, `scheduleEnvironmentNames()`,
`scheduleEnvironmentNameCount()`, `scheduleHasEnvironmentNames()`,
`scheduleTotalEnvironmentFilterCount()`, `scheduleFirstEnvironmentName()`,
`scheduleFirstEnvironmentNameOrNull()`,
`scheduleNamesForEnvironment()`, `schedulesForEnvironment()`,
`scheduleCountForEnvironment()`, `scheduleHasSchedulesForEnvironment()`,
`firstScheduleForEnvironment()`, `firstScheduleForEnvironmentOrNull()`,
`firstScheduleNameForEnvironment()`, `firstScheduleNameForEnvironmentOrNull()`,
`scheduleNamesWithoutEnvironment()`, `schedulesWithoutEnvironment()`,
`scheduleCountWithoutEnvironment()`, `scheduleHasSchedulesWithoutEnvironment()`,
`firstScheduleWithoutEnvironment()`, `firstScheduleWithoutEnvironmentOrNull()`,
`firstScheduleNameWithoutEnvironment()`, `firstScheduleNameWithoutEnvironmentOrNull()`,
`scheduleNamesWithoutEnvironmentFilters()`,
`schedulesWithoutEnvironmentFilters()`,
`scheduleCountWithoutEnvironmentFilters()`,
`scheduleHasSchedulesWithoutEnvironmentFilters()`,
`firstScheduleWithoutEnvironmentFilters()`,
`firstScheduleWithoutEnvironmentFiltersOrNull()`,
`firstScheduleNameWithoutEnvironmentFilters()`,
`firstScheduleNameWithoutEnvironmentFiltersOrNull()`,
`scheduleNamesWithoutOverlapping()`, `schedulesWithoutOverlapping()`,
`scheduleCountWithoutOverlapping()`, `scheduleHasSchedulesWithoutOverlapping()`,
`firstScheduleWithoutOverlapping()`, `firstScheduleWithoutOverlappingOrNull()`,
`firstScheduleNameWithoutOverlapping()`,
`firstScheduleNameWithoutOverlappingOrNull()`,
`scheduleHookNames()`, `scheduleHookNameCount()`,
`scheduleTotalEnabledHookCount()`, `scheduleHasHookNames()`,
`scheduleFirstHookName()`, `scheduleFirstHookNameOrNull()`, `scheduleNamesWithHook()`,
`schedulesWithHook()`, `scheduleCountWithHook()`,
`scheduleHasSchedulesWithHook()`, `firstScheduleWithHook()`,
`firstScheduleWithHookOrNull()`, `firstScheduleNameWithHook()`,
`firstScheduleNameWithHookOrNull()`, `scheduleNamesWithoutHook()`,
`schedulesWithoutHook()`, `scheduleCountWithoutHook()`,
`scheduleHasSchedulesWithoutHook()`, `firstScheduleWithoutHook()`,
`firstScheduleWithoutHookOrNull()`, `firstScheduleNameWithoutHook()`,
`firstScheduleNameWithoutHookOrNull()`, `scheduleNamesWithoutHooks()`,
`schedulesWithoutHooks()`, `scheduleCountWithoutHooks()`,
`scheduleHasSchedulesWithoutHooks()`, `firstScheduleWithoutHooks()`,
`firstScheduleWithoutHooksOrNull()`, `firstScheduleNameWithoutHooks()`,
`firstScheduleNameWithoutHooksOrNull()`,
`scheduleCronExpression()`, and `scheduleIntervalMilliseconds()` so admin
tooling can inspect backend-owned scheduler contracts without duplicating
strings, filter logic, or local first selector `... ?? null` wrappers. The same
manifest includes
`SchedulerRuntimeManifest`, `SchedulerTickIntervalMs`,
`SchedulerLeaderLeaseTtlMs`, `SchedulerShutdownTimeoutMs`,
`schedulerTickIntervalMs()`, `schedulerLeaderLeaseTtlMs()`, and
`schedulerShutdownTimeoutMs()` from
`SchedulerConfig`, so dashboards can display scheduler cadence and leadership
policy without mirroring config. Runtime export requires
`scheduler.tick_interval_ms` and `scheduler.leader_lease_ttl_ms` to be greater
than `0`, while `scheduler.shutdown_timeout_ms = 0` keeps its documented
abort-immediately meaning. Direct schedule descriptor exports must use parseable
cron expressions and positive JavaScript-safe interval milliseconds. The
manifest is read-only metadata; frontend
clients should trigger scheduler actions through intentional application routes.
Generated schedule manifest, runtime, and id metadata are frozen at runtime, so
scheduler dashboards cannot mutate backend-owned scheduler metadata through
direct constants. Schedule selector helpers clone entries, environment arrays,
and hook metadata before returning them, so dashboards can apply local UI-only
annotations to selector results.

When custom validation rules are registered in the app, `types:export` writes
`ValidationRuleManifest.ts` from the validation rule registry. It includes each
custom rule id, `serverOnly: true`, `ValidationRuleIds`,
`ValidationRuleId`, `isValidationRuleName()`, `isValidationRuleId()`,
`validationRuleNameOrNull()`, `validationRuleIdOrNull()`,
`validationRuleManifestEntry()`, `validationRuleManifestEntryOrNull()`,
`validationRuleManifestEntryById()`, `validationRuleManifestEntryByIdOrNull()`,
`validationRuleId()`, `validationRuleIdForNameOrNull()`,
`validationRuleIsRegistered()`, `validationRuleIdIsServerOnly()`,
`validationRuleIsServerOnlyOrNull()`,
`validationRuleIds()`, `validationRuleIdCount()`,
`validationRuleHasIds()`, `validationRuleFirstId()`,
`validationRuleFirstIdOrNull()`,
`validationRuleNames()`, `validationRuleNameCount()`,
`validationRuleHasNames()`, `validationRuleEntries()`,
`validationRuleCount()`, `validationRuleHasEntries()`,
`validationRuleFirstEntry()`, `validationRuleFirstEntryOrNull()`,
`validationRuleFirstName()`, `validationRuleFirstNameOrNull()`,
`serverOnlyValidationRuleIds()`, `serverOnlyValidationRuleIdCount()`,
`serverOnlyValidationRuleHasIds()`, `serverOnlyValidationRuleFirstId()`,
`serverOnlyValidationRuleFirstIdOrNull()`,
`serverOnlyValidationRuleNames()`, `serverOnlyValidationRuleNameCount()`,
`serverOnlyValidationRuleHasNames()`, `serverOnlyValidationRuleFirstName()`,
`serverOnlyValidationRuleFirstNameOrNull()`,
`serverOnlyValidationRules()`, `serverOnlyValidationRuleCount()`,
`serverOnlyValidationRuleHasEntries()`, and
`serverOnlyValidationRuleFirstEntry()`,
`serverOnlyValidationRuleFirstEntryOrNull()` so frontend form builders and docs
tooling can discover backend-owned custom checks without copying rule strings,
rebuilding rule lists, or wrapping first selectors with `... ?? null`. The browser runtime still only executes supported client-checkable
rules; custom rules remain backend-enforced. Generated validation-rule manifest
and id metadata are frozen at runtime, so direct mutation cannot change
backend-owned rule metadata. Validation-rule selector helpers clone entries
before returning them, so form builders and docs tooling can add UI-only copy or
grouping state to selector results locally.
App-backed OpenAPI specs publish the same registered custom-rule map as
`x-foundry-validation-rules`, so OpenAPI-driven docs and client generators can
join schema-level custom `x-foundry-validation` rules to the backend registry
without maintaining a second rule list.
Use `validationRuleIdOrNull(value)` to parse a runtime rule id, and
`validationRuleIdForNameOrNull(name)` or `validationRuleIsServerOnlyOrNull(name)`
when a runtime rule name should drive nullable metadata reads.

When events have registered listeners, `types:export` writes `EventManifest.ts`
from the event bus registry. It includes each event id, listener count,
`EventIds`, `isEventName()`, `eventNameOrNull()`, `isEventPayloadName()`, `eventPayloadNameOrNull()`,
`eventManifestEntry()`, `eventManifestEntryOrNull()`, `eventNames()`, `eventEntries()`,
`eventCount()`, `eventHasEntries()`, `eventFirstEntry()`, `eventFirstName()`,
`eventFirstEntryOrNull()`, `eventFirstNameOrNull()`, `eventListenerCount()`,
`eventTotalListenerCount()`, `eventPayloadName()`, `eventPayloadNames()`,
`eventPayloadNameCount()`, `eventHasPayloadNames()`,
`eventFirstPayloadName()`, `eventFirstPayloadNameOrNull()`, `eventHasPayload()`,
`eventNamesWithPayload()`, `eventUsesPayload()`, `eventsWithPayload()`,
`eventCountWithPayload()`, `eventHasEventsWithPayload()`,
`firstEventWithPayload()`, `firstEventWithPayloadOrNull()`,
`firstEventNameWithPayload()`, `firstEventNameWithPayloadOrNull()`,
`eventsWithoutPayload()`,
`eventNamesWithoutPayload()`, `eventCountWithoutPayload()`,
`eventHasEventsWithoutPayload()`, `firstEventWithoutPayload()`,
`firstEventWithoutPayloadOrNull()`, `firstEventNameWithoutPayload()`,
`firstEventNameWithoutPayloadOrNull()`, `eventNamesWithListeners()`,
`eventCountWithListeners()`, `eventHasEventsWithListeners()`,
`firstEventWithListeners()`, `firstEventWithListenersOrNull()`,
`firstEventNameWithListeners()`, `firstEventNameWithListenersOrNull()`,
`eventNamesWithoutListeners()`, `eventCountWithoutListeners()`,
`eventHasEventsWithoutListeners()`, `firstEventWithoutListeners()`,
`firstEventWithoutListenersOrNull()`, `firstEventNameWithoutListeners()`,
`firstEventNameWithoutListenersOrNull()`, `eventsWithListeners()`, and
`eventsWithoutListeners()` so dashboards and plugins can inspect backend-owned
event names without copying filter logic or local `... ?? null` wrappers.
Apps can register payload DTOs with
`TsEventPayload::new(EventId::new("..."), "DtoName")` to add `EventPayloadMap`
and `EventPayload<Name>` entries. Payload-only entries are allowed with
`listenerCount: 0`, which keeps event payload contracts exportable before an
app has listeners for that event. Events created with `make:event` include that
payload registration beside the event type. Generated event manifest and id
metadata are frozen at runtime, so dashboards cannot mutate backend-owned event
metadata through direct constants. Event selector helpers clone entries before
returning them, so dashboards can attach local listener or display state to
selector results.
`EventManifest.ts` rejects per-event listener counts and total listener counts
above JavaScript's safe integer range before emitting `listenerCount` and
`eventTotalListenerCount()` helpers.

When broadcast notification payloads are registered, `types:export` writes
`NotificationManifest.ts` with `NotificationPayloadMap`,
`TypedNotificationBroadcastPayload`, the built-in channel ids, the canonical
broadcast channel/event constants, plus `notificationEntries()`,
`notificationTypes()`, `notificationCount()`, `notificationHasEntries()`,
`notificationFirstEntry()`, `notificationFirstType()`,
`notificationFirstEntryOrNull()`, `notificationFirstTypeOrNull()`,
`notificationManifestEntry()`, `notificationManifestEntryOrNull()`,
`notificationChannelNames()`, channel count/presence/first-name selectors,
delivery-channel selectors/predicates, broadcast channel/event
selectors/predicates, `notificationPayloadNames()`,
payload-name count/presence/first-name selectors, `isNotificationPayloadName()`,
`notificationTypeOrNull()`, `notificationChannelNameOrNull()`,
`notificationPayloadNameOrNull()`,
`notificationsWithPayload()`, payload-filtered entry count/presence/first-entry
selectors, `notificationTypesWithPayload()`, payload-filtered type
count/presence/first-type selectors, payload-exclusion entry/type
selectors with their count/presence/first-item helpers, `notificationUsesPayload()`,
`notificationBroadcastPayloadType()`,
`isRegisteredNotificationBroadcastPayload()`,
`notificationBroadcastPayloadManifestEntry()`, and
`notificationBroadcastPayloadName()` selectors.
Nullable first-selector companions such as `notificationFirstChannelNameOrNull()`,
`notificationFirstPayloadNameOrNull()`, `notificationFirstEntryWithPayloadOrNull()`,
and `notificationFirstTypeWithoutPayloadOrNull()` return explicit `null` for
empty backend-owned notification metadata lists.
Notifications created with `make:notification` include the `TsNotification`
registration beside the notification type and use the notification struct as
the typed broadcast `data` payload.
Generated notification manifests, channel id maps, and notification type lists
are frozen at runtime, so direct mutation cannot change backend-owned
notification metadata. Notification selector helpers clone manifest entries
before returning them, so dashboards can add local delivery status or grouping
state to selector results.

When the app registers datatables, `types:export` also writes
`DatatableManifest.ts`. It includes each datatable id, column metadata, computed
mappings, relation-filter aliases, and backend default sort order, plus
`DatatableRuntimeManifest`, `DatatableMaxPerPage`, `DatatableMaxExportRows`,
`datatableMaxPerPage()`, `datatableMaxExportRows()`, `DatatableIds`, and
`isDatatableName()` / `datatableNameOrNull()` for frontend table builders that should
not duplicate server-owned table declarations or configured pagination/export
caps. Dotted datatable ids are grouped and camelCased for property access, so
`reports.daily-sales` becomes `DatatableIds.reports.dailySales`. It also exports
`datatableMaxPerPage()`, `datatableMaxExportRows()`,
`datatablePerPageCap()`, `datatableExportRowsCap()`, `datatableManifestEntry()`,
`datatableManifestEntryOrNull()`, `datatableNameOrNull()`,
`datatableEntries()`, `datatableNames()`, `datatableColumns()`,
`datatableColumn()`, `datatableColumnNames()`,
`datatableSortableColumns()`, `datatableSortableColumn()`,
`datatableSortableFieldNames()`, `datatableNonSortableColumns()`,
`datatableNonSortableColumn()`, `datatableNonSortableColumnNames()`,
`datatableFilterableColumns()`, `datatableFilterableColumn()`,
`datatableFilterableColumnNames()`, `datatableNonFilterableColumns()`,
`datatableNonFilterableColumn()`, `datatableNonFilterableColumnNames()`,
`datatableExportableColumns()`, `datatableExportableColumn()`,
`datatableExportableColumnNames()`, `datatableNonExportableColumns()`,
`datatableNonExportableColumn()`, `datatableNonExportableColumnNames()`,
`datatableRelationColumns()`, `datatableRelationColumn()`,
`datatableRelationColumnNames()`, `datatableNonRelationColumns()`,
`datatableNonRelationColumn()`, `datatableNonRelationColumnNames()`,
`datatableColumnRelationNames()`, `datatableColumnsForRelation()`,
`datatableColumnNamesForRelation()`, `DatatableColumnForRelation<Name, Relation>`,
`DatatableColumnNameForRelation<Name, Relation>`,
`datatableMappings()`, `datatableMappingNames()`,
`datatableRelationFilters()`, `datatableRelationFilterForField()`,
`datatableRelationFilterCanonicalField()`,
`datatableRelationFilterRelationNames()`,
`datatableRelationFiltersForRelation()`,
`datatableRelationFilterForFieldForRelation()`,
`datatableRelationFilterCanonicalFieldForRelation()`,
`datatableRelationFilterFieldsForRelation()`,
`datatableRelationFilterAliasesForRelation()`,
`datatableRelationFilterFieldNamesForRelation()`,
`DatatableRelationFilterForRelation<Name, Relation>`,
`DatatableRelationFilterCanonicalFieldNameForRelation<Name, Relation>`,
`DatatableRelationFilterAliasNameForRelation<Name, Relation>`,
`DatatableRelationFilterFieldNameForRelation<Name, Relation>`,
`datatableRelationFilterFields()`,
`datatableRelationFilterAliases()`, `datatableRelationFilterFieldNames()`,
`datatableStaticFilterFieldNames()`,
`datatableDefaultSort()`, `datatableDefaultSortForField()`,
`datatableDefaultSortDirection()`,
`datatableDefaultSortFieldNames()`, `datatableSort()`, `datatableFilter()`,
`datatableRequest()`, `datatableQueryParams()`, and `DatatableRequestFor<Name>`
so frontend code can build datatable requests, columns, filters, mappings,
relation filter UIs, and URL query params from backend-owned sortable and static
filter fields. Guard helpers such as `isDatatableColumnNameForRelation()`,
`isDatatableRelationFilterCanonicalField()`,
`isDatatableRelationFilterAlias()`, `isDatatableRelationFilterField()`,
and their `ForRelation` variants narrow backend-owned relation-column and
relation-filter inputs
without local membership checks. Matching safe parsers such as
`datatableColumnNameOrNull()`, `datatableColumnNameForRelationOrNull()`,
`datatableRelationFilterFieldNameOrNull()`,
`datatableRelationFilterFieldNameForRelationOrNull()`,
`datatableStaticFilterFieldNameOrNull()`, `datatableMappingNameOrNull()`, and
`datatableDefaultSortFieldNameOrNull()` normalize runtime strings into the same
generated unions without local `isDatatable*(...) ? value : null` wrappers.
Matching count, presence, and first-item helpers such as
`datatableColumnCount()`, `datatableHasSortableColumns()`,
`datatableFirstFilterableColumnName()`, `datatableMappingNameCount()`,
`datatableHasRelationFilters()`,
`datatableRelationFilterRelationNameCount()`,
`datatableFirstRelationFilterFieldForRelation()`,
`datatableRelationFilterAliasCountForRelation()`,
`datatableRelationFilterFieldNameCountForRelation()`,
`datatableFirstRelationFilterAliasForRelation()`,
`datatableFirstRelationFilterFieldName()`,
`datatableFirstStaticFilterFieldName()`, and
`datatableHasDefaultSort()` summarize backend-owned table metadata, including
non-sortable, non-filterable, exportable, non-exportable, relation-backed, plain,
relation-name column groups, and relation-filter relation groups, without local
`.length`, `.length > 0`, or `[0]` wrappers.
Nullable first-selector companions such as `datatableFirstEntryOrNull()`,
`datatableFirstColumnOrNull()`, `datatableFirstColumnForRelationOrNull()`,
`datatableFirstRelationFilterForRelationOrNull()`,
`datatableFirstStaticFilterFieldNameOrNull()`, and
`datatableFirstDefaultSortOrNull()` return explicit `null` for empty
backend-owned datatable metadata lists.
Aggregate selectors such as
`datatableTotalColumnCount()`, `datatableTotalSortableColumnCount()`,
`datatableTotalNonSortableColumnCount()`,
`datatableTotalFilterableColumnCount()`,
`datatableTotalNonFilterableColumnCount()`,
`datatableTotalExportableColumnCount()`,
`datatableTotalNonExportableColumnCount()`,
`datatableTotalRelationColumnCount()`,
`datatableTotalNonRelationColumnCount()`,
`datatableTotalColumnRelationNameCount()`, `datatableTotalMappingCount()`,
`datatableTotalRelationFilterCount()`,
`datatableTotalRelationFilterRelationNameCount()`,
`datatableTotalRelationFilterAliasCount()`,
`datatableTotalRelationFilterFieldNameCount()`,
`datatableTotalStaticFilterFieldCount()`, and
`datatableTotalDefaultSortCount()` summarize all registered datatables without
local `datatableEntries().reduce(...)` wrappers. `DatatableRequestFor<Name>`
narrows filters to those static manifest fields, and `datatableRequest()` throws if a stale filter field
is passed at runtime. Dynamic filters returned by `available_filters()` can
still use the raw generated `DatatableRequest` shape when they are not part of
the static manifest. `datatableQueryParams()` emits exact-match filters as
`f-eq-<field>` so backend query parsing does not confuse operator-prefixed field
names such as `in-stock` with `in` filters. Directly constructed
`DatatableDescriptor` values must use non-empty, trimmed field names, unique
column/mapping/default-sort names, and non-colliding static filter fields for
filterable columns and relation filter aliases.
Runtime manifest export requires `datatable.max_per_page` and
`datatable.max_export_rows` to stay within JavaScript's safe integer range; `0`
remains the documented unlimited sentinel for each cap.
Generated datatable manifests, runtime caps, and id trees are frozen at runtime,
so direct mutation cannot change backend-owned table metadata. Datatable
selector helpers clone entries, columns, mappings, relation-filter aliases, and
default sorts before returning them, so table builders can add local column
state, selection state, or UI-only labels to selector results. Datatable lookup
helpers retrieve a single backend-owned column and normalize relation-filter
aliases to their canonical field names without local `.find(...)` scans.

Route params support Axum `{id}` / `{*path}` patterns and legacy `:id` patterns.
The helper URL-encodes substituted params and throws a clear runtime error if a
required param is missing. Generated TypeScript uses the same RFC3986 path-param
encoding as Rust `RouteRegistry::url`, including escaping `!'()*`. Catch-all
`{*path}` params preserve `/` separators and encode each path segment, while
normal params encode `/` as `%2F`. Routes without path params use an exact empty
params type, so TypeScript rejects accidental param objects; pass route options
directly, for example `routeUrl(RouteIds.health, { basePath: "/api" })`. Query
strings stay typed through `RouteUrlOptions.query`, including repeated values:
`routeUrl(RouteIds.health, { query: { page: 2, tag: ["rust", "dx"] } })`.
`createRouteUrlBuilder(...)` preserves builder-level query defaults and merges
per-call query values over them, so shared filters or tenant hints do not need
to be copied into every route call. Use `routeUrlOrNull(...)` or
`createRouteUrlBuilderOrNull(...)` when user-provided params should produce a
nullable URL instead of throwing a missing-param error. `routeMatch(...)`,
`routeMatches(...)`, and
`routeMatchAny(...)` reuse the same backend-owned path templates to detect active
routes and extract decoded path params from `string` or `URL` inputs; pass the
same `basePath` when matching URLs generated through `createRouteUrlBuilder(...)`.
Use strict selectors such as `routePath(...)`, `routeGuard(...)`, and
`routeResponseByStatus(...)` when the route name is already a typed `RouteName`.
Use the nullable selector companions such as `routePathOrNull(...)`,
`routeGuardOrNull(...)`, `routeRequestTransportForRouteOrNull(...)`, and
`routeResponseByStatusOrNull(...)` when route names come from runtime data and
should not require frontend-local route-name guards.

When a named route has request or response docs, Foundry also writes a typed
endpoint helper under `routes/`. URL-only named routes still appear in
`RouteManifest` and `RouteIds`, but `clientExport` is `false` and no endpoint
helper file is generated until the route documents a request or response schema.
The helper is optional and headless: it does not depend on React, Vue, or a
specific HTTP library. Any Axios-compatible client with `request(config)` works.
`submitForm()` and `submitResponse()` require method metadata from method-aware
registrar or scoped helpers such as `routes.post(...)` / `routes.get(...)`, or
an explicit `options.method`, so unannotated `route_named(...)` registrations
never silently submit with the wrong verb.

```rust
#[derive(Debug, serde::Deserialize, foundry::ts_rs::TS, foundry::ApiSchema, foundry::Validate)]
#[ts(crate = "foundry::ts_rs")]
pub struct LoginRequest {
    #[validate(required, email)]
    pub email: String,

    #[validate(required, min(8))]
    pub password: String,
}

#[derive(Debug, serde::Serialize, foundry::ts_rs::TS, foundry::ApiSchema)]
#[ts(crate = "foundry::ts_rs")]
pub struct LoginResponse {
    pub token: String,
}

routes.post(
    foundry::RouteId::new("user.portal.login"),
    "/login",
    login,
    |route| {
        route
            .request::<LoginRequest>()
            .response::<LoginResponse>(200);
    },
);
```

Generated TypeScript:

```typescript
import {
  FoundryValidationClientError,
  UserPortalLogin,
  type UserPortalLoginFieldErrorDetails,
  type UserPortalLoginFieldErrors,
  type UserPortalLoginFieldErrorsInput,
  type UserPortalLoginFieldValue,
  type UserPortalLoginPatchData,
  type UserPortalLoginPreparedSubmitRequest,
  type UserPortalLoginRequestField,
  type UserPortalLoginRouteUrlOptions,
  type UserPortalLoginState,
  type UserPortalLoginSubmitOptions,
  type UserPortalLoginSubmitUrlOptions,
  type UserPortalLoginTouchedFields,
  type UserPortalLoginValidationAttribute,
  type UserPortalLoginValidationContainer,
  type UserPortalLoginValidationErrorBag,
  type UserPortalLoginValidationField,
  type UserPortalLoginValidationFieldState,
  type UserPortalLoginValidationFieldStates,
  type UserPortalLoginValidationMessage,
  type UserPortalLoginValidationResult,
  type UserPortalLoginValidationRule,
  type UserPortalLoginValidationSchema,
} from "@shared/types/generated";

const loginForm = UserPortalLogin(axios, {
  email: "",
  password: "",
});

loginForm.validateForm();

const response = await loginForm.submitForm();
response.token;
loginForm.busy;
loginForm.processing;
loginForm.submitted;
loginForm.dirty;
loginForm.valid;
loginForm.invalid;
loginForm.errors.email;
loginForm.firstError("email");
loginForm.fieldCodes("email");
const snapshot = loginForm.state();
snapshot.routeName;
snapshot.routeUrl;
snapshot.submitUrl; // string | null
snapshot.submitMode?.submitsAsQuery;
snapshot.submitMode?.requestMediaType;
snapshot.method;
snapshot.requestTransport;
snapshot.requestMediaType;
snapshot.responseStatuses;
snapshot.responseMetadata[0]?.mediaType;
snapshot.responseStatus;
snapshot.responseStatusMetadata?.hasBody;
snapshot.hasDocumentedResponseStatus;
snapshot.validation.valid;
snapshot.valid;
snapshot.invalid;
snapshot.hasErrors;
snapshot.dirty;
snapshot.processing;
snapshot.dirtyFields;
snapshot.firstDirtyField;
snapshot.dirtyFieldStates;
snapshot.firstDirtyFieldState;
snapshot.touched.email;
snapshot.touchedFields;
snapshot.firstTouchedField;
snapshot.touchedFieldStates;
snapshot.firstTouchedFieldState;
snapshot.invalidFields;
snapshot.firstInvalidField;
snapshot.submitted;
snapshot.submitCount;
snapshot.pendingSubmitCount;
snapshot.hasPendingSubmissions;
snapshot.wasSuccessful;
snapshot.recentlySuccessful;
snapshot.progress?.percentage;
snapshot.hasProgress;
snapshot.rawResponse?.status;
const unsubscribe = loginForm.subscribeState((state) => {
  state.busy;
  state.processing;
}, { immediate: true });
const loginState: UserPortalLoginState = loginForm.state();
const loginEmailField: UserPortalLoginRequestField = "email";
const loginEmailValue: UserPortalLoginFieldValue<"email"> = "user@example.com";
loginForm.setField(loginEmailField, loginEmailValue);
const loginTouched: UserPortalLoginTouchedFields = loginForm.touched;
const loginErrors: UserPortalLoginFieldErrors = loginForm.errors;
const loginErrorDetails: UserPortalLoginFieldErrorDetails = loginForm.errorDetails;
const loginValidation: UserPortalLoginValidationResult = loginForm.validationResult();
const loginValidationBag: UserPortalLoginValidationErrorBag = loginValidation;
const loginValidationSchema: UserPortalLoginValidationSchema = loginForm.validation;
const loginClientError = new FoundryValidationClientError(
  loginValidation.errors,
  loginValidation.errorDetails,
);
const loginValidationFieldState: UserPortalLoginValidationFieldState =
  loginClientError.fieldState("email");
const loginValidationFieldStates: UserPortalLoginValidationFieldStates =
  loginClientError.fieldStates(["email", "password"]);
loginForm.setFieldAndValidate("email", "user@example.com");
loginForm.setFieldAndValidateDependents("password", "secret");
loginForm.validateField("email");
loginForm.validateFields(["email", "password"]);
loginForm.validateFieldAndDependents("password_confirmation");
loginForm.setFieldError("email", "Email is already registered.");
const localErrors: UserPortalLoginFieldErrorsInput = {
  password: { code: "weak_password", message: "Choose a stronger password." },
};
loginForm.setFieldErrors(localErrors);
loginForm.setError("email", "Use your work email.");
loginForm.setErrors({ password: ["Password confirmation does not match."] });
loginForm.clearFieldError("password");
loginForm.clearFieldErrors(["email", "password"]);
loginForm.clearError("email");
loginForm.clearErrors("email", "password");
loginForm.touchField("email");
loginForm.touchFields(["email", "password"]);
loginForm.touchFieldAndValidate("email");
loginForm.touchFieldsAndValidate(["email", "password"]);
loginForm.touchFieldAndValidateDependents("password");
loginForm.isFieldTouched("email");
loginForm.resetTouched("email");
loginForm.touchedFields();
loginForm.firstTouchedField();
loginForm.firstTouchedFieldOrNull();
loginForm.touchedFieldStates();
loginForm.firstTouchedFieldState();
loginForm.firstTouchedFieldStateOrNull();
loginForm.isDirty();
loginForm.isFieldDirty("email");
loginForm.dirtyFields();
loginForm.firstDirtyField();
loginForm.firstDirtyFieldOrNull();
loginForm.dirtyFieldStates();
loginForm.firstDirtyFieldState();
loginForm.firstDirtyFieldStateOrNull();
loginForm.setInitialData();
loginForm.setInitialField("email");
loginForm.setInitialFields(["email", "password"]);
loginForm.defaults();
loginForm.defaults("email", "password");
loginForm.resetData();
loginForm.resetField("email");
loginForm.resetFields(["email", "password"]);
loginForm.reset();
loginForm.reset("email", "password");
loginForm.resetAndClearErrors("email", "password");
loginForm.resetSubmitState();
loginForm.clearRecentlySuccessful();
loginForm.clearProgress();

const loginPatch: UserPortalLoginPatchData = { email: "user@example.com" };
loginForm.patchData(loginPatch);
loginForm.setData({ email: "", password: "" });

const raw = await loginForm.submitResponse();
if (raw.status === 200) {
  raw.data.token;
}
loginForm.responseStatuses();
loginForm.responseStatusCount();
loginForm.firstResponseStatus();
loginForm.responseSchemas();
loginForm.hasResponseSchemas();
loginForm.firstResponseSchema();
loginForm.hasResponseSchema("MinimalLoginResponse");
loginForm.responseMetadataForSchema("MinimalLoginResponse");
loginForm.responseMediaTypes();
loginForm.responseMediaTypeCount();
loginForm.firstResponseMediaType();
loginForm.hasResponseMediaType("application/json");
loginForm.firstResponseMetadataForMediaType("application/json");
loginForm.responseBodyFlags();
loginForm.hasResponseBodyFlags();
loginForm.firstResponseBodyFlag();
loginForm.hasResponseBody();
loginForm.responseMetadataForBody(false);
loginForm.responseStatus();
loginForm.hasResponseStatus(200);
loginForm.responseMetadataForStatus(200)?.hasBody;
loginForm.hasDocumentedResponseStatus();
const loginRouteUrlOptions: UserPortalLoginRouteUrlOptions = {
  query: { redirect: "/dashboard" },
};
loginForm.routeUrl(loginRouteUrlOptions);
loginForm.routeUrlOrNull(loginRouteUrlOptions);
loginForm.submitMode().submitsWithBody;
loginForm.submitModeOrNull()?.submitsWithBody;
loginForm.submitUrl();
loginForm.submitUrlOrNull();
const loginSubmitUrlOptions: UserPortalLoginSubmitUrlOptions = {
  query: { preview: true },
};
loginForm.submitUrl(loginSubmitUrlOptions);
const preparedLoginSubmit: UserPortalLoginPreparedSubmitRequest = loginForm.prepareSubmit({
  query: { preview: true },
});
preparedLoginSubmit.method;
preparedLoginSubmit.url;
preparedLoginSubmit.routeName;
preparedLoginSubmit.requestSchema;
preparedLoginSubmit.hasRequestMetadata;
preparedLoginSubmit.headers;
preparedLoginSubmit.config.url;
preparedLoginSubmit.mode.submitsAsQuery;
loginForm.prepareSubmitOrNull({ query: { preview: true } })?.config.url;
loginForm.applyResponse(raw);
loginForm.resetResponse();
loginForm.clearResponse();

await loginForm.submitForm({
  transform: (data) => ({
    ...data,
    email: data.email.trim().toLowerCase(),
  }),
});

const loginSubmitOptions: UserPortalLoginSubmitOptions = {
  recentlySuccessfulDurationMs: 1500,
  onStart: (data) => data.email,
  onSuccess: (response) => response.data,
  onError: (error) => error,
  onProgress: (progress) => progress.percentage,
  onFinish: () => undefined,
};
await loginForm.submitForm(loginSubmitOptions);
await loginForm.submit(loginSubmitOptions);
loginForm.hasResponse();
loginForm.cancel();

try {
  await loginForm.submitForm();
} catch (error) {
  if (error instanceof FoundryValidationClientError) {
    error.firstError("email");
    error.fieldCodes("email");
  }
  loginForm.applyServerError(error);
  loginForm.hasServerError();
  loginForm.hasErrorResponse();
  loginForm.hasValidationErrorResponse();
  loginForm.errorResponse?.message;
  loginForm.errorResponse?.error_code;
  loginForm.validationErrorResponse?.errors[0]?.code;
  loginForm.errors.email;
  loginForm.errorDetails.email?.[0]?.code;
}
```

Generated route metadata values preserve backend-owned literal types while
checking against framework unions, so `loginForm.routeName` stays typed as
`"user.portal.login"`, `UserPortalLoginMethod` stays typed as `"post"`, and
`UserPortalLoginRequestSchema`, `UserPortalLoginRequestTransport`, and
`UserPortalLoginRequestMediaType` stay typed as their exact backend-owned
request schema, transport, and media values instead of widening to every
supported route name, method, schema, transport, or media value.

Routes without a request schema default to an empty request object, so simple GET helpers do not
need a placeholder `{}`:

```typescript
const usersIndex = AdminUsersIndex(axios);
const users = await usersIndex.submitForm();
```

Route helpers for paths with parameters require those params when the endpoint is
created, so the instance owns its URL state:

```typescript
const userShow = AdminUsersShow(axios, { id: userId });
const user = await userShow.submitForm();

userShow.setParams({ id: nextUserId });
userShow.routeUrl();
userShow.routeUrl({ params: { id: nextUserId }, query: { tab: "profile" } });
userShow.submitUrl({ query: { tab: "profile" } });
```

For routes that also have a request body, pass path params before the body data:
`AdminUsersUpdate(axios, { id: userId }, formData)`. Per-submit
`submitForm({ params })` / `submitResponse({ params })` still overrides the
stored params for one request. The backend-owned route manifest includes
`requestTransport` and `requestMediaType`, and generated helpers use them to
decide whether request data is sent as body/query params and whether body
requests use JSON or multipart. GET and HEAD helpers for plain object-shaped
request DTOs whose fields are scalar values or scalar arrays use query
parameters; wrapper request schemas such as `Collection<T>`, nested object DTOs,
and DTOs containing `UploadedFile` stay body/multipart requests.
OpenAPI mirrors that and emits
`x-foundry-request-transport`; body requests also emit
`x-foundry-request-media-type`, so generated API clients and custom tooling do
not need to infer transport or media type from method, schema shape, or
`requestBody.content`. OpenAPI array query
parameters are explicitly marked `style: form` and `explode: true`.
Query-transport endpoint submissions require object-shaped request data and
encode scalar/array fields through the same repeated-key route query encoder as
`routeUrl`, so arrays are sent as `tag=rust&tag=dx` and `null` / `undefined`
values are skipped. Route helper files export `{RouteName}RouteUrlOptions`
aliases for typed `routeUrl(...)` link/action builders. `submitUrl(...)`
returns that same generated submit URL without sending the request, and
`prepareSubmit(...)` returns the resolved method, URL, serialized body or
`FormData`, adapter params, merged headers, full HTTP request config, static
route/request metadata (`routeName`, `path`, `requestSchema`,
`requestTransport`, `requestMediaType`, and request metadata presence flags),
and submit mode from the same backend-owned request-preparation path used by
`submitForm(...)`. Search forms, debug panels, and custom transports can preview
the actual request from endpoint data, params, `query`, `route`, `url`,
transport/media overrides, `headers`, `request.params`, or `transform` without
duplicating route-helper encoding or carrying endpoint metadata beside the
prepared envelope.
Use `routeUrlOrNull(...)`, `submitModeOrNull(...)`, `submitUrlOrNull(...)`, or
`prepareSubmitOrNull(...)` when a UI is working with incomplete params or
user-provided options and should render a disabled/empty state instead of
catching URL-preparation errors itself.
Custom HTTP clients receive the generated query string in `config.url` instead
of raw DTO arrays in `config.params`. Extra URL query values should use
`submitForm({ query })`, `submitResponse({ query })`, `submitUrl({ query })`,
`prepareSubmit({ query })`, or `route.query`;
query-transport submissions reject `options.request.params` so adapter params
cannot compete with the backend-owned DTO query payload. Scalar, array,
collection-wrapper, file, and date root requests stay body or multipart requests
unless the route explicitly documents query transport.

The route file exports the path, method, request alias, response alias, params
type, status-keyed response map, response-status alias, raw response alias,
route-specific state, state-subscriber, submit-options aliases,
submit-url-options aliases, prepare-submit-options aliases, prepared-submit
request aliases, response metadata, validation metadata, endpoint class, and
factory function. Generated response metadata and validation metadata constants
are frozen at runtime so direct mutation cannot change the backend-owned route
contract for every importer. Endpoint instances also expose `responseMetadata`,
`responseStatuses()`, `responseSchemas()`, `responseMediaTypes()`,
`responseBodyFlags()`, matching count/presence/first helpers,
schema/media/body metadata filters,
`hasResponseStatus(...)`, `responseMetadataForStatus(...)`, and
`hasDocumentedResponseStatus(...)`, plus `routeUrl(...)` for building the
endpoint's current route URL, `routeUrlOrNull(...)` for nullable route URL
resolution, `submitUrl(...)` / `submitUrlOrNull(...)` for building the generated
submit URL, `prepareSubmit(...)` / `prepareSubmitOrNull(...)` for previewing the
generated request envelope, and `submitMode(...)` / `submitModeOrNull(...)` for
the resolved method, query/body transport, and multipart/body booleans from the
same backend-owned request metadata. UI adapters can inspect backend-owned
status, `hasBody`, and `mediaType` decisions without indexing the global
manifest.
`submitForm()` returns the successful response-body union for existing
form-style usage; `submit(...)` is its Laravel/Inertia-style alias.
`submitResponse()` returns a status-discriminated response typed from every
documented route response.
When multiple submits run on the same endpoint instance, `busy` remains `true`
until every pending submit settles, but response and error state only follow the
latest submit. Earlier slower responses still resolve or reject their own
promises, but they do not overwrite the endpoint's newer `response`,
`serverError`, `errors`, or validation envelope state.
Endpoint instances and `state()` snapshots expose `pendingSubmitCount` and
`hasPendingSubmissions()` / `state().hasPendingSubmissions` from the same
backend-owned counter, so adapters can render concurrent request details without
local bookkeeping.
`state().routeUrl`, `state().submitUrl`, and `state().submitMode` are nullable
snapshots, so incomplete route params or unsupported submit metadata can be
rendered as disabled UI instead of throwing during state emission.
Starting a new submit clears stale `response`, `rawResponse`, and `status`
before validation and network work so pending UI does not display an older
successful response as current.
Submit options accept a typed `signal`, and endpoint instances expose
`abortPending(reason?)` for canceling generated in-flight submits. Use
`submitForm({ abortPrevious: true })` or `submitResponse({ abortPrevious: true })`
when a new submit should cancel older pending submits on the same endpoint
instance. Custom HTTP clients need to forward `config.signal` to their underlying
fetch/Axios call for the cancellation to stop the network request. `cancel(...)`
is a Laravel/Inertia-style alias for `abortPending(...)`.
Submit options also accept a typed `transform(data)` callback for per-submit
request normalization such as trimming strings or adding derived fields. The
transform receives a cloned generated request DTO, runs before generated
client-side validation and request serialization, and its returned payload is
cloned before request preparation. Mutating the transform input or the
`onStart(...)` payload does not mutate endpoint state or the request body; return
the transformed payload or call a generated setter for persistent changes.
Submit options also include typed `onStart(data)`, `onSuccess(response)`,
`onError(error)`, and `onFinish()` callbacks so form adapters can run
route-specific side effects without wrapping every generated submit call.
`onStart(...)` receives a cloned transformed request DTO, and `onSuccess(...)`
receives a cloned typed Foundry HTTP response envelope for the route response
map.
`onError(...)` runs for transform failures, generated client validation
failures, request serialization failures, HTTP adapter rejections, and
undocumented-status rejections from `submitResponse()`.
Upload-capable adapters can call `config.onUploadProgress(event)` with their
native progress event. Generated endpoints normalize it into
`progress: { loaded, total, progress, percentage, lengthComputable }` on
`state()` snapshots and pass the same normalized value to `onProgress(...)`;
`progress` clears when the latest submit settles, and custom upload flows can
call `clearProgress()` to dismiss progress state manually. Endpoint instances
and snapshots expose `hasProgress` so upload UIs can branch without repeating
`progress !== null`.
Because that type depends on the actual HTTP status, custom adapters used with
`submitResponse()` must return a numeric integer `status`; `submitForm()` still
only needs `data`. The generated runtime passes `FoundryResponseData<TResponses>`
to the HTTP client generic, so adapter wrappers can type both success bodies and
documented non-2xx payloads from the route manifest. `submitResponse()` also
checks that the returned status exists in the generated route manifest; document
additional statuses with route `response::<T>(status)` metadata before relying on
them in status-aware screens.
Guarded routes automatically include Foundry's framework-owned `401` and `403`
`ErrorResponse` branches when they already have a typed endpoint contract, so
status-aware clients can handle auth failures without hand-written response
unions.
If the backend returns Foundry's standard error JSON, the endpoint stores it as
typed `errorResponse` while still keeping the raw adapter error in `serverError`;
validation field messages are copied into `errors`, and code-bearing
`FieldError` details are grouped by field in `errorDetails`. Validation failures
use the generated `ValidationErrorResponse` envelope (`message`, `status`, and
required `errors`) when a route documents `validation_errors()`.
Thrown server validation responses are also stored as typed
`validationErrorResponse` for form UIs that need the raw backend envelope.
Custom adapters and multi-step forms can call `applyServerError(error)` with a
caught HTTP adapter error or raw Foundry error payload to hydrate the same
`serverError`, typed `errorResponse`, typed `validationErrorResponse`, `errors`,
and `errorDetails` state that generated submissions use. Success-side custom
flows can call `applyResponse(response, { recentlySuccessfulDurationMs })` to
hydrate `response`, typed `rawResponse`, `status`, success flags, and the
recent-success pulse while clearing stale upload progress through the same path
generated submissions use; response bodies/envelopes are cloned as they enter
endpoint state. Endpoint instances and `state()` snapshots also
expose `hasResponse`, `hasProgress`, `hasServerError`, `hasErrorResponse`, and
`hasValidationErrorResponse` booleans so form stores do not need to duplicate
nullable-payload checks.
Endpoint instances expose `state()` and `subscribeState(...)` for typed frontend
adapter snapshots plus `validationResult()`, `hasResponse()`,
`responseStatuses()`, `responseSchemas()`, `responseMediaTypes()`,
`responseBodyFlags()`, response metadata count/presence/first helpers,
schema/media/body metadata filters and their `OrNull` first helpers,
`hasResponseStatus(...)`, `hasProgress()`, `hasPendingSubmissions(...)`, `clearProgress(...)`,
`responseStatus()`, `responseMetadataForStatus(...)`,
`responseMetadataForStatusOrNull(...)`,
`hasDocumentedResponseStatus(...)`,
`hasServerError()`, `hasErrorResponse()`, `hasValidationErrorResponse()`, `hasErrors()`,
`errorFieldCount()`, `hasErrorFields()`, `errorFieldCountWithCode(...)`,
`hasErrorFieldWithCode(...)`, `errorFields()`,
`errorMessageCount()`, `hasErrorMessages()`,
`errorMessageCountWithCode(...)`, `hasErrorMessageWithCode(...)`, `errorMessages()`,
`errorDetailCount()`, `hasErrorDetails()`,
`errorDetailCountWithCode(...)`, `hasErrorDetailWithCode(...)`, `allErrorDetails()`,
`errorCodeCount()`, `hasErrorCodes()`, `errorCodes()`,
`errorMessagesWithCode(...)`, `errorDetailsWithCode(...)`, `hasErrorCode(...)`,
`fieldsWithErrorCode(...)`, `firstFieldWithErrorCode(...)`,
`firstFieldWithErrorCodeOrNull(...)`, `firstErrorField()`,
`firstErrorFieldOrNull()`, `firstErrorMessage()`,
`firstErrorMessageOrNull()`, `firstErrorMessageWithCode(...)`,
`firstErrorMessageWithCodeOrNull(...)`, `firstErrorDetail()`,
`firstErrorDetailOrNull()`, `firstErrorDetailWithCode(...)`,
`firstErrorDetailWithCodeOrNull(...)`,
`firstErrorCode()`, `firstErrorCodeOrNull()`, `fieldMessages(...)`, `fieldHasErrors(...)`,
`fieldHasMessages(...)`, `fieldHasDetails(...)`, `fieldHasCodes(...)`,
`fieldHasDetailWithCode(...)`, `fieldHasMessageWithCode(...)`, `fieldHasVisibleErrors(...)`,
`fieldState(...)`, `fieldStates(...)`,
`visibleFieldMessages(...)`, `firstVisibleFieldMessage(...)`,
`firstVisibleFieldMessageOrNull(...)`, `visibleFieldDetails(...)`,
`firstVisibleFieldDetail(...)`, `firstVisibleFieldDetailOrNull(...)`,
`visibleFieldCodes(...)`, `firstVisibleFieldCode(...)`,
`firstVisibleFieldCodeOrNull(...)`,
`visibleFieldDetailsWithCode(...)`, `visibleFieldMessagesWithCode(...)`,
`firstVisibleFieldDetailWithCode(...)`, `firstVisibleFieldDetailWithCodeOrNull(...)`,
`firstVisibleFieldMessageWithCode(...)`, `firstVisibleFieldMessageWithCodeOrNull(...)`,
`dirtyFieldCount(...)`, `hasDirtyFields(...)`, `dirtyFields(...)`, `firstDirtyField(...)`,
`firstDirtyFieldOrNull(...)`, `dirtyFieldStates(...)`,
`firstDirtyFieldState(...)`, `firstDirtyFieldStateOrNull(...)`,
`touchedFieldCount(...)`, `hasTouchedFields(...)`, `touchedFields(...)`, `firstTouchedField(...)`,
`firstTouchedFieldOrNull(...)`, `touchedFieldStates(...)`,
`firstTouchedFieldState(...)`, `firstTouchedFieldStateOrNull(...)`,
`invalidFieldCount(...)`, `hasInvalidFields(...)`, `invalidFields(...)`, `firstInvalidField(...)`,
`firstInvalidFieldOrNull(...)`, `invalidFieldStates(...)`,
`firstInvalidFieldState(...)`, `firstInvalidFieldStateOrNull(...)`,
`fieldStateCountWithErrorCode(...)`, `hasFieldStateWithErrorCode(...)`,
`fieldStatesWithErrorCode(...)`, `firstFieldStateWithErrorCode(...)`,
`firstFieldStateWithErrorCodeOrNull(...)`,
`visibleErrorFieldStates(...)`,
`visibleErrorFieldStateCount(...)`, `hasVisibleErrorFieldStates(...)`,
`firstVisibleErrorFieldState(...)`,
`firstVisibleErrorFieldStateOrNull(...)`,
`visibleErrorFieldStatesWithErrorCode(...)`,
`visibleErrorFieldStateCountWithErrorCode(...)`,
`hasVisibleErrorFieldStatesWithErrorCode(...)`,
`firstVisibleErrorFieldStateWithErrorCode(...)`,
`firstVisibleErrorFieldStateWithErrorCodeOrNull(...)`,
`hasVisibleErrors(...)`, `hasVisibleErrorCode(...)`,
`hasVisibleErrorFields(...)`, `visibleErrorFieldCount(...)`,
`hasVisibleErrorFieldWithCode(...)`, `visibleErrorFieldCountWithCode(...)`,
`visibleErrorFields(...)`, `firstVisibleErrorField(...)`,
`firstVisibleErrorFieldOrNull(...)`,
`visibleErrorFieldsWithCode(...)`, `firstVisibleErrorFieldWithCode(...)`,
`firstVisibleErrorFieldWithCodeOrNull(...)`,
`hasVisibleErrorMessages(...)`, `visibleErrorMessageCount(...)`,
`hasVisibleErrorMessageWithCode(...)`, `visibleErrorMessageCountWithCode(...)`,
`visibleErrorMessages(...)`, `firstVisibleErrorMessage(...)`,
`firstVisibleErrorMessageOrNull(...)`,
`hasVisibleErrorDetails(...)`, `visibleErrorDetailCount(...)`,
`hasVisibleErrorDetailWithCode(...)`, `visibleErrorDetailCountWithCode(...)`,
`visibleErrorDetails(...)`, `firstVisibleErrorDetail(...)`,
`firstVisibleErrorDetailOrNull(...)`,
`hasVisibleErrorCodes(...)`, `visibleErrorCodeCount(...)`,
`visibleErrorCodes(...)`, `firstVisibleErrorCode(...)`,
`firstVisibleErrorCodeOrNull(...)`,
`visibleErrorDetailsWithCode(...)`, `visibleErrorMessagesWithCode(...)`,
`firstVisibleErrorDetailWithCode(...)`, `firstVisibleErrorDetailWithCodeOrNull(...)`,
`firstVisibleErrorMessageWithCode(...)`, `firstVisibleErrorMessageWithCodeOrNull(...)`,
`firstError(...)`, `firstErrorOrNull(...)`,
`firstFieldMessage(...)`, `firstFieldMessageOrNull(...)`, `fieldMessageCount(...)`,
`fieldMessageCountWithCode(...)`, `fieldDetails(...)`,
`fieldDetailCount(...)`, `fieldDetailsWithCode(...)`,
`fieldDetailCountWithCode(...)`, `fieldMessagesWithCode(...)`,
`fieldCodes(...)`, `fieldCodeCount(...)`,
`fieldHasErrorCode(...)`, `fieldHasVisibleErrorCode(...)`,
`firstFieldMessageWithCode(...)`, `firstFieldMessageWithCodeOrNull(...)`,
`firstFieldDetail(...)`, `firstFieldDetailOrNull(...)`,
`firstFieldDetailWithCode(...)`, `firstFieldDetailWithCodeOrNull(...)`,
`firstFieldCode(...)`, `firstFieldCodeOrNull(...)`, `setFieldAndValidate(...)`,
`setFieldAndValidateDependents(...)`, `validateFields(...)`,
`validateField(...)`, `validateFieldAndDependents(...)`, `clearFieldErrors(...)`,
`clearFieldError(...)`, `clearErrors(...)`, `clearError(...)`,
`setFieldErrors(...)`, `setFieldError(...)`,
`setErrors(...)`, `setError(...)`,
`touchField(...)`, `touchFields(...)`, `touchFieldAndValidate(...)`,
`touchFieldsAndValidate(...)`,
`touchFieldAndValidateDependents(...)`, `isFieldTouched(...)`, `resetTouched(...)`,
`setInitialData(...)`, `setInitialFields(...)`, `setInitialField(...)`,
`defaults(...)`,
`dirty`, `isDirty(...)`, `isFieldDirty(...)`, `resetData(...)`,
`resetFields(...)`, `resetField(...)`, `reset(...)`, `resetAndClearErrors(...)`, `resetSubmitState(...)`, `clearRecentlySuccessful(...)`, `clearProgress(...)`, `routeUrl(...)`, `routeUrlOrNull(...)`,
`submit(...)`, `submitUrl(...)`, `submitUrlOrNull(...)`, `prepareSubmit(...)`, `prepareSubmitOrNull(...)`, `submitMode(...)`, `submitModeOrNull(...)`, `hasRequestSchema(...)`, `hasRequestTransport(...)`, `hasRequestMediaType(...)`, `hasRequestMetadata(...)`, `resetResponse(...)`, `clearResponse(...)`, `applyValidationResult(...)`,
`applyResponse(...)`, `applyServerError(...)`, and `cancel(...)`, all backed by the same
generated validation result, form-state, and server-error helpers used by custom
form hooks.
`subscribeState(...)` can emit the current snapshot immediately with
`{ immediate: true }`, and returns the same unsubscribe function shape as
`subscribe(...)`. Route helper files export `{RouteName}RequestField` and
`{RouteName}FieldValue<Field>` aliases for typed field props and reusable field
components, plus `{RouteName}PatchData`, `{RouteName}TouchedFields`,
`{RouteName}FieldErrors`, `{RouteName}FieldErrorDetails`,
`{RouteName}ValidationErrorBag`, `{RouteName}ValidationResult`,
`{RouteName}ValidationRule`, `{RouteName}ValidationField`,
`{RouteName}ValidationMessage`, `{RouteName}ValidationAttribute`,
`{RouteName}ValidationContainer`, `{RouteName}ValidationSchema`,
`{RouteName}ValidationFieldState`, and `{RouteName}ValidationFieldStates`
aliases for form stores, validation metadata rows, and standalone
client-validation flows that mirror generated endpoint slices. The generic
`FoundryEndpointState`, `FoundryTouchedFields`,
and `FoundryEndpointStateSubscriber` types, plus route-specific aliases such as
`UserPortalLoginState`, `UserPortalLoginFieldState`, `UserPortalLoginFieldStates`,
`UserPortalLoginStateSubscriber`, and
`UserPortalLoginSubmitOptions` / `UserPortalLoginSubmitUrlOptions`, keep
React/Vue stores aligned with the generated endpoint runtime instead of copying
local state interfaces. Snapshots include static route metadata (`routeName`,
`path`, `routeUrl`, `method`, `requestSchema`, `requestTransport`,
`requestMediaType`, request metadata presence flags (`hasRequestSchema`,
`hasRequestTransport`, `hasRequestMediaType`, `hasRequestMetadata`),
`responseMetadata`, `responseStatuses`, `responseSchemas`, `responseMediaTypes`,
`responseBodyFlags`, and their count/presence/first summaries) plus current
response-status derivations
(`responseStatus`, `responseStatusMetadata`, and
`hasDocumentedResponseStatus`), current generated submit URL (`submitUrl`, or
`null` when the URL cannot be built from the current endpoint metadata/data),
current submit mode (`submitMode`, or `null` when it cannot be resolved),
`initialData`, `dirty`, `processing`, `touched`,
`submitted`, `submitCount`, `wasSuccessful`, `recentlySuccessful`, upload
`progress`, `hasProgress`, typed `rawResponse`, and generated `fieldStates`;
request/form/response snapshot values such as `data`, `initialData`,
`routeParams`, `touched`, `errors`, `errorDetails`, `progress`, `response`,
`rawResponse`, `responseMetadata`, `responseStatuses`,
`responseSchemas`, `responseMediaTypes`, `responseBodyFlags`,
`responseStatusMetadata`, and `validation` are cloned so stores cannot mutate
endpoint internals by mutating the snapshot object; primitive response
metadata count/presence/first summaries are emitted directly as read-only
snapshot fields.
`touchField(...)`, `isFieldTouched(...)`, and `resetTouched(...)` use the same
exact, nested, indexed, and root-container path matching as validation and field
reads.
`fieldState(...)` returns the backend-owned per-field validation snapshot in one
object (`valid`, `invalid`, messages, details, codes, and first message/detail/code).
On generated endpoint instances it also includes `touched`, `dirty`,
`submitted`, and `shouldShowErrors`, so form adapters can share one display-state
shape without local reducers. `fieldStates()` maps every generated schema field
and reachable nested child path, plus current validation error fields and
currently touched fields, to the same object by default. Server-only,
unknown-field, exact indexed error keys, and exact indexed touched keys stay
visible to subscribers; pass a field list to limit the map to a custom subset or
exact indexed paths.
`state().fieldStates` contains that generated default field-state map for
subscribers, and `state().dirtyFieldCount`, `state().dirtyFields`,
`state().hasDirtyFields`, `state().firstDirtyField`, `state().dirtyFieldStates`,
`state().firstDirtyFieldState`, `state().touchedFieldCount`,
`state().hasTouchedFields`, `state().touchedFields`, `state().firstTouchedField`,
`state().touchedFieldStates`, `state().firstTouchedFieldState`,
`state().invalidFieldCount`, `state().hasInvalidFields`, `state().invalidFields`,
`state().firstInvalidField`, `state().invalidFieldStates`,
`state().firstInvalidFieldState`,
`state().visibleErrorFieldStates`, and `state().firstVisibleErrorFieldState`
expose the common derived counts and lists from the same map. Snapshots also include
`state().errorFieldCount`, `state().errorFields`, `state().firstErrorField`,
`state().errorMessageCount`, `state().errorMessages`,
`state().firstErrorMessage`, `state().errorDetailCount`,
`state().allErrorDetails`, `state().firstErrorDetail`,
`state().errorCodeCount`, `state().errorCodes`, and
`state().firstErrorCode` for whole-form validation summaries, plus
`state().hasVisibleErrors`, `state().visibleErrorFieldCount`,
`state().visibleErrorFields`, `state().firstVisibleErrorField`,
`state().visibleErrorMessageCount`, `state().visibleErrorMessages`,
`state().firstVisibleErrorMessage`, `state().visibleErrorDetailCount`,
`state().visibleErrorDetails`, `state().firstVisibleErrorDetail`,
`state().visibleErrorCodeCount`, `state().visibleErrorCodes`, and
`state().firstVisibleErrorCode`, so
React/Vue/Svelte stores do not need to call back into the endpoint or flatten a
snapshot after receiving it.
Snapshots also expose `valid`, `invalid`, and `hasErrors` beside
`validation.valid` for stores that branch on whole-form validity.
`clearFieldErrors(...)` clears a generated field subset in one state emission,
defaulting to the same generated field-state key list when no fields are passed.
`clearErrors(...)` and `clearError(...)` are Laravel-style aliases; call
`clearErrors("email", "password")` for a variadic subset or `clearErrors()` for
the whole generated error bag, including backend-returned fields outside the
generated field-state key list.
`setFieldErrors(...)` / `setFieldError(...)` replace a generated field subset
with local/client validation errors in the same `errors` and `errorDetails`
bags used by backend validation, accepting either string messages or
`FieldError`-shaped `{ code, message }` objects. Route helper files export
`{RouteName}FieldErrorsInput` aliases for these local error bags.
`setErrors(...)` replaces the whole generated error bag, so `setErrors({})`
clears every field; use `setFieldErrors(...)` for partial merges.
`setError(...)` is the Laravel-style single-field alias. Applying local errors
also clears stale success/response state so an invalid generated endpoint does
not keep showing an old successful submit or response.
Use `dirtyFieldCount(...)`, `hasDirtyFields(...)`, `dirtyFields(...)`,
`firstDirtyField(...)`, `firstDirtyFieldOrNull(...)`, `dirtyFieldStates(...)`,
and `firstDirtyFieldState(...)` / `firstDirtyFieldStateOrNull(...)` when a form
needs changed field-state totals, presence checks, fields, or rows from the
clean baseline.
Use `touchedFieldCount(...)`, `hasTouchedFields(...)`, `touchedFields(...)`,
`firstTouchedField(...)`, `firstTouchedFieldOrNull(...)`,
`touchedFieldStates(...)`, and `firstTouchedFieldState(...)` /
`firstTouchedFieldStateOrNull(...)` when a form needs interacted field-state
totals, presence checks, fields, or rows from the same generated map.
Use `invalidFieldCount(...)`, `hasInvalidFields(...)`, `invalidFields(...)`,
`firstInvalidField(...)`, `firstInvalidFieldOrNull(...)`,
`invalidFieldStates(...)`, `firstInvalidFieldState(...)`, and
`firstInvalidFieldStateOrNull(...)` when a form needs currently invalid
field-state totals, presence checks, fields, or rows. Use
`fieldStateCountWithErrorCode(...)`, `hasFieldStateWithErrorCode(...)`,
`fieldStatesWithErrorCode(...)`, and
`firstFieldStateWithErrorCode(...)` /
`firstFieldStateWithErrorCodeOrNull(...)` when a summary needs full field-state
rows for a backend-owned rule code plus matching count/presence reads. Use
`foundryValidationFieldStateHasErrors(...)`,
`foundryValidationFieldStateMessages(...)`,
`foundryValidationFieldStateDetails(...)`,
`foundryValidationFieldStateCodes(...)`, first-value row helpers,
row presence helpers,
`foundryValidationFieldStateMessageCount(...)`,
`foundryValidationFieldStateDetailCount(...)`,
`foundryValidationFieldStateCodeCount(...)`,
`foundryValidationFieldStateDetailsWithCode(...)`,
`foundryValidationFieldStateMessagesWithCode(...)`, first/count `WithCode`
variants, and first-value `OrNull` variants when a component receives a
generated validation field-state row directly and should not duplicate optional
checks, `?? null`, or count/filter/map logic over backend-owned errors. Use
`visibleErrorFieldStates(...)` /
`firstVisibleErrorFieldState(...)` /
`firstVisibleErrorFieldStateOrNull(...)` when the UI should only show touched or
submitted field errors, and
`visibleErrorFieldStatesWithErrorCode(...)` /
`visibleErrorFieldStateCountWithErrorCode(...)` /
`hasVisibleErrorFieldStatesWithErrorCode(...)` /
`firstVisibleErrorFieldStateWithErrorCode(...)` /
`firstVisibleErrorFieldStateWithErrorCodeOrNull(...)` when a summary needs
visible error rows, counts, presence, or first state for one rule code. Use
`OrNull` first helpers when stores use `null` for missing first values. Use `foundryEndpointFieldStateHasVisibleErrorCode(...)`
for a single endpoint field-state row, or `fieldHasVisibleErrorCode(...)` on an
endpoint instance when a field component needs one field/rule-code boolean.
Use `visibleFieldMessages(...)`, `firstVisibleFieldMessage(...)`,
`firstVisibleFieldMessageOrNull(...)`, `visibleFieldMessageCount(...)`,
`visibleFieldDetails(...)`, `firstVisibleFieldDetail(...)`,
`firstVisibleFieldDetailOrNull(...)`, `visibleFieldDetailCount(...)`,
`visibleFieldCodes(...)`, `firstVisibleFieldCode(...)`,
`firstVisibleFieldCodeOrNull(...)`, `visibleFieldCodeCount(...)`,
`fieldHasVisibleMessages(...)`, `fieldHasVisibleDetails(...)`,
`fieldHasVisibleCodes(...)`, and their `WithCode` / `OrNull` variants when a field
component should read only touched/submitted display errors without duplicating
the generated `shouldShowErrors` rule or local `.length` wrappers.
Use `hasVisibleErrors(...)`, `hasVisibleErrorCode(...)`,
`visibleErrorFieldCount(...)`, `visibleErrorFieldCountWithCode(...)`,
`visibleErrorFields(...)`, `visibleErrorMessageCount(...)`,
`visibleErrorMessageCountWithCode(...)`, `visibleErrorMessages(...)`,
`visibleErrorDetailCount(...)`, `visibleErrorDetailCountWithCode(...)`,
`visibleErrorDetails(...)`, `visibleErrorCodeCount(...)`,
`visibleErrorCodes(...)`, and their first-value / `WithCode` / `OrNull` variants when a
form-level summary needs only touched/submitted display errors without
flattening visible field-state rows locally; subscribed stores can read the same
boolean, counts, and aggregate field/message/detail/code arrays from `state()`.
`submitCount` increments for every generated submit attempt, including
client-side validation failures, while `wasSuccessful` only follows the latest
completed backend submit. `recentlySuccessful` turns on for the latest
successful submit and clears automatically after
`recentlySuccessfulDurationMs` milliseconds, defaulting to 2000.
`dirty` is available as an endpoint getter backed by `isDirty()`, `processing`
is a Laravel/Inertia-style alias for `busy`, and `submitted` is available on
both the endpoint instance and `state()` snapshots. `valid` and `invalid` are
also available on the endpoint instance for adapters that branch without first
creating a snapshot.
`setInitialData()` marks the current request body as the clean baseline,
`setInitialFields(...)` / `setInitialField(...)` mark a generated field subset
as clean from the current request body, and `defaults(...)` is the
Laravel/Inertia-style alias for the same clean-baseline update. Generated
endpoints clone full-form and field-level clean-baseline request values when
they are captured or restored, so mutating current form data does not
accidentally mutate `initialData`.
`dirty` / `isDirty()` compare data against that
baseline, `isFieldDirty("email")` compares one generated field path,
`resetData()` restores a clean baseline value while clearing generated error
state, `resetFields(...)` restores a generated field subset from the clean
baseline in one state emission, and
`reset(...)` is the Laravel/Inertia-style alias for restoring all data or a
variadic field subset, and `resetAndClearErrors(...)` is an alias for the same
generated reset path because reset already clears matching generated errors.
`resetSubmitState()` clears the submit lifecycle counters and recent-success
pulse when a reused form should look new again. `clearRecentlySuccessful()`
dismisses only the recent-success pulse and keeps `submitCount`,
`wasSuccessful`, progress, responses, and errors intact. `resetResponse()` / `clearResponse()` clear the
last response body, raw response envelope, and status without touching the
request data or generated validation errors.
`validateForm()`, `validateField(...)`, and `applyValidationResult(...)` clear
stale backend error envelopes before replacing the generated field bag, and
clear stale success/response state when the resulting endpoint has validation
errors. Each submission clears stale response/status at submit start, applies
the same validation-envelope reset before client validation, clears stale
`errors` and `errorDetails` before sending the request, then repopulates them
from the current client validation result or server `422` response.
Server-error application also clears stale successful response state before
hydrating the error envelope.
Use `setField("email", value)` from input handlers when a form should update one
generated field path and clear that field's generated messages without throwing
away the whole bag. Use `setFieldAndValidate(...)` when a change should update
the DTO and immediately refresh that field's backend-owned client rules, or
`setFieldAndValidateDependents(...)` when sibling rules such as `confirmed`,
`required_if`, or temporal comparisons should refresh the fields that reference
the changed value. These writes accept the same nested and indexed path family as
validation, so `setField("profile.name", value)`,
`setField("children[0].name", value)`, `[0].email`, and `items[0].email` can be
written through the generated endpoint instead of a custom path helper. Use
`validateField("email")` on blur when the value was already written elsewhere,
`touchFieldAndValidate("email")` when blur should mark the field touched and
refresh errors in one emitted state update, `touchFieldAndValidateDependents(...)`
when blur should also refresh sibling-rule dependents, `touchFields([...])` when
a step should mark a known subset as touched, `touchFieldsAndValidate([...])`
when that step should also refresh its validation state, and `validateFields([...])`
when a step should refresh a known subset without changing touched state.
Root container request schemas such as `Array<Dto>` and `Collection<Dto>`
validate the selected field across items and update container paths such as
`[0].email` or `items[0].email`. Nested DTO paths also work:
`validateField("profile.name")` validates the child rule only, `validateField("children.name")`
refreshes that child field across `each(nested)` items, and exact generated
paths such as `validateField("children[0].name")` or `validateField("[tenant].email")`
target one item. `FoundryKnownFieldPath<T>` preserves those known nested and
indexed paths for generated DTOs while endpoint helpers still accept runtime-only
backend paths. Malformed array or collection indexes such as
`children[abc].name`, `[tenant].email` on an array request, or
`items[abc].email` on a collection request are ignored instead of validating
every item. Nested root containers use the same prefix family, so generated
paths such as `[tenant][0].email` and `items[0][1].email` still read, validate,
and clear through the inner field name `email`. Field read helpers use the same
path-family matching, so
`fieldMessages("children.name")`, `fieldDetails(...)`,
`fieldDetailsWithCode(...)`, `fieldMessagesWithCode(...)`, `fieldCodes(...)`,
`fieldHasErrorCode(...)`, `fieldState(...)`, `fieldStates(...)`,
`hasDirtyFields(...)`, `dirtyFields(...)`, `firstDirtyField(...)`,
`firstDirtyFieldOrNull(...)`, `dirtyFieldStates(...)`,
`firstDirtyFieldState(...)`, `firstDirtyFieldStateOrNull(...)`,
`hasTouchedFields(...)`, `touchedFields(...)`, `firstTouchedField(...)`,
`firstTouchedFieldOrNull(...)`, `touchedFieldStates(...)`,
`firstTouchedFieldState(...)`, `firstTouchedFieldStateOrNull(...)`,
`hasInvalidFields(...)`, `invalidFields(...)`, `firstInvalidField(...)`,
`firstInvalidFieldOrNull(...)`, `invalidFieldStates(...)`,
`firstInvalidFieldState(...)`, `firstInvalidFieldStateOrNull(...)`,
`firstFieldMessage(...)`, `firstFieldMessageOrNull(...)`,
`firstFieldDetail(...)`, `firstFieldDetailOrNull(...)`,
`firstFieldCode(...)`, `firstFieldCodeOrNull(...)`, `firstError(...)`, and
`firstErrorOrNull(...)` include generated item paths such as
`children[0].name`.
Client-side throws use `FoundryValidationClientError`, which exposes
`validationResult()`, `hasErrors()`, `errorFieldCount()`,
`hasErrorFields()`, `errorFieldCountWithCode(...)`,
`hasErrorFieldWithCode(...)`, `errorFields()`, `errorMessageCount()`,
`hasErrorMessages()`, `errorMessageCountWithCode(...)`,
`hasErrorMessageWithCode(...)`, `errorMessages()`,
`errorMessagesWithCode(...)`, `errorDetailCount()`,
`hasErrorDetails()`, `errorDetailCountWithCode(...)`,
`hasErrorDetailWithCode(...)`, `allErrorDetails()`,
`errorDetailsWithCode(...)`, `errorCodeCount()`, `hasErrorCodes()`, `errorCodes()`,
`hasErrorCode(...)`, `fieldsWithErrorCode(...)`, `firstFieldWithErrorCode(...)`,
`firstFieldWithErrorCodeOrNull(...)`, `firstErrorField()`,
`firstErrorFieldOrNull()`, `firstErrorMessage()`,
`firstErrorMessageOrNull()`, `firstErrorMessageWithCode(...)`,
`firstErrorMessageWithCodeOrNull(...)`, `firstErrorDetail()`,
`firstErrorDetailOrNull()`, `firstErrorDetailWithCode(...)`,
`firstErrorDetailWithCodeOrNull(...)`, `firstErrorCode()`,
`firstErrorCodeOrNull()`,
`fieldMessages(...)`, `fieldHasErrors(...)`, `fieldHasMessages(...)`,
`fieldHasDetails(...)`, `fieldHasCodes(...)`, `fieldHasDetailWithCode(...)`,
`fieldHasMessageWithCode(...)`, `fieldState(...)`, `fieldStates(...)`,
`invalidFieldCount(...)`, `hasInvalidFields(...)`, `invalidFields(...)`, `firstInvalidField(...)`,
`firstInvalidFieldOrNull(...)`, `invalidFieldStates(...)`,
`firstInvalidFieldState(...)`, `firstInvalidFieldStateOrNull(...)`,
`fieldStateCountWithErrorCode(...)`, `hasFieldStateWithErrorCode(...)`,
`fieldStatesWithErrorCode(...)`, `firstFieldStateWithErrorCode(...)`,
`firstFieldStateWithErrorCodeOrNull(...)`,
`firstError(...)`, `firstErrorOrNull(...)`, `fieldMessageCount(...)`, `fieldMessageCountWithCode(...)`,
`fieldDetails(...)`, `fieldDetailCount(...)`,
`fieldDetailsWithCode(...)`, `fieldDetailCountWithCode(...)`,
`fieldHasDetailWithCode(...)`, `fieldMessagesWithCode(...)`,
`fieldMessageCountWithCode(...)`, `fieldHasMessageWithCode(...)`,
`fieldCodes(...)`, `fieldCodeCount(...)`, `fieldHasCodes(...)`,
`fieldHasErrorCode(...)`, `firstFieldMessage(...)`, `firstFieldMessageOrNull(...)`,
`firstFieldMessageWithCode(...)`, `firstFieldMessageWithCodeOrNull(...)`,
`firstFieldDetail(...)`, `firstFieldDetailOrNull(...)`,
`firstFieldDetailWithCode(...)`, `firstFieldDetailWithCodeOrNull(...)`, and
`firstFieldCode(...)` / `firstFieldCodeOrNull(...)` with the same path matching as the generated endpoint
instance. The error owns cloned error bags, so mutating a caught error does not
mutate the endpoint that produced it.
Standalone validation result helpers accept `{ includeContainerPaths: true }`
when a custom root container form store needs `fieldMessages(result, "email")`
to include `[0].email`, `[tenant].email`, or `items[0].email` keys. Use
`clearFieldErrors(["email", "password"])`, `clearFieldError("email")`, or
`clearFieldError("children.name")` when the UI only needs to clear messages.
Use `clearErrors("email", "password")` / `clearError("email")` when a
Laravel-style adapter wants the same behavior without wrapping the generated
endpoint.
Use `setFieldErrors({ email: "Email is already registered." })` or
`setFieldError("email", { code: "reserved", message: "Email is reserved." })`
when local client checks or external widgets need to hydrate the generated error
bag without keeping a parallel store.
`setData()` replaces any request body. Generated setters clone accepted
request/form values (`setData(...)`, `patchData(...)`, `setField(...)`,
`setFieldAndValidate(...)`, `setInitialData(...)`, `resetData(...)`,
`setParams(...)`, and `patchParams(...)`), so mutating an object after passing it
to the endpoint does not change endpoint state without a generated setter call.
`resetData()` restores the current clean baseline, or accepts a new clean value
when the backend returns updated defaults after a save. `setInitialFields(...)`
and `setInitialField(...)` mark a generated field subset clean after partial
saves without replacing the rest of `initialData`; `defaults(...)` is the
Laravel/Inertia-style alias for the same operation.
`resetFields(["email", "password"])` restores a generated field subset in one
state emission. `resetField("email")`, `resetField("children.name")`, or exact indexed paths such as
`resetField("children[0].name")` restore one generated path from `initialData`
and clear that path's generated errors, using the same path-family matching as
validation and field reads. `reset(...)` is the Laravel/Inertia-style alias for
restoring all data or a variadic field subset. `resetAndClearErrors(...)`
delegates to the same generated reset path because matching errors are already
cleared by `resetData(...)` / `resetFields(...)`. `patchData()` stays typed for object-like request
bodies only, including nullable object DTOs and string-keyed maps; use
`setField()` for generated field paths and `setData()` for replacing arrays,
files, scalars, and unit/no-body requests. For complex
screens, import only the DTOs or constants you need:

```typescript
import type {
  UserPortalLoginRequest,
  UserPortalLoginResponse,
  UserPortalLoginSubmitOptions,
} from "@shared/types/generated";
```

Request and response aliases preserve common container shapes from route docs:
`Vec<T>`, `BTreeSet<T>`, and `HashSet<T>` export as `T[]`, `Collection<T>`
exports as `{ items: T[] }`, string-keyed `BTreeMap<String, T>` /
`HashMap<String, T>` exports as `Record<string, T>`, and `Option<T>` exports as
`T | null`. Foundry scalar route schemas such as `DateTime`, `Date`, `Time`,
`Timezone`, `Numeric`, `ModelId`, semantic IDs, and `Uuid` export as `string`;
`UploadedFile` exports as browser `File`. `serde_json::Value` route schemas and
DTO fields export through Foundry's generated recursive `JsonValue` union, and
string-keyed maps of JSON values export as `Record<string, JsonValue>`.
Route helpers also export `{RouteName}RequestSchema` plus
`{RouteName}RequestSchemaName`, `{RouteName}RequestTransportValue`, and
`{RouteName}RequestMediaTypeValue` aliases, plus
`{RouteName}HasRequestSchema`, `{RouteName}HasRequestTransport`,
`{RouteName}HasRequestMediaType`, and `{RouteName}HasRequestMetadata`
constants, so schema explorers, debug panels, and custom adapters can read or
branch on the route-local request schema, transport, and media metadata without
local `RouteManifest` lookups, `typeof ...` wrappers, or `!== null` checks.
Unit/no-content route responses documented with
`response::<()>(204)` export as `void`; OpenAPI output omits JSON content for
those bodyless responses and marks response objects with
`x-foundry-response-has-body` plus `x-foundry-response-media-type` for
body-bearing responses. `RouteManifestResponse.hasBody` / `mediaType` expose the
same body and content-type decision for frontend tooling; generated endpoint
helpers also expose that array as `{RouteName}ResponseMetadata` and
`endpoint.responseMetadata`. Route helpers also export
`{RouteName}ResponseMetadataList` and `{RouteName}ResponseMetadataEntry` aliases
derived from that generated constant, so status-aware screens and docs tooling
can type backend-owned response metadata without local
`typeof {RouteName}ResponseMetadata[number]` wrappers. A frozen
`{RouteName}ResponseStatuses` constant exposes the backend-owned documented
status list without a local metadata `.map(...)`, and
`{RouteName}ResponseSchemas` plus `{RouteName}ResponseSchema` expose the
route-local response schema-name list and union without local schema metadata
maps. `{RouteName}ResponseMediaTypes`, `{RouteName}ResponseBodyFlags`,
`{RouteName}ResponseMediaType`, and `{RouteName}ResponseBodyFlag` expose the
route-local response content-type and body-presence lists and unions without
local metadata maps. `{RouteName}ResponseStatusCount`,
`{RouteName}HasResponseStatuses`, `{RouteName}FirstResponseStatus`,
`{RouteName}FirstResponseStatusOrNull`, and the schema/media/body count,
presence, first-value, and `OrNull` first-value variants expose static response
metadata summaries without local `.length`, `.length > 0`, `[0]`, or
`... ?? null` wrappers. Route helpers also export
`{RouteName}ResponseStatusMetadata<Status>`,
`{RouteName}HasResponseStatus(status)`, and
`{RouteName}ResponseMetadataForStatus(status)` plus
`{RouteName}ResponseMetadataForStatusOrNull(status)` so status-aware adapters can
guard arbitrary status numbers and read route-specific response metadata without
local `responseMetadata.find(...)` or `... ?? null` wrappers. Static route helpers also expose
schema/media/body response metadata filters such as
`{RouteName}HasResponseSchema(...)`,
`{RouteName}ResponseMetadataForSchema(...)`,
`{RouteName}FirstResponseMetadataForMediaType(...)`,
`{RouteName}FirstResponseMetadataForMediaTypeOrNull(...)`,
`{RouteName}HasResponseBody(...)`, and
`{RouteName}ResponseMetadataForBody(...)` so docs, download adapters, and
no-content checks can avoid local `responseMetadata.some(...)`, `.filter(...)`,
`.find(...)`, or `... ?? null` wrappers. The shared `RouteManifest.ts` surface also exports
`routeResponsesWithSchema(...)`, `routeResponsesWithMediaType(...)`,
`routeResponsesWithBody(...)`, and their count, presence, and first-response
variants so generic route explorers can filter one route's backend-owned
response metadata without local `routeResponses(name).filter(...)` wrappers.
Responses
documented with `response::<UploadedFile>(...)` or
`response::<Vec<UploadedFile>>(...)`, including nullable `Option<_>` wrappers,
use `application/octet-stream` in OpenAPI and route manifest metadata instead of
JSON. No-body route requests documented with
`request::<()>()` export as `void`, and their generated helper defaults the
request argument to `undefined` so callers do not need placeholder data. OpenAPI
route docs inline nullable and generic wrapper schemas so `Option<T>`, `Vec<T>`,
`Collection<T>`, string-keyed maps, `PaginatedResponse<T>`, and
`CursorPaginated<T>` cannot collide under shared component names. Route responses documented with
`response::<PaginatedResponse<T>>(...)` export as
`{ data: T[]; meta: PaginationMeta; links: PaginationLinks }`, and
`response::<CursorPaginated<T>>(...)` exports as
`{ data: T[]; meta: CursorMeta; cursors: CursorInfo }`.
Nullable request bodies documented with `request::<Option<T>>()` export as
`T | null`; their helpers default the body to `null`, skip client validation for
`null`/`undefined`, and still apply the inner DTO validation rules when an object
is provided. Array and string-keyed map request bodies documented with
`request::<Vec<T>>()`, `request::<BTreeSet<T>>()`, `request::<HashSet<T>>()`,
`request::<Collection<T>>()`, `request::<HashMap<String, T>>()`, or
`request::<BTreeMap<String, T>>()` also reuse `T`'s generated validation
metadata for each array item, collection `items` entry, or map value, keeping
bulk endpoints on the same DTO contract as single-object endpoints. Nested
request containers such as
`HashMap<String, Vec<T>>` walk every container layer and still validate the
innermost DTO. Collection request validation reports item paths like
`items[0].email`, matching the JSON wrapper shape.
Each generated route helper also exports `{RouteName}Responses`, keyed by HTTP
status code, `{RouteName}ResponseStatus`, and `{RouteName}RawResponse` for
status-aware clients that need to distinguish `201`, `204`, validation errors,
or other documented outcomes. Endpoint `state().rawResponse` stores the latest
documented raw response envelope with that status-keyed type when the HTTP
client supplies a status, while `state().responseStatus` and
`state().responseStatusMetadata` expose the documented current status and its
backend-owned `hasBody` / `mediaType` metadata without snapshot-side scans. The
endpoint and snapshot response schema/media/body arrays are derived from the
same backend-owned response metadata as `{RouteName}ResponseSchemas`,
`{RouteName}ResponseMediaTypes`, and `{RouteName}ResponseBodyFlags`. The
endpoint response metadata filters return cloned rows for schema/media/body
criteria without local `responseMetadata.filter(...)` scans. The
raw/status-aware path requires the adapter response to include `status`; data-only
adapters can keep using `submitForm()`. It also rejects statuses that are not in
the route manifest, keeping the raw response union aligned with backend docs.
Route helper generation rejects unresolved request or response schema names so
the generated client cannot silently widen a backend-owned contract to `unknown`.
Client-exported endpoint helpers must also document at least one `2xx` response;
use `response::<()>(204)` for bodyless success routes, document the actual
success DTO, or disable client export for routes that should only appear in
`RouteManifest`.
Manual `ApiSchema` implementations that document container-like route roots must
also preserve Foundry's inner schema markers (`x-foundry-item-schema`,
`x-foundry-additional-schema`, or `x-foundry-data-schema`), or use the built-in
`Vec<T>`, set, string-keyed map, `Collection<T>`, and pagination schema
implementations.

Request DTO files are also marked as generated and include validation remarks
above fields when the Rust DTO derives `Validate`:

```typescript
// Auto-generated from Foundry types. Do not edit.
export type LoginRequest = {
  // Validation: required, email
  email: string,
  // Validation: required, min(8)
  password: string,
};
```

Basic `#[validate(...)]` rules are embedded into the generated validation schema
and run in `validateForm()`. File metadata rules that can be checked from the
browser `File` object (`max_file_size`, `allowed_extensions`) also run before
submit. Server-backed rules such as `unique`, `exists`, custom
`rule(MY_RULE_ID)` validators, and content-sniffing file rules (`image`,
`allowed_mimes`, image dimensions) are kept as server-only metadata.
Struct-level `#[validate(after(...))]` hooks are exported the same way on the
generated validation schema. Custom rules use the registered rule id as the
generated metadata `code`; the backend remains the final source of validation
truth through `JsonValidated<T>` and `Validated<T>`.
The generated `{RouteName}Validation` constant is frozen at runtime; use schema
selectors, which clone returned rule/message/attribute metadata, for derived UI
metadata instead of mutating the generated contract.
Generated endpoint runtimes also export `validateFoundrySchema(...)`,
`validateFoundrySchemaField(...)`, `assertFoundrySchemaValid(...)`,
`foundryValidationResultFromError(...)`, and `FoundryValidationResult` field
helpers such as
`emptyFoundryValidationResult()`, `foundryValidationFirstMessage(...)`,
`foundryValidationFirstMessageOrNull(...)`,
`foundryValidationFirstFieldMessage(...)`,
`foundryValidationFirstFieldMessageOrNull(...)`,
`foundryValidationFieldMessageCount(...)`,
`foundryValidationFieldMessageCountWithCode(...)`,
`foundryValidationFieldCodes(...)`, `foundryValidationFieldCodeCount(...)`,
`foundryValidationFieldHasErrorCode(...)`,
`foundryValidationFieldDetailCount(...)`,
`foundryValidationFieldDetailCountWithCode(...)`,
`foundryValidationFieldDetailsWithCode(...)`,
`foundryValidationFieldMessagesWithCode(...)`,
`foundryValidationFirstFieldMessageWithCode(...)`,
`foundryValidationFirstFieldMessageWithCodeOrNull(...)`,
`foundryValidationFirstFieldDetail(...)`,
`foundryValidationFirstFieldDetailOrNull(...)`,
`foundryValidationFirstFieldDetailWithCode(...)`,
`foundryValidationFirstFieldDetailWithCodeOrNull(...)`,
`foundryValidationFirstFieldCode(...)`,
`foundryValidationFirstFieldCodeOrNull(...)`, `foundryValidationFieldHasErrors(...)`,
`foundryValidationFieldState(...)`, `foundryValidationFieldStateHasErrorCode(...)`,
`foundryValidationFieldStateHasErrors(...)`,
`foundryValidationFieldStateMessages(...)`,
`foundryValidationFieldStateFirstMessage(...)`,
`foundryValidationFieldStateFirstMessageOrNull(...)`,
`foundryValidationFieldStateMessageCount(...)`,
`foundryValidationFieldStateMessageCountWithCode(...)`,
`foundryValidationFieldStateDetails(...)`,
`foundryValidationFieldStateFirstDetail(...)`,
`foundryValidationFieldStateFirstDetailOrNull(...)`,
`foundryValidationFieldStateDetailCount(...)`,
`foundryValidationFieldStateDetailCountWithCode(...)`,
`foundryValidationFieldStateCodes(...)`,
`foundryValidationFieldStateFirstCode(...)`,
`foundryValidationFieldStateFirstCodeOrNull(...)`,
`foundryValidationFieldStateCodeCount(...)`,
`foundryValidationFieldStateHasMessages(...)`,
`foundryValidationFieldStateHasDetails(...)`,
`foundryValidationFieldStateHasCodes(...)`,
`foundryValidationFieldStateDetailsWithCode(...)`,
`foundryValidationFieldStateHasDetailWithCode(...)`,
`foundryValidationFieldStateMessagesWithCode(...)`,
`foundryValidationFieldStateHasMessageWithCode(...)`,
`foundryValidationFieldStateFirstDetailWithCode(...)`,
`foundryValidationFieldStateFirstDetailWithCodeOrNull(...)`,
`foundryValidationFieldStateFirstMessageWithCode(...)`,
`foundryValidationFieldStateFirstMessageWithCodeOrNull(...)`,
`foundryValidationFieldStateCountWithErrorCode(...)`,
`foundryValidationFieldStates(...)`,
`foundryValidationFieldStatesWithErrorCode(...)`,
`foundryValidationHasFieldStateWithErrorCode(...)`,
`foundryValidationHasInvalidFields(...)`,
`foundryValidationInvalidFields(...)`,
`foundryValidationFirstInvalidField(...)`,
`foundryValidationFirstInvalidFieldOrNull(...)`,
`foundryValidationInvalidFieldStates(...)`,
`foundryValidationFirstFieldStateWithErrorCode(...)`,
`foundryValidationFirstFieldStateWithErrorCodeOrNull(...)`,
`foundryValidationFirstInvalidFieldState(...)`,
`foundryValidationFirstInvalidFieldStateOrNull(...)`,
`foundryEndpointDirtyFields(...)`,
`foundryEndpointHasDirtyFields(...)`,
`foundryEndpointFirstDirtyField(...)`,
`foundryEndpointFirstDirtyFieldOrNull(...)`,
`foundryEndpointDirtyFieldStates(...)`,
`foundryEndpointFirstDirtyFieldState(...)`,
`foundryEndpointFirstDirtyFieldStateOrNull(...)`,
`foundryEndpointTouchedFields(...)`,
`foundryEndpointHasTouchedFields(...)`,
`foundryEndpointFirstTouchedField(...)`,
`foundryEndpointFirstTouchedFieldOrNull(...)`,
`foundryEndpointTouchedFieldStates(...)`,
`foundryEndpointFirstTouchedFieldState(...)`,
`foundryEndpointFirstTouchedFieldStateOrNull(...)`,
`foundryEndpointVisibleErrorFieldStates(...)`,
`foundryEndpointVisibleErrorFieldStateCount(...)`,
`foundryEndpointHasVisibleErrorFieldStates(...)`,
`foundryEndpointFirstVisibleErrorFieldState(...)`,
`foundryEndpointFieldStateHasVisibleErrorCode(...)`,
`foundryEndpointVisibleErrorFieldStatesWithErrorCode(...)`,
`foundryEndpointVisibleErrorFieldStateCountWithErrorCode(...)`,
`foundryEndpointHasVisibleErrorFieldStatesWithErrorCode(...)`,
`foundryEndpointFirstVisibleErrorFieldStateWithErrorCode(...)`, plus summary helpers such as
`foundryValidationErrorFields(...)`, `foundryValidationErrorMessages(...)`,
`foundryValidationErrorMessagesWithCode(...)`,
`foundryValidationErrorDetails(...)`, `foundryValidationErrorCodes(...)`,
`foundryValidationErrorDetailsWithCode(...)`,
`foundryValidationHasErrorCode(...)`, `foundryValidationFieldsWithErrorCode(...)`,
`foundryValidationFirstFieldWithErrorCode(...)`,
`foundryValidationFirstFieldWithErrorCodeOrNull(...)`,
`foundryValidationFirstErrorField(...)`,
`foundryValidationFirstErrorFieldOrNull(...)`,
`foundryValidationFirstErrorMessage(...)`,
`foundryValidationFirstErrorMessageOrNull(...)`,
`foundryValidationFirstErrorMessageWithCode(...)`,
`foundryValidationFirstErrorMessageWithCodeOrNull(...)`,
`foundryValidationFirstErrorDetail(...)`,
`foundryValidationFirstErrorDetailOrNull(...)`,
`foundryValidationFirstErrorDetailWithCode(...)`,
`foundryValidationFirstErrorDetailWithCodeOrNull(...)`, and
`foundryValidationFirstErrorCode(...)` /
`foundryValidationFirstErrorCodeOrNull(...)`, so Vue/React form hooks can run the
backend-owned schema directly, normalize Foundry 422 responses or generated
validation error bags without losing `errorDetails` rule codes, and read
field-level, summary, or first-error messages/codes from either
`FoundryValidationResult` or `FoundryValidationClientError` without constructing
a route endpoint instance. Use the `OrNull` first-value helpers when stores use
`null` for missing field-level, field-state, or summary first values. Generated
schema validators compute `valid` from both
`errors` and `errorDetails`, so detail-only bags stay invalid. Summary and field
message helpers also read
`errorDetails` messages when a custom store has detailed errors without a
parallel `errors` message map. `foundryValidationFieldCodes(...)` and
`fieldState(...).codes` return first-seen unique rule codes for a field; use
`foundryValidationFieldStateHasErrorCode(...)` for single field-state rows and
`fieldDetails(...)` when a UI needs every repeated backend detail.
Generated validation message and attribute lookup removes all indexed path
segments before falling back to base metadata, so metadata for
`addresses.streetName` applies to client-side errors such as
`addresses[0].streetName`.
Schema metadata selectors such as `foundryValidationSchemaFields(...)`,
`foundryValidationSchemaReachableFields(...)`,
`foundryValidationSchemaReachableFieldNames(...)`,
`foundryValidationSchemaReachableRules(...)`,
`foundryValidationSchemaReachableRuleCodes(...)`,
`foundryValidationSchemaFieldStateFields(...)`,
`foundryValidationSchemaField(...)`,
`foundryValidationSchemaFieldOrNull(...)`,
`foundryValidationSchemaFieldNameOrNull(...)`,
`foundryValidationSchemaFieldRules(...)`, `foundryValidationSchemaFieldRuleCodes(...)`,
`foundryValidationSchemaFieldRule(...)`,
`foundryValidationSchemaFieldRuleOrNull(...)`,
`foundryValidationSchemaFieldRuleParam(...)`,
`foundryValidationSchemaFieldRuleValues(...)`,
`foundryValidationSchemaFieldRuleNestedRules(...)`,
`foundryValidationSchemaFieldRuleSchema(...)`,
`foundryValidationSchemaRule(...)`, `foundryValidationSchemaRuleOrNull(...)`,
`foundryValidationSchemaRuleParam(...)`,
`foundryValidationSchemaRuleValues(...)`,
`foundryValidationSchemaRuleNestedRules(...)`,
`foundryValidationSchemaRuleSchema(...)`, `foundryValidationRuleParam(...)`,
`foundryValidationRuleValues(...)`, `foundryValidationRuleNestedRules(...)`,
`foundryValidationRuleSchema(...)`,
`foundryValidationSchemaMessages(...)`,
`foundryValidationSchemaMessageFields(...)`,
`foundryValidationSchemaMessageRuleCodes(...)`,
`foundryValidationSchemaMessagesByField(...)`,
`foundryValidationSchemaMessagesByRule(...)`,
`foundryValidationSchemaMessage(...)`,
`foundryValidationSchemaMessageOrNull(...)`,
`foundryValidationSchemaCustomMessage(...)`,
`foundryValidationSchemaCustomMessageOrNull(...)`,
`foundryValidationSchemaRuleMessage(...)`,
`foundryValidationSchemaAttributes(...)`,
`foundryValidationSchemaAttributeFields(...)`,
`foundryValidationSchemaAttributesByField(...)`,
`foundryValidationSchemaAttribute(...)`,
`foundryValidationSchemaAttributeOrNull(...)`,
`foundryValidationSchemaFieldLabel(...)`,
`foundryValidationSchemaFieldLabels(...)`,
`foundryValidationSchemaFieldLabelFields(...)`,
`foundryValidationSchemaFieldLabelCount(...)`,
`foundryValidationSchemaHasFieldLabels(...)`,
`foundryValidationSchemaFirstFieldLabelField(...)`,
`foundryValidationSchemaFirstFieldLabel(...)`,
`foundryValidationSchemaKnownFields(...)`,
`foundryValidationSchemaStrictFields(...)`,
`foundryValidationSchemaHasKnownField(...)`,
`foundryValidationSchemaKnownFieldOrNull(...)`,
`foundryValidationSchemaDeniesUnknownFields(...)`,
`foundryValidationSchemaAllowsUnknownFields(...)`, and
`foundryValidationSchemaUnknownFields(...)`,
`foundryValidationSchemaUnknownFieldCount(...)`,
`foundryValidationSchemaHasUnknownFields(...)`, and
`foundryValidationSchemaFirstUnknownField(...)`, plus their generated
first-selector `OrNull` companions, let dynamic form builders and validation
docs read the same backend-owned schema, copy maps, and strict-field contract
without hand-scanning raw `FoundryValidationSchema` objects. Field
lookup and field rule selectors resolve reachable nested child paths such as
`primaryAddress.streetName` while `foundryValidationSchemaFields(...)` remains
the direct immediate-field list for the current schema. Use
`foundryValidationSchemaReachableFields(...)` or
`foundryValidationSchemaReachableFieldNames(...)` when a form builder wants the
complete parent-prefixed nested field contract. `*OrNull(...)` schema lookup
helpers normalize runtime field names, rule codes, message keys, attribute fields,
and known-field checks into explicit `null` results, so dynamic form builders do
not need local optional-chaining wrappers around backend-owned validation
metadata. Message, attribute, label, and
rule-message selectors include parent-prefixed nested child metadata such as
`primaryAddress.streetName` or
`previousAddresses.streetName`, and concrete indexed reads such as
`previousAddresses[0].streetName` resolve against that same backend-owned
nested metadata.
Filtered message selectors and copy selectors accept plain field strings, so
server-returned paths or concrete collection paths can be passed directly
without app-local casts.
`foundryValidationSchemaUnknownFields(...)` traverses root array, map, and
collection request containers and returns the same container-prefixed paths as
generated validation errors, such as `[0].extra`, `[tenant].extra`, or
`items[0].extra`, typed as `FoundryRequestField<TRequest>` so they can flow back
into generated endpoint field APIs without casts. Selectors
that return schema fields, rules, nested rule schema, messages, attributes,
container lists, nullable-item lists, or strict-field lists clone those values,
so mutating a selector result cannot mutate the generated validation contract.
Count helpers mirror the same schema metadata selectors:
`foundryValidationSchemaRuleCount(...)`,
`foundryValidationSchemaClientRuleCount(...)`,
`foundryValidationSchemaControlRuleCount(...)`,
`foundryValidationSchemaServerOnlyRuleCount(...)`,
`foundryValidationSchemaCustomRuleCount(...)`,
`foundryValidationSchemaFieldCount(...)`,
`foundryValidationSchemaFieldRuleCount(...)`,
`foundryValidationSchemaFieldClientRuleCount(...)`,
`foundryValidationSchemaFieldControlRuleCount(...)`,
`foundryValidationSchemaFieldServerOnlyRuleCount(...)`,
`foundryValidationSchemaFieldCustomRuleCount(...)`,
`foundryValidationSchemaFieldReferenceCount(...)`,
field grouping counts such as
`foundryValidationSchemaFieldCountWithClientRules(...)`,
`foundryValidationSchemaFieldCountWithServerOnlyRules(...)`,
`foundryValidationSchemaFieldCountWithRuleCode(...)`,
`foundryValidationSchemaFieldCountReferencing(...)`,
field-name grouping counts such as
`foundryValidationSchemaFieldNameCountWithClientRules(...)`,
`foundryValidationSchemaFieldNameCountWithServerOnlyRules(...)`,
`foundryValidationSchemaFieldNameCountWithRuleCode(...)`,
`foundryValidationSchemaFieldNameCountReferencing(...)`,
required/nullability counts, message/attribute counts, root container counts,
nullable-item counts, and known/strict-field counts. Rule-row helpers also expose
`foundryValidationRuleValueCount(...)`,
`foundryValidationRuleFieldReferenceCount(...)`, and
`foundryValidationRuleNestedRuleCount(...)`, so diagnostics and form-builder
badges do not need local `.length` wrappers over backend-owned metadata arrays.
Schema and field rule wrappers mirror those count helpers with
`foundryValidationSchemaRuleValueCount(...)`,
`foundryValidationSchemaFieldRuleCodeCount(...)`,
`foundryValidationSchemaFieldRuleValueCount(...)`,
`foundryValidationSchemaFieldRuleFieldReferenceCount(...)`, and
`foundryValidationSchemaFieldRuleNestedRuleCount(...)`.
Presence helpers mirror those counts with generated booleans such as
`foundryValidationSchemaHasRules(...)`,
`foundryValidationSchemaHasClientRules(...)`,
`foundryValidationSchemaHasServerOnlyRules(...)`,
`foundryValidationSchemaHasCustomRules(...)`,
`foundryValidationSchemaHasFields(...)`,
`foundryValidationSchemaFieldHasRules(...)`,
`foundryValidationSchemaHasFieldsWithRuleCode(...)`,
`foundryValidationSchemaHasFieldNamesWithRuleCode(...)`,
`foundryValidationSchemaHasFieldsReferencing(...)`,
`foundryValidationSchemaHasFieldNamesReferencing(...)`,
`foundryValidationSchemaHasMessages(...)`,
`foundryValidationSchemaHasAttributes(...)`,
`foundryValidationSchemaHasKnownFields(...)`,
`foundryValidationSchemaHasStrictFields(...)`, and
`foundryValidationRuleHasValues(...)`, plus rule-wrapper checks such as
`foundryValidationSchemaHasRuleValues(...)`,
`foundryValidationSchemaFieldHasRuleFieldReferences(...)`, and
`foundryValidationSchemaFieldHasRuleCodes(...)`,
`foundryValidationSchemaFieldHasRuleNestedRules(...)`, so form sections can
branch on backend-owned schema metadata without local `count(...) > 0` wrappers.
First-value helpers complete the same metadata set with generated reads such as
`foundryValidationSchemaFirstRule(...)`,
`foundryValidationSchemaFirstField(...)`,
`foundryValidationSchemaFieldFirstRule(...)`,
`foundryValidationSchemaFirstFieldWithRuleCode(...)`,
`foundryValidationSchemaFirstMessage(...)`,
`foundryValidationSchemaFirstAttribute(...)`,
`foundryValidationSchemaFirstKnownField(...)`,
their generated `OrNull` companions, and
`foundryValidationRuleFirstValue(...)`, so inspectors and form builders do not
need local `[0]` or `... ?? null` access over generated metadata arrays.
Nullable companions such as `foundryValidationSchemaFirstRuleOrNull(...)`,
`foundryValidationSchemaFieldFirstRuleCodeOrNull(...)`,
`foundryValidationSchemaFirstMessageOrNull(...)`,
`foundryValidationRuleFirstValueOrNull(...)`,
`foundryValidationRuleFirstFieldReferenceOrNull(...)`, and
`foundryValidationRuleFirstNestedRuleOrNull(...)` return explicit `null` for
empty metadata lists.
Direct optional lookups also expose nullable companions, including
`foundryValidationSchemaContainerOrNull(...)`,
`foundryValidationSchemaReachableRuleOrNull(...)`,
`foundryValidationSchemaRuleParamOrNull(...)`,
`foundryValidationSchemaRuleSchemaOrNull(...)`,
`foundryValidationSchemaFieldRuleParamOrNull(...)`,
`foundryValidationSchemaFieldRuleSchemaOrNull(...)`,
`foundryValidationRuleParamOrNull(...)`,
`foundryValidationRuleSchemaOrNull(...)`, and
`foundryValidationRuleCustomRuleIdOrNull(...)`.
Endpoint instances mirror the same backend-owned schema metadata with
`validationSchemaFields()`, `validationSchemaFieldRules(field)`,
`validationSchemaFieldRuleCodeCount(field)`,
`validationSchemaFieldHasRuleCodes(field)`,
`validationSchemaFieldFirstRule(field)`,
`validationSchemaContainerOrNull()`,
`validationSchemaReachableRuleOrNull(code)`,
`validationSchemaRuleParamOrNull(code, param)`,
`validationSchemaRuleSchemaOrNull(code)`,
`validationSchemaFieldRuleParamOrNull(field, code, param)`,
`validationSchemaFieldRuleSchemaOrNull(field, code)`,
`validationSchemaFieldsWithRuleCode(code)`,
`validationSchemaFirstFieldWithRuleCode(code)`,
`validationSchemaMessages(field?)`, `validationSchemaFirstMessage(field?)`,
`validationSchemaFirstMessageOrNull(field?)`,
`validationSchemaAttributes()`, `validationSchemaFieldLabel(field)`,
`validationSchemaCustomMessage(field, rule)`,
`validationSchemaRuleMessage(field, rule)`, `validationSchemaKnownFields()`,
`validationSchemaUnknownFields(data?)`, `validationSchemaUnknownFieldCount(data?)`,
`validationSchemaHasUnknownFields(data?)`, and
`validationSchemaFirstUnknownField(data?)` /
`validationSchemaFirstUnknownFieldOrNull(data?)`.
Route-local form builders can stay on the generated endpoint object instead of
wrapping `endpoint.validation` with app-local selector, count, presence, or
first-item nullable helpers.
`FoundryClientValidationRuleCodes`, `FoundryValidationControlRuleCodes`,
`FoundryValidationRequiredRuleCodes`,
`FoundryValidationFieldReferenceRuleCodes`, `FoundryValidationRuntimeRuleCodes`,
the `foundry...RuleCodes()` selector helpers, and the
`isFoundry...RuleCode(...)` guards expose the generated runtime's
browser-checkable/control rule surface. The rule-code lists are frozen at
runtime, and selector helpers return fresh arrays plus generated
count/presence/first helpers such as
`foundryClientValidationRuleCodeCount()`,
`foundryValidationControlHasRuleCodes()`,
`foundryValidationFieldReferenceFirstRuleCode()`, and
`foundryValidationRuntimeFirstRuleCode()` so form tooling can add local labels
or display grouping to selector results. Nullable first-selector companions such
as `foundryClientValidationFirstRuleCodeOrNull()`,
`foundryValidationControlFirstRuleCodeOrNull()`,
`foundryValidationRequiredFirstRuleCodeOrNull()`,
`foundryValidationFieldReferenceFirstRuleCodeOrNull()`, and
`foundryValidationRuntimeFirstRuleCodeOrNull()` return explicit `null` for
empty backend-owned rule-code lists. Safe parser helpers such as
`foundryClientValidationRuleCodeOrNull(...)`,
`foundryValidationControlRuleCodeOrNull(...)`,
`foundryValidationRequiredRuleCodeOrNull(...)`,
`foundryValidationFieldReferenceRuleCodeOrNull(...)`, and
`foundryValidationRuntimeRuleCodeOrNull(...)` normalize raw rule-code strings
into generated unions,
while `foundryValidationRuleIsClientCheckable(...)`,
`foundryValidationRuleIsControl(...)`, `foundryValidationRuleIsServerOnly(...)`,
`foundryValidationRuleIsCustom(...)`, `foundryValidationRuleCustomRuleId(...)`,
`foundryValidationRuleHasCustomRuleId(...)`,
and the schema/field rule filters such as
`foundryValidationSchemaFieldClientRules(...)`,
`foundryValidationSchemaFieldControlRules(...)`, and
`foundryValidationSchemaFieldServerOnlyRules(...)` classify individual metadata
rules. `foundryValidationSchemaFieldsWithClientRules(...)`,
`foundryValidationSchemaFieldNamesWithClientRules(...)`,
`foundryValidationSchemaFieldsWithControlRules(...)`,
`foundryValidationSchemaFieldNamesWithControlRules(...)`,
`foundryValidationSchemaFieldsWithServerOnlyRules(...)`, and
`foundryValidationSchemaFieldNamesWithServerOnlyRules(...)` identify fields by
browser-checkable, control-only, or server-only rule classification without
local schema scans. `foundryValidationSchemaFieldHasClientRules(...)`,
`foundryValidationSchemaFieldHasControlRules(...)`,
`foundryValidationSchemaFieldHasServerOnlyRules(...)`,
`foundryValidationSchemaFieldHasCustomRules(...)`, and
`foundryValidationSchemaFieldHasFieldReferences(...)` expose the same
classification as field-level booleans for form controls.
`foundryValidationSchemaReachableRules(...)`,
`foundryValidationSchemaReachableRuleCodes(...)`,
`foundryValidationSchemaReachableRulesWithCode(...)`, and reachable
classification helpers such as `foundryValidationSchemaReachableClientRules(...)`
expose the complete nested rule contract across schema rules, fields,
`each(...)`, and `nested` child schemas; `foundryValidationSchemaRules(...)`
remains the immediate root-schema rule list. Root code helpers such as
`foundryValidationSchemaRuleCodes(...)`,
`foundryValidationSchemaRulesWithCode(...)`,
`foundryValidationSchemaRuleCodeCount(...)`, and
`foundryValidationSchemaHasRuleCode(...)` summarize or filter immediate
root-schema rules without local `.map(...)`, `.filter(...)`, or `.length`
wrappers. Rule-code summary selectors return first-seen unique backend rule
codes; use the rule row selectors when repeated occurrences matter.
`foundryValidationSchemaFieldRuleCodeCount(...)`,
`foundryValidationSchemaFieldHasRuleCodes(...)`,
`foundryValidationSchemaFieldHasRuleCode(...)`,
`foundryValidationSchemaFieldsWithRuleCode(...)`, and
`foundryValidationSchemaFieldNamesWithRuleCode(...)` find fields by backend-owned
rule code without local `field.rules.some(...)` scans. Required/nullability
helpers such as `foundryValidationRuleIsNullable(...)`,
`foundryValidationRuleIsBail(...)`, `foundryValidationRuleIsRequired(...)`,
`foundryValidationRuleIsConditionalRequired(...)`,
`foundryValidationRuleIsRequiredRule(...)`,
`foundryValidationSchemaFieldIsNullable(...)`,
`foundryValidationSchemaFieldIsRequired(...)`,
`foundryValidationSchemaFieldHasConditionalRequiredRule(...)`,
`foundryValidationSchemaFieldHasRequiredRule(...)`,
`foundryValidationSchemaNullableFields(...)`,
`foundryValidationSchemaNullableFieldNames(...)`,
`foundryValidationSchemaRequiredFields(...)`,
`foundryValidationSchemaRequiredFieldNames(...)`,
`foundryValidationSchemaConditionallyRequiredFields(...)`,
`foundryValidationSchemaConditionallyRequiredFieldNames(...)`,
`foundryValidationSchemaFieldsWithRequiredRules(...)`, and
`foundryValidationSchemaFieldNamesWithRequiredRules(...)` let form builders render
required badges, conditional-required hints, nullable inputs, and
validation-control state from generated metadata. `filled` is included in
required/presence helpers because generated and backend validation both reject
absent or empty `filled` values; `required_keys` is an object-key rule and is not
included in the field-presence helper list.
Sibling-field dependency helpers such as
`foundryValidationFieldReferenceMatches(...)`,
`foundryValidationRuleFieldReferences(...)`,
`foundryValidationRuleFieldReferencesForField(...)`,
`foundryValidationRuleHasFieldReferences(...)`,
`foundryValidationRuleReferencesField(...)`,
`foundryValidationRuleReferencesFieldForField(...)`,
`foundryValidationSchemaFieldRuleFieldReferences(...)`,
`foundryValidationSchemaFieldRuleReferencesField(...)`,
`foundryValidationSchemaFieldReferences(...)`,
`foundryValidationSchemaFieldDependsOn(...)`,
`foundryValidationSchemaFieldsReferencing(...)`, and
`foundryValidationSchemaFieldNamesReferencing(...)` let form adapters inspect
fields affected by `required_if`, `required_with_all`, `confirmed`, temporal
comparison, prohibition, and accepted/declined conditional rules without copying
Foundry's sibling-rule list or parsing `params.other`. Endpoint instance
methods keep `FoundryRequestField<TRequest>` return types for these
field-reference lists and first-item helpers, so dependent-field widgets keep
the generated request field union instead of falling back to plain strings. Use
`foundryValidationRuleFieldReferencesForField(...)` when a rule is attached to a
concrete nested/indexed field and the UI needs the sibling references resolved to
the same field path; count, first, nullable-first, and presence companions are
available, along with `foundryValidationRuleReferencesFieldForField(...)` for
path-resolved predicate checks. Use
`foundryValidationSchemaFieldRuleReferencesField(...)` when a form needs to test
whether one specific rule code on a field references another field. Use
`foundryValidationDependentFieldName(...)` and
`foundryValidationSchemaDependentFieldNames(...)` when an adapter needs the
concrete runtime field paths to revalidate after a change; matching count, first,
and presence helpers are available as
`foundryValidationSchemaDependentFieldNameCount(...)`,
`foundryValidationSchemaFirstDependentFieldName(...)`, and
`foundryValidationSchemaHasDependentFieldNames(...)`. Generated endpoint
instances use the same mapped metadata through `validateFieldAndDependents(...)`
when a changed field should refresh itself and fields that reference it.
Dependency matching uses the same path-family rules as validation reads, so a
reference to `children.name` matches `children[0].name`, parent object changes
match child references, and root-container paths such as `[0].email` or
`items[0].email` match an `email` reference while dependent refreshes preserve
the same address, for example `[0].emailConfirmation` or
`items[0].emailConfirmation`. References inside `nested` and `each(nested)`
child schemas are parent-prefixed before dependency matching, so a child rule
such as `confirmed(accessCodeConfirmation)` under `primaryAddress.accessCode`
is exposed as `primaryAddress.accessCodeConfirmation`, while
`previousAddresses[0].accessCodeConfirmation` refreshes
`previousAddresses[0].accessCode`. Browser-side conditional and sibling rule
execution resolves those references through the generated field-path reader
after checking for an exact flat key, so rules can target `profile.enabled`,
`children[0].status`, or `children.status` without copying those values into
top-level shadow fields.
Custom-rule selectors such
as `foundryValidationSchemaCustomRuleIds(...)`,
`foundryValidationSchemaCustomRulesWithRuleId(...)`,
`foundryValidationSchemaReachableCustomRulesWithRuleId(...)`, and
`foundryValidationSchemaFieldCustomRulesWithRuleId(...)` read explicit
`rule(...)`/`TsValidationRule::custom(...)` metadata without local
`params.rule` scans. Custom-rule ID selectors return first-seen unique backend
rule IDs; use the custom-rule row selectors when repeated occurrences matter.
`foundryValidationSchemaFieldsWithCustomRules(...)` and
`foundryValidationSchemaFieldNamesWithCustomRules(...)` identify fields that
carry backend custom checks without hand-scanning every field rule. This lets
form builders and validation docs distinguish client-side prevalidation,
validation controls, and server-only custom/database/file-sniffing checks
without duplicating Foundry's rule-code list. Field classification selectors for
client, control, server-only, custom, required, nullable, and rule-code groups
return reachable nested-schema field paths and include reachable `each(...)`
item rules, so `primaryAddress.streetName` appears as the
required/client-checkable field for a nested child rule while collection fields
with `each(required)` do not need local recursive scans to be treated as
browser-checkable. Lookup and rule-code selectors such as
`foundryValidationSchemaFieldRule(...)`,
`foundryValidationSchemaFieldHasRuleCode(...)` and
`foundryValidationSchemaFieldsWithRuleCode(...)` use the same reachable-rule
view, so `each(required)` is found by `required` rule-code queries.
When `required`, `filled`,
or a prohibited/prohibits rule fails, generated helpers stop evaluating later
rules for that field, matching backend validation's early exit even when the
field does not use `bail`. Numeric DTO fields using `#[validate(size(...))]`
are checked as exact numeric values with the same small floating-point tolerance
as the Rust validator; string DTO fields keep character-length semantics. URL
rules reject literal whitespace before browser URL normalization, matching the
backend parser guard. Timezone rules validate against the backend `chrono_tz`
timezone table plus `UTC` and fixed offsets, so browser-only aliases such as
`PST` are rejected before submit.
`#[validate(json)]` on `serde_json::Value` and nullable JSON value fields is
exported as server-only metadata because those fields are already parsed JSON;
string fields with `#[validate(json)]` still run client-side JSON text
prevalidation.
Generated endpoint runtimes embed the backend app timezone and use it when
validating or comparing offset-less `datetime`, `before`, `after`, and
`date_equals` values, matching Rust's app-timezone interpretation instead of the
browser's local timezone. Date parsing also follows Rust's chrono shape:
unsigned years use four digits, while expanded years require an explicit sign
such as `+10000-01-01`.
Nullable collection fields skip validation only when the value is absent/null;
present empty arrays still run collection rules such as `min_items`, matching
`Option<Vec<T>>` backend validation.
Generated `regex` / `not_regex` checks translate common Rust regex syntax such
as leading `(?i)`, `(?m)`, `(?s)` flags and `(?P<name>...)` captures before
running in the browser; manual regex metadata must still contain a valid Rust
regex pattern. Derived regex rules that Foundry cannot safely translate to
JavaScript are emitted as `serverOnly` metadata instead of browser checks;
manual client-checkable regex metadata with those patterns fails export until
the rule is marked `.server_only()`.
Generated `contains` checks use `params.value` for the single string-substring
case and `values` for all-required collection-style contains metadata, so manual
metadata follows the same contract shape as derive-generated collection rules.
Generated fallback, inline, and custom validation messages derive `{{value}}`
from that same `values` list when no explicit `params.value` exists, and
`required_keys` derives `{{keys}}` from its values list.
For `each(...)` validation, generated client-side messages resolve display
attributes and custom messages through the base field name, so an item error such
as `tags[0]` uses the same `tags` label and `messages(tags(...))` overrides that
Rust uses. Inline/custom client messages also interpolate `{{attribute}}` and
rule parameters from the generated validation metadata.
For root container request schemas such as `Array<CreateUserRequest>` or
`Map<CreateUserRequest>`, client-side validation keys are prefixed before the
DTO field, for example `[0].email` or `[tenant].email`. Generated endpoint
field reads aggregate those container-prefixed keys when the caller asks for the
DTO field, and standalone error-bag reads can opt into the same behavior with
`FoundryValidationFieldReadOptions.includeContainerPaths`. Form builders and
docs UIs can inspect the same backend-owned root-container contract through
`FoundryValidationContainer`, `FoundryValidationContainers`,
`isFoundryValidationContainer(...)`, `foundryValidationContainerOrNull(...)`,
`foundryValidationSchemaIsNullable(...)`,
`foundryValidationSchemaContainer(...)`, `foundryValidationSchemaContainers(...)`,
`foundryValidationSchemaContainerCount(...)`,
`foundryValidationSchemaFirstContainer(...)`,
`foundryValidationSchemaHasContainers(...)`,
`foundryValidationSchemaHasRootContainer(...)`,
`foundryValidationSchemaHasContainer(...)`,
`foundryValidationSchemaItemsAreNullable(...)`, and
`foundryValidationSchemaNullableItems(...)`.
DTOs that implement `RequestValidator` manually can opt into the same generated
route-helper metadata by implementing `TsValidationSchemaProvider` and
submitting a `TsValidation` inventory entry. Use `TsValidationSchema::new()`,
`.field(...)`, and `TsValidationRule::new(...)` to describe browser-checkable
rules, and mark async/database/custom checks with `.server_only()`. Unknown
non-server-only rule codes fail export because the generated TypeScript runtime
cannot prevalidate them. Schema-level manual rules added with
`TsValidationSchema::rule(...)` must also be server-only because generated
browser validation currently runs field-level rules only. The same inventory
metadata is merged into generated OpenAPI request schemas, so custom docs and
clients can read one backend-owned validation contract.
`types:export` also emits `ValidationRuleManifest.ts` from the runtime
`RuleRegistry`; it lists registered custom validation rule ids as server-only
checks for frontend form builders or docs UIs that need to discover backend-only
rules without duplicating names. Use `validationRuleIds()`,
`validationRuleIdCount()`, `validationRuleHasIds()`,
`validationRuleFirstId()`, `validationRuleFirstIdOrNull()`,
`isValidationRuleId()`,
`validationRuleIdOrNull()`, `validationRuleNameOrNull()`,
`validationRuleManifestEntryOrNull()`, `validationRuleManifestEntryById()`,
`validationRuleManifestEntryByIdOrNull()`,
`validationRuleIsRegistered()`,
`validationRuleIdForNameOrNull()`, `validationRuleIdIsServerOnly()`,
`validationRuleIsServerOnlyOrNull()`, `validationRuleNames()`,
`validationRuleNameCount()`, `validationRuleHasNames()`,
`validationRuleFirstEntryOrNull()`, `validationRuleFirstNameOrNull()`,
`serverOnlyValidationRuleIds()`,
`serverOnlyValidationRuleIdCount()`, `serverOnlyValidationRuleHasIds()`,
`serverOnlyValidationRuleFirstId()`, `serverOnlyValidationRuleFirstIdOrNull()`,
`serverOnlyValidationRuleNames()`,
`serverOnlyValidationRuleNameCount()`, `serverOnlyValidationRuleHasNames()`,
`serverOnlyValidationRuleFirstName()`,
`serverOnlyValidationRuleFirstNameOrNull()`, `serverOnlyValidationRules()`,
`serverOnlyValidationRuleCount()`, `serverOnlyValidationRuleHasEntries()`, and
`serverOnlyValidationRuleFirstEntry()`,
`serverOnlyValidationRuleFirstEntryOrNull()` when the UI needs to validate,
list, filter, or summarize backend-owned rule ids with explicit `null`
contracts rather than raw manifest entry objects or local first selector wrappers.
App-backed `types:export` checks explicit `#[validate(rule(...))]` and
`TsValidationRule::custom(...)` metadata against the runtime rule registry before
generated frontend files are written, so stale custom rule references fail
early even when no custom rules are registered. Manual custom metadata must use
the same registered rule id for the rule `code` and `params.rule`.
Generated DTO validation comments render those custom checks as `rule(id)
[server]`, matching the Rust validation syntax while keeping the generated
metadata keyed by the registered rule id.
Each DTO may have one `TsValidation` entry; `types:export` and fallible OpenAPI
generation fail on duplicate validation schema names instead of choosing
whichever registration inventory returns last. Manual validation metadata names
must be non-empty and trimmed, and custom messages/attribute labels must contain
visible text before the schema can be exported. Manual message and attribute
fields must match declared schema fields, and message rule names must match a
rule reachable from that field, including server-only rules and item rules
nested under `each(...)`. Client-checkable manual rules must include the params or values
their generated TypeScript runtime case needs, such as `min`, `max`, `value`,
`other`, `pattern`, or non-empty `values` for list/key rules, and they cannot
include extra params, values, nested rules, or schemas that their generated
runtime case ignores. `each(...)` needs at least one nested item rule, and
`nested` needs a child validation schema. Numeric params must be shaped like
generated metadata too: non-negative integers for length/count rules, finite
numbers for numeric comparisons, positive `multiple_of` values, `uuid` versions
from 1 through 8, and `min <= max` for range rules such as `decimal`,
`digits_between`, and `between`. Client-checkable `regex` / `not_regex`
patterns must also be browser-compatible after Foundry's Rust-to-JavaScript
translation; mark Rust-only regex metadata with `.server_only()`. Field-list
rules such as `required_with_all`,
`required_without_all`, and `prohibits` must keep `params.other` aligned with
their `values`; the matching `TsValidationRule` helper constructors fill both
from one field list. Cross-field `other` params and field-list values must be
non-empty, trimmed serde wire field names. Value-list rules such as
`starts_with`, `ends_with`, and `contains` must keep `params.value` equal to
`values.join(", ")` when both are present, so generated messages describe the
same values the runtime checks.
Manual schemas can use the Rust-DX helpers
`TsValidationRule::starts_with_value(...)`,
`doesnt_start_with_value(...)`, `ends_with_value(...)`,
`doesnt_end_with_value(...)`, `contains_value(...)`,
`doesnt_contain_value(...)`, `starts_with_any(...)`,
`doesnt_start_with_any(...)`, `ends_with_any(...)`,
`doesnt_end_with_any(...)`, and `size_numeric(...)`; they emit the same canonical
`starts_with`, `doesnt_start_with`, `ends_with`, `doesnt_end_with`, `contains`,
`doesnt_contain`, and `size` rule metadata that derive-generated validation
schemas export.

Request schemas containing `UploadedFile` are documented as
`multipart/form-data` in OpenAPI. Generated endpoint helpers read the generated
`requestMediaType` metadata and automatically send multipart request DTOs as
`FormData`; direct `UploadedFile` / `Vec<UploadedFile>` route request aliases
use a repeated `file` multipart part by default. OpenAPI documents those direct
file roots as a multipart object with a `file` property, matching the generated
helper default. `Option<UploadedFile>` and `Option<Vec<UploadedFile>>` direct
roots document that `file` property as optional. Pass `formDataField` to
`submitForm()` when a direct-file endpoint expects a different root field name.

Disable the route helper when a route should only export manifest metadata and
DTOs:

```rust
routes.post(
    foundry::RouteId::new("billing.webhook"),
    "/webhook",
    webhook,
    |route| {
        route.without_client_export();
    },
);

// Equivalent builder style:
HttpRouteOptions::new().client_export(false)
```

### `foundry::TS` (escape hatch)

For any type that isn't a DTO or AppEnum but needs TypeScript export:

```rust
#[derive(serde::Serialize, foundry::ts_rs::TS, foundry::TS)]
#[ts(crate = "foundry::ts_rs")]
pub struct SomeCustomType {
    pub name: String,
    pub value: f64,
}
```

Each generated artifact must own a unique TypeScript type name and `.ts` output
path. `types:export` fails before writing files if a `foundry::TS` /
`ApiSchema` registration uses an invalid/duplicate type name, reuses an AppEnum,
route-helper, or framework barrel export name, or uses a custom
`#[ts(export_to = "...")]` path already owned by another type, an AppEnum
export, a route helper, the generated barrel, or a Foundry runtime manifest file.

---

## ts_rs Attributes

Control TypeScript output with `#[ts(...)]` attributes:

```rust
#[derive(serde::Serialize, foundry::ts_rs::TS, foundry::ApiSchema)]
#[ts(crate = "foundry::ts_rs")]
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
- `#[ts(export_to = "...")]` — rare filename/subdirectory override inside `typescript.output_dir`

Use serde attributes for JSON wire names. `#[derive(ApiSchema)]` follows
`#[serde(rename = "...")]` and supported `#[serde(rename_all = "...")]` rules
when it writes OpenAPI schemas, so those names stay aligned with generated
TypeScript and backend request/response bodies. Public DTO fields must resolve
to unique JSON names; Foundry rejects duplicate serde wire names instead of
letting generated TypeScript, validation metadata, or OpenAPI property maps
disagree about which field owns that key. Plain contract enum variants must also
resolve to unique JSON names so generated unions and OpenAPI enum values stay
one-to-one with backend variants. `#[serde(alias = "...")]` is rejected on public
contract fields and enum variants because it makes the backend accept alternate
input names that generated TypeScript and OpenAPI cannot honestly advertise as
the canonical contract. Custom field codecs such as `#[serde(with = "...")]`,
`#[serde(serialize_with = "...")]`, and `#[serde(deserialize_with = "...")]`
are also rejected on public contract fields because generated TypeScript and
OpenAPI cannot infer the hidden wire shape. Fields marked `#[serde(flatten)]`
are emitted as parent-level OpenAPI properties, matching the flattened JSON
payload shape.
TypeScript-only renames such as `#[ts(rename = "...")]` and
`#[ts(rename_all = "...")]` are rejected on public contracts; use serde
`rename` / `rename_all` so JSON, OpenAPI, validation metadata, and TypeScript
share the same property names.
Flattened `Option<T>` fields are rejected because the TypeScript exporter cannot
represent them safely; flatten a normal child DTO and put `Option` on the child
fields that may be omitted. Flattened fields are
also rejected on `#[serde(deny_unknown_fields)]` DTOs because serde cannot safely
combine flattened fields with strict unknown-field rejection. Derived OpenAPI
schemas also fail fast when flattened child fields collide with parent fields or
with another flattened child after serde renaming; rename the fields or split the
DTO so every flattened property is unique. Flattened fields also cannot carry
`#[validate(...)]` rules yet because generated route-helper validation metadata
cannot safely target flattened JSON keys. For both
`ApiSchema` and `foundry::TS`, non-`Option` fields with `#[serde(default)]` must
either add `#[ts(optional, as = "Option<_>")]` so generated TypeScript also
allows omission, or add `#[validate(required)]` when a missing value should fail
request validation. Fields with `#[serde(skip_serializing_if = "...")]` must use
`#[ts(optional)]` because serde may omit the property from response JSON; derived
OpenAPI schemas also leave those sparse response fields out of `required`.
Directional skips, `#[serde(skip_serializing)]` and
`#[serde(skip_deserializing)]`, are rejected because one generated contract cannot
describe both request and response shapes safely. Split the request/response DTOs,
or use `#[serde(skip)]` / `#[ts(skip)]` when the field is never public.
Validation metadata from `#[derive(Validate)]` treats `#[serde(skip)]` and
`#[serde(skip_deserializing)]` as skipped request fields: they are omitted from
strict `knownFields`, ignored by generated multipart extraction, initialized from
serde-style defaults, and cannot carry `#[validate(...)]` rules.
`#[serde(deny_unknown_fields)]` is reflected in generated OpenAPI schemas as
`additionalProperties: false`, and generated route-helper validation metadata
sets `denyUnknownFields: true` with the full `knownFields` request shape for
DTOs that also derive `Validate`, so schema clients and form helpers see the
same extra-field rejection that serde applies.

Do not use `#[ts(export)]` on types deriving `foundry::ApiSchema` or
`foundry::TS`. Foundry registers those types automatically and `types:export`
writes them to the configured `typescript.output_dir`; direct ts-rs export
creates unmanaged files such as root `bindings/*.ts`.

---

## Framework Types

These types are auto-exported by the framework (no configuration needed):

| Type | Module | TypeScript |
|------|--------|------------|
| `Actor` | `foundry::auth` | authenticated actor id, guard, roles, permissions, claims |
| `AuthErrorCode` | `foundry::auth` | known auth `error_code` string union + helpers |
| `AuthOutcome` | `foundry::logging` | auth diagnostic outcome string union + helpers |
| `Attachment` | `foundry::attachments` | attachment record with storage metadata and `JsonValue` custom properties |
| `AuditLog` | `foundry::audit` | audit review record with `JsonValue` snapshots |
| `ClientAction` | `foundry::websocket` | WebSocket client action string union + helpers |
| `ClientMessage` | `foundry::websocket` | raw inbound WebSocket client frame |
| `Country` | `foundry::countries` | country record with typed currency, phone, TLD, and timezone metadata |
| `CountryCurrency` | `foundry::countries` | `{ code, name, symbol, minor_units }` |
| `CountryStatus` | `foundry::countries` | `"enabled" \| "disabled"` |
| `CsrfTokenResponse` | `foundry::http::response` | `{ token }` |
| `CursorInfo` | `foundry::database` | nullable cursor tokens for cursor-paginated responses |
| `CursorMeta` | `foundry::database` | `has_next`, `has_prev`, and `per_page` cursor metadata |
| `TokenPair` | `foundry::auth::token` | `{ access_token, refresh_token, ... }` |
| `RefreshTokenRequest` | `foundry::auth::token` | `{ refresh_token }` |
| `TokenResponse` | `foundry::auth::token` | `{ tokens: TokenPair }` |
| `WsTokenResponse` | `foundry::auth::token` | `{ token }` |
| `MfaCodeRequest` | `foundry::auth::mfa` | `{ code }` |
| `MfaEnrollChallenge` | `foundry::auth::mfa` | `{ secret, otpauth_url }` |
| `MfaRecoveryCodesRequest` | `foundry::auth::mfa` | `{ current_code }` |
| `MfaRecoveryCodesResponse` | `foundry::auth::mfa` | `{ recovery_codes }` |
| `ModelMeta` | `foundry::metadata` | metadata record with `value: JsonValue \| null` |
| `ModelTranslation` | `foundry::translations` | single translated field row |
| `TranslatedFields` | `foundry::translations` | locale map + resolved current-locale value |
| `MessageResponse` | `foundry::http::response` | `{ message }` |
| `DatatableRequest` | `foundry::datatable::request` | typed filters + sorts + pagination |
| `DatatableJsonResponse` | `foundry::datatable::response` | typed columns + filters + `JsonValue` rows + sorts |
| `DatatableExportAccepted` | `foundry::datatable::response` | queued datatable export response |
| `DatatableExportStatus` | `foundry::datatable::response` | `"queued"` export status string union + helpers |
| `HttpOutcomeClass` | `foundry::logging` | HTTP status-class diagnostic string union + helpers |
| `FailedJobResponse` | `foundry::logging` | row returned by `/_foundry/jobs/failed` |
| `JobsFailedResponse` | `foundry::logging` | failed/retried job list response |
| `JobsStatsResponse` | `foundry::logging` | grouped job history status counts |
| `JobStatusCountResponse` | `foundry::logging` | `{ status: JobHistoryStatus, count }` |
| `JobHistoryStatus` | `foundry::jobs` | `"succeeded" \| "retried" \| "dead_lettered"` |
| `JobOutcome` | `foundry::logging` | job diagnostic outcome string union + helpers |
| `LivenessReport` | `foundry::logging` | health liveness response |
| `LogLevel` | `foundry::logging` | logging level string union + helpers |
| `NotificationBroadcastPayload` | `foundry::notifications` | `{ notification_type, data }` |
| `Pagination` | `foundry::database` | optional `page` / `per_page` pagination request fields |
| `PaginationLinks` | `foundry::database` | nullable `next` / `prev` offset pagination links |
| `PaginationMeta` | `foundry::database` | offset pagination response metadata |
| `PresenceInfo` | `foundry::websocket` | presence member record returned by WebSocket presence helpers |
| `ProbeResult` | `foundry::logging` | readiness probe result |
| `ProbeState` | `foundry::logging` | health/readiness state string union + helpers |
| `ReadinessReport` | `foundry::logging` | readiness response with probe results |
| `RuntimeBackendKind` | `foundry::logging` | diagnostics backend string union + helpers |
| `RuntimeSnapshot` | `foundry::logging` | `/_foundry/runtime` diagnostic snapshot |
| `SchedulerLeadershipState` | `foundry::logging` | scheduler leadership diagnostic string union + helpers |
| `Setting` | `foundry::settings` | setting value + admin form metadata |
| `SettingType` | `foundry::settings` | `"text" \| "textarea" \| "number" \| ...` |
| `StoredFile` | `foundry::storage` | stored upload/write result |
| `StorageObject` | `foundry::storage` | prefix listing object metadata |
| `ServerMessage` | `foundry::websocket` | raw outbound WebSocket server frame |
| `WebSocketAckPayload` | `foundry::websocket` | `system/ack` protocol frame payload |
| `WebSocketAckStatus` | `foundry::websocket` | `"ok" \| "error"` ACK status union + helpers |
| `WebSocketChannelDescriptor` | `foundry::websocket` | registered channel metadata used by dashboards and manifests |
| `WebSocketChannelsResponse` | `foundry::logging` | `/_foundry/ws/channels` observability response |
| `WebSocketChannelStatsResponse` | `foundry::logging` | per-channel WebSocket observability counters |
| `WebSocketConnectionState` | `foundry::logging` | WebSocket connection diagnostic string union + helpers |
| `WebSocketGlobalStatsResponse` | `foundry::logging` | global WebSocket observability counters |
| `WebSocketHistoryMessageResponse` | `foundry::logging` | single `/_foundry/ws/history/{channel}` message row |
| `WebSocketHistoryResponse` | `foundry::logging` | WebSocket history observability response |
| `WebSocketPresenceJoinPayload` | `foundry::websocket` | `presence:join` protocol frame payload |
| `WebSocketPresenceLeavePayload` | `foundry::websocket` | `presence:leave` protocol frame payload |
| `WebSocketPresenceMemberResponse` | `foundry::logging` | single `/_foundry/ws/presence/{channel}` member row |
| `WebSocketPresenceResponse` | `foundry::logging` | WebSocket presence observability response |
| `WebSocketStatsResponse` | `foundry::logging` | `/_foundry/ws/stats` observability response |

Datatable exports keep JSON-facing numeric fields as `number`, include the supporting filter option imports needed by generated metadata files, and mark defaulted `DatatableRequest` fields (`page`, `per_page`, `sort`, `filters`, `search`) optional.

---

## Generated Output

```
frontend/shared/types/generated/
├── index.ts                    ← barrel (auto-generated)
├── FoundryEndpoint.ts          ← headless endpoint base runtime
├── AppManifest.ts              ← app name, environment, timezone, and shutdown metadata
├── AuthManifest.ts             ← auth guards, policies, and authenticatable metadata
├── AuditManifest.ts            ← audit event types and redaction policy metadata
├── LoggingManifest.ts          ← log level, format, directory, and retention helpers
├── RouteManifest.ts            ← route URL helpers and metadata
├── HttpManifest.ts             ← browser-safe HTTP config metadata
├── WebSocketChannelManifest.ts ← WebSocket channel helpers and metadata
├── EventManifest.ts            ← event ids, listener/payload selectors, and payload map
├── JobManifest.ts              ← job ids, queue/payload selectors, runtime policy, and optional payload map
├── CommandManifest.ts          ← CLI command ids, descriptions, argument metadata, option switch, flag, and subcommand selectors
├── ScheduleManifest.ts         ← schedule ids, timing, environment/hook selectors, and runtime policy
├── ValidationRuleManifest.ts   ← custom validation rule ids and server-only selectors
├── PluginManifest.ts           ← plugin ids, versions, assets, scaffolds, and contributions
├── StorageManifest.ts          ← storage disk ids, drivers, visibility, default disk, and upload/image caps
├── EmailManifest.ts            ← email mailer ids, drivers, default mailer, queue, and attachment caps
├── I18nManifest.ts             ← locale ids, default/fallback locale, map, and resolver
├── ReadinessManifest.ts        ← readiness probe ids and built-in/custom selectors
├── SettingManifest.ts          ← setting keys, form metadata, and value helpers
├── CacheManifest.ts            ← cache driver, policy, Redis namespace, and remember-lock helpers
├── DatabaseManifest.ts         ← pagination defaults without DB connection internals
├── ObservabilityManifest.ts    ← observability base path, retention, service, and capture metadata
├── DatatableManifest.ts        ← datatable ids, columns, filters, default sorts, and caps
├── NotificationManifest.ts     ← typed broadcast notification payload map, payload selectors, channel ids, and broadcast constants
├── routes/
│   └── UserPortalLogin.ts      ← route helper class + route DTO aliases
├── CreateOrderRequest.ts       ← from project
├── OrderStatus.ts              ← from project
├── Actor.ts                    ← from framework
├── Attachment.ts               ← from framework
├── AuditLog.ts                 ← from framework
├── AuthOutcome.ts              ← from framework
├── AuthErrorCode.ts            ← from framework
├── ClientAction.ts             ← from framework
├── ClientMessage.ts            ← from framework
├── Country.ts                  ← from framework
├── CountryCurrency.ts          ← from framework
├── CountryStatus.ts            ← from framework
├── CsrfTokenResponse.ts        ← from framework
├── CursorInfo.ts               ← from framework
├── CursorMeta.ts               ← from framework
├── DatatableExportAccepted.ts  ← from framework
├── DatatableExportStatus.ts    ← from framework
├── DatatableJsonResponse.ts    ← from framework
├── DatatableRequest.ts         ← from framework
├── ErrorResponse.ts            ← from framework
├── FailedJobResponse.ts        ← from framework
├── FieldError.ts               ← from framework
├── HttpOutcomeClass.ts         ← from framework
├── JobHistoryStatus.ts         ← from framework
├── JobOutcome.ts               ← from framework
├── JobsFailedResponse.ts       ← from framework
├── JobsStatsResponse.ts        ← from framework
├── JobStatusCountResponse.ts   ← from framework
├── LivenessReport.ts           ← from framework
├── LogLevel.ts                 ← from framework
├── MessageResponse.ts          ← from framework
├── ValidationErrorResponse.ts  ← from framework
├── MfaCodeRequest.ts           ← from framework
├── MfaEnrollChallenge.ts       ← from framework
├── MfaRecoveryCodesRequest.ts  ← from framework
├── MfaRecoveryCodesResponse.ts ← from framework
├── ModelMeta.ts                ← from framework
├── ModelTranslation.ts         ← from framework
├── Pagination.ts               ← from framework
├── PaginationLinks.ts          ← from framework
├── PaginationMeta.ts           ← from framework
├── PresenceInfo.ts             ← from framework
├── ProbeResult.ts              ← from framework
├── ProbeState.ts               ← from framework
├── RefreshTokenRequest.ts      ← from framework
├── ReadinessReport.ts          ← from framework
├── RuntimeBackendKind.ts       ← from framework
├── RuntimeSnapshot.ts          ← from framework
├── SchedulerLeadershipState.ts ← from framework
├── ServerMessage.ts            ← from framework
├── StoredFile.ts               ← from framework
├── StorageObject.ts            ← from framework
├── TokenPair.ts                ← from framework
├── TokenResponse.ts            ← from framework
├── TranslatedFields.ts         ← from framework
├── WebSocketAckPayload.ts      ← from framework
├── WebSocketAckStatus.ts       ← from framework
├── WebSocketChannelDescriptor.ts ← from framework
├── WebSocketChannelsResponse.ts ← from framework
├── WebSocketChannelStatsResponse.ts ← from framework
├── WebSocketConnectionState.ts ← from framework
├── WebSocketGlobalStatsResponse.ts ← from framework
├── WebSocketHistoryMessageResponse.ts ← from framework
├── WebSocketHistoryResponse.ts ← from framework
├── WebSocketPresenceJoinPayload.ts ← from framework
├── WebSocketPresenceLeavePayload.ts ← from framework
├── WebSocketPresenceMemberResponse.ts ← from framework
├── WebSocketPresenceResponse.ts ← from framework
├── WebSocketStatsResponse.ts   ← from framework
├── WsTokenResponse.ts          ← from framework
└── ...
```

The barrel `index.ts` re-exports generated types and helpers. A shortened
example looks like:

```typescript
// Auto-generated barrel. Do not edit.
export type { CreateOrderRequest } from "./CreateOrderRequest";
export { type AuthErrorCode, AuthErrorCodeValues, AuthErrorCodeOptions, AuthErrorCodeMeta, getAuthErrorCodeValues, getAuthErrorCodeValueCount, hasAuthErrorCodeValues, getAuthErrorCodeFirstValue, getAuthErrorCodeFirstValueOrNull, getAuthErrorCodeOptions, getAuthErrorCodeOptionCount, hasAuthErrorCodeOptions, getAuthErrorCodeFirstOption, getAuthErrorCodeFirstOptionOrNull, getAuthErrorCodeMeta, isAuthErrorCode, parseAuthErrorCodeOrNull, getAuthErrorCodeOption, getAuthErrorCodeLabelKey, AuthErrorCodeKeys, getAuthErrorCodeKeys, getAuthErrorCodeKeyNames, getAuthErrorCodeKeyCount, hasAuthErrorCodeKeys, getAuthErrorCodeFirstKeyName, getAuthErrorCodeFirstKeyNameOrNull, getAuthErrorCodeFirstKeyValue, getAuthErrorCodeFirstKeyValueOrNull } from "./AuthErrorCode";
export { type ClientAction, ClientActionValues, ClientActionOptions, ClientActionMeta, getClientActionValues, getClientActionValueCount, hasClientActionValues, getClientActionFirstValue, getClientActionFirstValueOrNull, getClientActionOptions, getClientActionOptionCount, hasClientActionOptions, getClientActionFirstOption, getClientActionFirstOptionOrNull, getClientActionMeta, isClientAction, parseClientActionOrNull, getClientActionOption, getClientActionLabelKey, ClientActionKeys, getClientActionKeys, getClientActionKeyNames, getClientActionKeyCount, hasClientActionKeys, getClientActionFirstKeyName, getClientActionFirstKeyNameOrNull, getClientActionFirstKeyValue, getClientActionFirstKeyValueOrNull } from "./ClientAction";
export type { ClientMessage } from "./ClientMessage";
export { type CountryStatus, CountryStatusValues, CountryStatusOptions, CountryStatusMeta, getCountryStatusValues, getCountryStatusValueCount, hasCountryStatusValues, getCountryStatusFirstValue, getCountryStatusFirstValueOrNull, getCountryStatusOptions, getCountryStatusOptionCount, hasCountryStatusOptions, getCountryStatusFirstOption, getCountryStatusFirstOptionOrNull, getCountryStatusMeta, isCountryStatus, parseCountryStatusOrNull, getCountryStatusOption, getCountryStatusLabelKey, CountryStatusKeys, getCountryStatusKeys, getCountryStatusKeyNames, getCountryStatusKeyCount, hasCountryStatusKeys, getCountryStatusFirstKeyName, getCountryStatusFirstKeyNameOrNull, getCountryStatusFirstKeyValue, getCountryStatusFirstKeyValueOrNull } from "./CountryStatus";
export type { CsrfTokenResponse } from "./CsrfTokenResponse";
export type { CursorInfo } from "./CursorInfo";
export type { CursorMeta } from "./CursorMeta";
export type { DatatableExportAccepted } from "./DatatableExportAccepted";
export { type DatatableExportStatus, DatatableExportStatusValues, DatatableExportStatusOptions, DatatableExportStatusMeta, getDatatableExportStatusValues, getDatatableExportStatusValueCount, hasDatatableExportStatusValues, getDatatableExportStatusFirstValue, getDatatableExportStatusFirstValueOrNull, getDatatableExportStatusOptions, getDatatableExportStatusOptionCount, hasDatatableExportStatusOptions, getDatatableExportStatusFirstOption, getDatatableExportStatusFirstOptionOrNull, getDatatableExportStatusMeta, isDatatableExportStatus, parseDatatableExportStatusOrNull, getDatatableExportStatusOption, getDatatableExportStatusLabelKey, DatatableExportStatusKeys, getDatatableExportStatusKeys, getDatatableExportStatusKeyNames, getDatatableExportStatusKeyCount, hasDatatableExportStatusKeys, getDatatableExportStatusFirstKeyName, getDatatableExportStatusFirstKeyNameOrNull, getDatatableExportStatusFirstKeyValue, getDatatableExportStatusFirstKeyValueOrNull } from "./DatatableExportStatus";
export type { DatatableJsonResponse } from "./DatatableJsonResponse";
export type { DatatableRequest } from "./DatatableRequest";
export type { ErrorResponse } from "./ErrorResponse";
export type { FailedJobResponse } from "./FailedJobResponse";
export type { FieldError } from "./FieldError";
export { type JobHistoryStatus, JobHistoryStatusValues, JobHistoryStatusOptions, JobHistoryStatusMeta, getJobHistoryStatusValues, getJobHistoryStatusValueCount, hasJobHistoryStatusValues, getJobHistoryStatusFirstValue, getJobHistoryStatusFirstValueOrNull, getJobHistoryStatusOptions, getJobHistoryStatusOptionCount, hasJobHistoryStatusOptions, getJobHistoryStatusFirstOption, getJobHistoryStatusFirstOptionOrNull, getJobHistoryStatusMeta, isJobHistoryStatus, parseJobHistoryStatusOrNull, getJobHistoryStatusOption, getJobHistoryStatusLabelKey, JobHistoryStatusKeys, getJobHistoryStatusKeys, getJobHistoryStatusKeyNames, getJobHistoryStatusKeyCount, hasJobHistoryStatusKeys, getJobHistoryStatusFirstKeyName, getJobHistoryStatusFirstKeyNameOrNull, getJobHistoryStatusFirstKeyValue, getJobHistoryStatusFirstKeyValueOrNull } from "./JobHistoryStatus";
export type { JobsFailedResponse } from "./JobsFailedResponse";
export type { JobsStatsResponse } from "./JobsStatsResponse";
export type { JobStatusCountResponse } from "./JobStatusCountResponse";
export type { MessageResponse } from "./MessageResponse";
export type { ValidationErrorResponse } from "./ValidationErrorResponse";
export type { Pagination } from "./Pagination";
export type { PaginationLinks } from "./PaginationLinks";
export type { PaginationMeta } from "./PaginationMeta";
export type { PresenceInfo } from "./PresenceInfo";
export { type OrderStatus, OrderStatusValues, OrderStatusOptions, OrderStatusMeta, getOrderStatusValues, getOrderStatusValueCount, hasOrderStatusValues, getOrderStatusFirstValue, getOrderStatusFirstValueOrNull, getOrderStatusOptions, getOrderStatusOptionCount, hasOrderStatusOptions, getOrderStatusFirstOption, getOrderStatusFirstOptionOrNull, getOrderStatusMeta, isOrderStatus, parseOrderStatusOrNull, getOrderStatusOption, getOrderStatusLabelKey, OrderStatusKeys, getOrderStatusKeys, getOrderStatusKeyNames, getOrderStatusKeyCount, hasOrderStatusKeys, getOrderStatusFirstKeyName, getOrderStatusFirstKeyNameOrNull, getOrderStatusFirstKeyValue, getOrderStatusFirstKeyValueOrNull } from "./OrderStatus";
export type { RefreshTokenRequest } from "./RefreshTokenRequest";
export type { TokenPair } from "./TokenPair";
export type { TokenResponse } from "./TokenResponse";
export type { ServerMessage } from "./ServerMessage";
export type { WebSocketAckPayload } from "./WebSocketAckPayload";
export { type WebSocketAckStatus, WebSocketAckStatusValues, WebSocketAckStatusOptions, WebSocketAckStatusMeta, getWebSocketAckStatusValues, getWebSocketAckStatusValueCount, hasWebSocketAckStatusValues, getWebSocketAckStatusFirstValue, getWebSocketAckStatusFirstValueOrNull, getWebSocketAckStatusOptions, getWebSocketAckStatusOptionCount, hasWebSocketAckStatusOptions, getWebSocketAckStatusFirstOption, getWebSocketAckStatusFirstOptionOrNull, getWebSocketAckStatusMeta, isWebSocketAckStatus, parseWebSocketAckStatusOrNull, getWebSocketAckStatusOption, getWebSocketAckStatusLabelKey, WebSocketAckStatusKeys, getWebSocketAckStatusKeys, getWebSocketAckStatusKeyNames, getWebSocketAckStatusKeyCount, hasWebSocketAckStatusKeys, getWebSocketAckStatusFirstKeyName, getWebSocketAckStatusFirstKeyNameOrNull, getWebSocketAckStatusFirstKeyValue, getWebSocketAckStatusFirstKeyValueOrNull } from "./WebSocketAckStatus";
export type { WebSocketChannelDescriptor } from "./WebSocketChannelDescriptor";
export type { WebSocketPresenceJoinPayload } from "./WebSocketPresenceJoinPayload";
export type { WebSocketPresenceLeavePayload } from "./WebSocketPresenceLeavePayload";
export type { WsTokenResponse } from "./WsTokenResponse";
export { DatatableManifest, DatatableRuntimeManifest, DatatableIds, DatatableMaxPerPage, DatatableMaxExportRows, datatableMaxPerPage, datatableMaxExportRows, isDatatableName, datatableNameOrNull, datatableManifestEntry, datatableManifestEntryOrNull, datatableEntries, datatableNames, datatableCount, datatableHasEntries, datatableFirstEntry, datatableFirstEntryOrNull, datatableFirstName, datatableFirstNameOrNull, datatableColumns, datatableColumn, datatableColumnCount, datatableTotalColumnCount, datatableHasColumns, datatableFirstColumn, datatableFirstColumnOrNull, datatableColumnNames, datatableColumnNameCount, datatableHasColumnNames, datatableFirstColumnName, datatableFirstColumnNameOrNull, datatableSortableColumns, datatableSortableColumn, datatableSortableColumnCount, datatableTotalSortableColumnCount, datatableHasSortableColumns, datatableFirstSortableColumn, datatableFirstSortableColumnOrNull, datatableSortableFieldNames, datatableSortableFieldNameCount, datatableHasSortableFieldNames, datatableFirstSortableFieldName, datatableFirstSortableFieldNameOrNull, datatableNonSortableColumns, datatableNonSortableColumn, datatableNonSortableColumnCount, datatableTotalNonSortableColumnCount, datatableHasNonSortableColumns, datatableFirstNonSortableColumn, datatableFirstNonSortableColumnOrNull, datatableNonSortableColumnNames, datatableNonSortableColumnNameCount, datatableHasNonSortableColumnNames, datatableFirstNonSortableColumnName, datatableFirstNonSortableColumnNameOrNull, datatableFilterableColumns, datatableFilterableColumn, datatableFilterableColumnCount, datatableTotalFilterableColumnCount, datatableHasFilterableColumns, datatableFirstFilterableColumn, datatableFirstFilterableColumnOrNull, datatableFilterableColumnNames, datatableFilterableColumnNameCount, datatableHasFilterableColumnNames, datatableFirstFilterableColumnName, datatableFirstFilterableColumnNameOrNull, datatableNonFilterableColumns, datatableNonFilterableColumn, datatableNonFilterableColumnCount, datatableTotalNonFilterableColumnCount, datatableHasNonFilterableColumns, datatableFirstNonFilterableColumn, datatableFirstNonFilterableColumnOrNull, datatableNonFilterableColumnNames, datatableNonFilterableColumnNameCount, datatableHasNonFilterableColumnNames, datatableFirstNonFilterableColumnName, datatableFirstNonFilterableColumnNameOrNull, datatableExportableColumns, datatableExportableColumn, datatableExportableColumnCount, datatableTotalExportableColumnCount, datatableHasExportableColumns, datatableFirstExportableColumn, datatableFirstExportableColumnOrNull, datatableExportableColumnNames, datatableExportableColumnNameCount, datatableHasExportableColumnNames, datatableFirstExportableColumnName, datatableFirstExportableColumnNameOrNull, datatableNonExportableColumns, datatableNonExportableColumn, datatableNonExportableColumnCount, datatableTotalNonExportableColumnCount, datatableHasNonExportableColumns, datatableFirstNonExportableColumn, datatableFirstNonExportableColumnOrNull, datatableNonExportableColumnNames, datatableNonExportableColumnNameCount, datatableHasNonExportableColumnNames, datatableFirstNonExportableColumnName, datatableFirstNonExportableColumnNameOrNull, datatableRelationColumns, datatableRelationColumn, datatableRelationColumnCount, datatableTotalRelationColumnCount, datatableHasRelationColumns, datatableFirstRelationColumn, datatableFirstRelationColumnOrNull, datatableRelationColumnNames, datatableRelationColumnNameCount, datatableHasRelationColumnNames, datatableFirstRelationColumnName, datatableFirstRelationColumnNameOrNull, datatableNonRelationColumns, datatableNonRelationColumn, datatableNonRelationColumnCount, datatableTotalNonRelationColumnCount, datatableHasNonRelationColumns, datatableFirstNonRelationColumn, datatableFirstNonRelationColumnOrNull, datatableNonRelationColumnNames, datatableNonRelationColumnNameCount, datatableHasNonRelationColumnNames, datatableFirstNonRelationColumnName, datatableFirstNonRelationColumnNameOrNull, datatableColumnRelationNames, datatableColumnRelationNameCount, datatableTotalColumnRelationNameCount, datatableHasColumnRelationNames, datatableFirstColumnRelationName, datatableFirstColumnRelationNameOrNull, datatableColumnsForRelation, datatableColumnNamesForRelation, datatableColumnCountForRelation, datatableHasColumnsForRelation, datatableFirstColumnForRelation, datatableFirstColumnForRelationOrNull, datatableFirstColumnNameForRelation, datatableFirstColumnNameForRelationOrNull, datatableMappings, datatableMappingCount, datatableTotalMappingCount, datatableHasMappings, datatableMappingNames, datatableMappingNameCount, datatableHasMappingNames, datatableFirstMappingName, datatableFirstMappingNameOrNull, datatableRelationFilters, datatableRelationFilterForField, datatableRelationFilterCanonicalField, datatableRelationFilterCount, datatableTotalRelationFilterCount, datatableHasRelationFilters, datatableFirstRelationFilter, datatableFirstRelationFilterOrNull, datatableRelationFilterRelationNames, datatableRelationFilterRelationNameCount, datatableTotalRelationFilterRelationNameCount, datatableHasRelationFilterRelationNames, datatableFirstRelationFilterRelationName, datatableFirstRelationFilterRelationNameOrNull, datatableRelationFiltersForRelation, datatableRelationFilterForFieldForRelation, datatableRelationFilterCanonicalFieldForRelation, datatableRelationFilterFieldsForRelation, datatableRelationFilterAliasesForRelation, datatableRelationFilterFieldNamesForRelation, datatableRelationFilterFieldNameCountForRelation, datatableHasRelationFilterFieldNamesForRelation, datatableFirstRelationFilterFieldNameForRelation, datatableFirstRelationFilterFieldNameForRelationOrNull, datatableRelationFilterAliasCountForRelation, datatableHasRelationFilterAliasesForRelation, datatableFirstRelationFilterAliasForRelation, datatableFirstRelationFilterAliasForRelationOrNull, datatableRelationFilterCountForRelation, datatableHasRelationFiltersForRelation, datatableFirstRelationFilterForRelation, datatableFirstRelationFilterForRelationOrNull, datatableFirstRelationFilterFieldForRelation, datatableFirstRelationFilterFieldForRelationOrNull, datatableRelationFilterFields, datatableRelationFilterFieldCount, datatableHasRelationFilterFields, datatableFirstRelationFilterField, datatableFirstRelationFilterFieldOrNull, datatableRelationFilterAliases, datatableRelationFilterFieldNames, datatableRelationFilterFieldNameCount, datatableTotalRelationFilterFieldNameCount, datatableHasRelationFilterFieldNames, datatableFirstRelationFilterFieldName, datatableFirstRelationFilterFieldNameOrNull, datatableRelationFilterAliasCount, datatableTotalRelationFilterAliasCount, datatableHasRelationFilterAliases, datatableFirstRelationFilterAlias, datatableFirstRelationFilterAliasOrNull, datatableStaticFilterFieldNames, datatableStaticFilterFieldCount, datatableTotalStaticFilterFieldCount, datatableHasStaticFilterFields, datatableFirstStaticFilterFieldName, datatableFirstStaticFilterFieldNameOrNull, isDatatableSortableField, isDatatableColumnName, isDatatableFilterableColumnName, isDatatableNonSortableColumnName, isDatatableNonFilterableColumnName, isDatatableExportableColumnName, isDatatableNonExportableColumnName, isDatatableRelationColumnName, isDatatableNonRelationColumnName, isDatatableColumnRelationName, isDatatableColumnNameForRelation, isDatatableRelationFilterRelationName, isDatatableRelationFilterCanonicalField, isDatatableRelationFilterAlias, isDatatableRelationFilterField, isDatatableRelationFilterCanonicalFieldForRelation, isDatatableRelationFilterAliasForRelation, isDatatableRelationFilterFieldForRelation, isDatatableStaticFilterField, isDatatableMappingName, isDatatableDefaultSortField, datatablePerPageCap, datatableExportRowsCap, datatableDefaultSort, datatableDefaultSortForField, datatableDefaultSortDirection, datatableDefaultSortCount, datatableTotalDefaultSortCount, datatableHasDefaultSort, datatableFirstDefaultSort, datatableFirstDefaultSortOrNull, datatableDefaultSortFieldNames, datatableDefaultSortFieldNameCount, datatableHasDefaultSortFieldNames, datatableFirstDefaultSortFieldName, datatableFirstDefaultSortFieldNameOrNull, datatableSort, datatableFilter, datatableRequest, datatableQueryParams, datatableRequestFromQueryParams, type DatatableManifestEntry, type DatatableName, type DatatableRuntimeManifestShape, type DatatableColumnName, type DatatableSortableFieldName, type DatatableNonSortableColumnName, type DatatableFilterableColumnName, type DatatableNonFilterableColumnName, type DatatableExportableColumnName, type DatatableNonExportableColumnName, type DatatableRelationColumnName, type DatatableNonRelationColumnName, type DatatableColumnRelationName, type DatatableColumnForRelation, type DatatableColumnNameForRelation, type DatatableMappingName, type DatatableDefaultSortFieldName, type DatatableRelationFilterCanonicalFieldName, type DatatableRelationFilterRelationName, type DatatableRelationFilterForRelation, type DatatableRelationFilterCanonicalFieldNameForRelation, type DatatableRelationFilterAliasNameForRelation, type DatatableRelationFilterFieldNameForRelation, type DatatableRelationFilterFieldName, type DatatableRelationFilterAliasName, type DatatableStaticFilterFieldName, type DatatableSortInputFor, type DatatableDefaultSortInputFor, type DatatableStaticFilterInputFor, type DatatableRequestFor, type DatatableQueryParams, type DatatableQueryParamRecord, type DatatableQueryParamSource, type DatatableQueryParamValue, type DatatableQueryParseOptions, type DatatableRelationFilterManifestEntry } from "./DatatableManifest";
export { CommandManifest, CommandIds, commandArguments, commandArgumentMetadata, commandArgumentMetadataForArgumentOrNull, commandArgumentHelp, commandArgumentValueNames, commandArgumentDefaultValues, commandArgumentPossibleValues, commandVisibleArgumentPossibleValueNames, commandRequiredArgumentNames, commandRepeatableArgumentNames, commandDefaultedArgumentNames, commandArgumentNamesWithPossibleValues, commandPositionalArguments, commandOptions, commandOptionSwitches, commandOptionTokens, commandPreferredOptionTokenOrNull, commandValueOptions, commandFlags, commandSubcommands, commandsWithOption, commandsWithValueOption, commandsWithFlag, commandsWithPositionalArguments, type CommandName, type CommandArgumentName, type CommandArgumentKind, type CommandArgumentMetadataEntry, type CommandArgumentMetadataFor, type CommandArgumentPossibleValueEntry, type CommandPositionalArgumentName, type CommandOptionName, type CommandOptionSwitchEntry, type CommandOptionToken, type CommandValueOptionName, type CommandFlagName } from "./CommandManifest";
export { EventManifest, EventIds, isEventName, isEventPayloadName, eventNameOrNull, eventPayloadNameOrNull, eventManifestEntry, eventManifestEntryOrNull, eventNames, eventEntries, eventCount, eventHasEntries, eventFirstEntry, eventFirstEntryOrNull, eventFirstName, eventFirstNameOrNull, eventListenerCount, eventTotalListenerCount, eventPayloadName, eventPayloadNames, eventPayloadNameCount, eventHasPayloadNames, eventFirstPayloadName, eventFirstPayloadNameOrNull, eventHasPayload, eventNamesWithPayload, eventUsesPayload, eventsWithPayload, eventCountWithPayload, eventHasEventsWithPayload, firstEventWithPayload, firstEventWithPayloadOrNull, firstEventNameWithPayload, firstEventNameWithPayloadOrNull, eventsWithoutPayload, eventNamesWithoutPayload, eventCountWithoutPayload, eventHasEventsWithoutPayload, firstEventWithoutPayload, firstEventWithoutPayloadOrNull, firstEventNameWithoutPayload, firstEventNameWithoutPayloadOrNull, eventNamesWithListeners, eventCountWithListeners, eventHasEventsWithListeners, firstEventWithListeners, firstEventWithListenersOrNull, firstEventNameWithListeners, firstEventNameWithListenersOrNull, eventNamesWithoutListeners, eventCountWithoutListeners, eventHasEventsWithoutListeners, firstEventWithoutListeners, firstEventWithoutListenersOrNull, firstEventNameWithoutListeners, firstEventNameWithoutListenersOrNull, eventsWithListeners, eventsWithoutListeners, type EventManifestEntry, type EventName, type EventPayloadName, type EventPayloadMap, type EventPayload } from "./EventManifest";
export { JobManifest, JobRuntimeManifest, JobIds, JobDefaultQueue, JobMaxRetries, jobDefaultQueueName, jobMaxRetries, jobMaxConcurrentJobs, jobTimeoutSeconds, jobHistoryRetentionDays, isJobName, isJobQueueName, isJobPayloadName, jobNameOrNull, jobQueueNameOrNull, jobPayloadNameOrNull, jobManifestEntry, jobManifestEntryOrNull, jobNames, jobEntries, jobCount, jobHasEntries, jobFirstEntry, jobFirstEntryOrNull, jobFirstName, jobFirstNameOrNull, jobQueue, jobQueueNames, jobQueueNameCount, jobHasQueueNames, jobFirstQueueName, jobFirstQueueNameOrNull, jobNamesInQueue, jobUsesQueue, jobsInQueue, jobCountInQueue, jobHasJobsInQueue, firstJobInQueue, firstJobInQueueOrNull, firstJobNameInQueue, firstJobNameInQueueOrNull, jobPriority, jobsWithQueuePriority, jobCountWithQueuePriority, jobHasJobsWithQueuePriority, firstJobWithQueuePriority, firstJobWithQueuePriorityOrNull, firstJobNameWithQueuePriority, firstJobNameWithQueuePriorityOrNull, jobPriorityQueueNames, jobPriorityQueueNameCount, jobHasPriorityQueueNames, jobFirstPriorityQueueName, jobFirstPriorityQueueNameOrNull, jobPayloadName, jobPayloadNames, jobPayloadNameCount, jobHasPayloadNames, jobFirstPayloadName, jobFirstPayloadNameOrNull, jobHasPayload, jobNamesWithPayload, jobUsesPayload, jobsWithPayload, jobCountWithPayload, jobHasJobsWithPayload, firstJobWithPayload, firstJobWithPayloadOrNull, firstJobNameWithPayload, firstJobNameWithPayloadOrNull, jobsWithoutPayload, jobNamesWithoutPayload, jobCountWithoutPayload, jobHasJobsWithoutPayload, firstJobWithoutPayload, firstJobWithoutPayloadOrNull, firstJobNameWithoutPayload, firstJobNameWithoutPayloadOrNull, jobQueuePriority, jobHistoryTracked, type JobHistoryManifestShape, type JobManifestEntry, type JobName, type JobPayloadName, type JobPayloadMap, type JobPayload, type JobQueueName, type JobRuntimeManifestShape } from "./JobManifest";
export { ScheduleManifest, SchedulerRuntimeManifest, ScheduleIds, SchedulerTickIntervalMs, SchedulerLeaderLeaseTtlMs, SchedulerShutdownTimeoutMs, schedulerTickIntervalMs, schedulerLeaderLeaseTtlMs, schedulerShutdownTimeoutMs, isScheduleName, isScheduleEnvironmentName, isScheduleHookName, scheduleNameOrNull, scheduleEnvironmentNameOrNull, scheduleHookNameOrNull, scheduleManifestEntry, scheduleManifestEntryOrNull, scheduleNames, scheduleEntries, scheduleCount, scheduleHasEntries, scheduleFirstEntry, scheduleFirstEntryOrNull, scheduleFirstName, scheduleFirstNameOrNull, scheduleKind, scheduleCronExpression, scheduleIntervalMilliseconds, cronScheduleNames, cronSchedules, cronScheduleCount, cronScheduleHasEntries, cronScheduleFirstEntry, cronScheduleFirstEntryOrNull, cronScheduleFirstName, cronScheduleFirstNameOrNull, intervalScheduleNames, intervalSchedules, intervalScheduleCount, intervalScheduleHasEntries, intervalScheduleFirstEntry, intervalScheduleFirstEntryOrNull, intervalScheduleFirstName, intervalScheduleFirstNameOrNull, scheduleEnvironments, scheduleEnvironmentNames, scheduleEnvironmentNameCount, scheduleTotalEnvironmentFilterCount, scheduleHasEnvironmentNames, scheduleFirstEnvironmentName, scheduleFirstEnvironmentNameOrNull, scheduleHasEnvironment, scheduleNamesForEnvironment, schedulesForEnvironment, scheduleCountForEnvironment, scheduleHasSchedulesForEnvironment, firstScheduleForEnvironment, firstScheduleForEnvironmentOrNull, firstScheduleNameForEnvironment, firstScheduleNameForEnvironmentOrNull, scheduleNamesWithoutEnvironment, schedulesWithoutEnvironment, scheduleCountWithoutEnvironment, scheduleHasSchedulesWithoutEnvironment, firstScheduleWithoutEnvironment, firstScheduleWithoutEnvironmentOrNull, firstScheduleNameWithoutEnvironment, firstScheduleNameWithoutEnvironmentOrNull, scheduleNamesWithoutEnvironmentFilters, schedulesWithoutEnvironmentFilters, scheduleCountWithoutEnvironmentFilters, scheduleHasSchedulesWithoutEnvironmentFilters, firstScheduleWithoutEnvironmentFilters, firstScheduleWithoutEnvironmentFiltersOrNull, firstScheduleNameWithoutEnvironmentFilters, firstScheduleNameWithoutEnvironmentFiltersOrNull, scheduleUsesOverlapLock, scheduleNamesWithoutOverlapping, schedulesWithoutOverlapping, scheduleCountWithoutOverlapping, scheduleHasSchedulesWithoutOverlapping, firstScheduleWithoutOverlapping, firstScheduleWithoutOverlappingOrNull, firstScheduleNameWithoutOverlapping, firstScheduleNameWithoutOverlappingOrNull, scheduleHookNames, scheduleHookNameCount, scheduleTotalEnabledHookCount, scheduleHasHookNames, scheduleFirstHookName, scheduleFirstHookNameOrNull, scheduleHasHook, scheduleNamesWithHook, schedulesWithHook, scheduleCountWithHook, scheduleHasSchedulesWithHook, firstScheduleWithHook, firstScheduleWithHookOrNull, firstScheduleNameWithHook, firstScheduleNameWithHookOrNull, scheduleNamesWithoutHook, schedulesWithoutHook, scheduleCountWithoutHook, scheduleHasSchedulesWithoutHook, firstScheduleWithoutHook, firstScheduleWithoutHookOrNull, firstScheduleNameWithoutHook, firstScheduleNameWithoutHookOrNull, scheduleNamesWithoutHooks, schedulesWithoutHooks, scheduleCountWithoutHooks, scheduleHasSchedulesWithoutHooks, firstScheduleWithoutHooks, firstScheduleWithoutHooksOrNull, firstScheduleNameWithoutHooks, firstScheduleNameWithoutHooksOrNull, type CronScheduleName, type IntervalScheduleName, type ScheduleEnvironmentName, type ScheduleEnvironmentNameFor, type ScheduleHookName, type ScheduleCronManifestEntry, type ScheduleIntervalManifestEntry, type ScheduleManifestEntry, type ScheduleName, type SchedulerRuntimeManifestShape } from "./ScheduleManifest";
export { ValidationRuleManifest, ValidationRuleIds, isValidationRuleId, isValidationRuleName, validationRuleIdOrNull, validationRuleNameOrNull, validationRuleManifestEntry, validationRuleManifestEntryOrNull, validationRuleManifestEntryById, validationRuleManifestEntryByIdOrNull, validationRuleId, validationRuleIdForNameOrNull, validationRuleIsRegistered, validationRuleIdIsServerOnly, validationRuleIds, validationRuleIdCount, validationRuleHasIds, validationRuleFirstId, validationRuleFirstIdOrNull, validationRuleNames, validationRuleNameCount, validationRuleHasNames, validationRuleEntries, validationRuleCount, validationRuleHasEntries, validationRuleFirstEntry, validationRuleFirstEntryOrNull, validationRuleFirstName, validationRuleFirstNameOrNull, validationRuleIsServerOnly, validationRuleIsServerOnlyOrNull, serverOnlyValidationRuleIds, serverOnlyValidationRuleIdCount, serverOnlyValidationRuleHasIds, serverOnlyValidationRuleFirstId, serverOnlyValidationRuleFirstIdOrNull, serverOnlyValidationRuleNames, serverOnlyValidationRuleNameCount, serverOnlyValidationRuleHasNames, serverOnlyValidationRuleFirstName, serverOnlyValidationRuleFirstNameOrNull, serverOnlyValidationRules, serverOnlyValidationRuleCount, serverOnlyValidationRuleHasEntries, serverOnlyValidationRuleFirstEntry, serverOnlyValidationRuleFirstEntryOrNull, type ValidationRuleId, type ValidationRuleManifestEntry, type ValidationRuleName } from "./ValidationRuleManifest";
export { PluginManifest, PluginIds, isPluginName, isPluginAssetKind, isPluginContributionName, pluginNameOrNull, pluginAssetKindOrNull, pluginContributionNameOrNull, pluginManifestEntry, pluginManifestEntryOrNull, pluginNames, pluginEntries, pluginCount, pluginHasEntries, pluginFirstEntry, pluginFirstEntryOrNull, pluginFirstName, pluginFirstNameOrNull, pluginDependencies, pluginDependencyCount, pluginTotalDependencyCount, pluginHasDependencies, pluginFirstDependency, pluginFirstDependencyOrNull, pluginFirstDependencyId, pluginFirstDependencyIdOrNull, pluginNamesWithDependencies, pluginsWithDependencies, pluginCountWithDependencies, pluginHasPluginsWithDependencies, pluginFirstPluginWithDependencies, pluginFirstPluginWithDependenciesOrNull, pluginFirstPluginNameWithDependencies, pluginFirstPluginNameWithDependenciesOrNull, pluginNamesWithoutDependencies, pluginsWithoutDependencies, pluginCountWithoutDependencies, pluginHasPluginsWithoutDependencies, pluginFirstPluginWithoutDependencies, pluginFirstPluginWithoutDependenciesOrNull, pluginFirstPluginNameWithoutDependencies, pluginFirstPluginNameWithoutDependenciesOrNull, pluginAssetKinds, pluginAssetKindCount, pluginHasAssetKinds, pluginFirstAssetKind, pluginFirstAssetKindOrNull, pluginAssets, pluginAssetCount, pluginTotalAssetCount, pluginHasAssets, pluginFirstAsset, pluginFirstAssetOrNull, pluginFirstAssetId, pluginFirstAssetIdOrNull, pluginNamesWithAssets, pluginsWithAssets, pluginCountWithAssets, pluginHasPluginsWithAssets, pluginFirstPluginWithAssets, pluginFirstPluginWithAssetsOrNull, pluginFirstPluginNameWithAssets, pluginFirstPluginNameWithAssetsOrNull, pluginNamesWithoutAssets, pluginsWithoutAssets, pluginCountWithoutAssets, pluginHasPluginsWithoutAssets, pluginFirstPluginWithoutAssets, pluginFirstPluginWithoutAssetsOrNull, pluginFirstPluginNameWithoutAssets, pluginFirstPluginNameWithoutAssetsOrNull, pluginScaffolds, pluginScaffoldCount, pluginTotalScaffoldCount, pluginHasScaffolds, pluginFirstScaffold, pluginFirstScaffoldOrNull, pluginFirstScaffoldId, pluginFirstScaffoldIdOrNull, pluginNamesWithScaffolds, pluginsWithScaffolds, pluginCountWithScaffolds, pluginHasPluginsWithScaffolds, pluginFirstPluginWithScaffolds, pluginFirstPluginWithScaffoldsOrNull, pluginFirstPluginNameWithScaffolds, pluginFirstPluginNameWithScaffoldsOrNull, pluginNamesWithoutScaffolds, pluginsWithoutScaffolds, pluginCountWithoutScaffolds, pluginHasPluginsWithoutScaffolds, pluginFirstPluginWithoutScaffolds, pluginFirstPluginWithoutScaffoldsOrNull, pluginFirstPluginNameWithoutScaffolds, pluginFirstPluginNameWithoutScaffoldsOrNull, pluginContributions, pluginContributionCount, pluginTotalContributionCount, pluginHasContribution, pluginContributionNames, pluginContributionNameCount, pluginHasContributionNames, pluginFirstContributionName, pluginFirstContributionNameOrNull, pluginNamesWithContribution, pluginsWithContribution, pluginCountWithContribution, pluginHasPluginsWithContribution, pluginFirstPluginWithContribution, pluginFirstPluginWithContributionOrNull, pluginFirstPluginNameWithContribution, pluginFirstPluginNameWithContributionOrNull, pluginNamesWithoutContribution, pluginsWithoutContribution, pluginCountWithoutContribution, pluginHasPluginsWithoutContribution, pluginFirstPluginWithoutContribution, pluginFirstPluginWithoutContributionOrNull, pluginFirstPluginNameWithoutContribution, pluginFirstPluginNameWithoutContributionOrNull, type PluginAssetKind, type PluginAssetManifestEntry, type PluginContributionName, type PluginContributionManifestEntry, type PluginDependencyManifestEntry, type PluginManifestEntry, type PluginName, type PluginScaffoldManifestEntry, type PluginScaffoldVariableManifestEntry } from "./PluginManifest";
export { AppManifest, ApplicationName, ApplicationEnvironment, ApplicationEnvironmentKinds, ApplicationEnvironmentKind, ApplicationTimezone, ApplicationBackgroundShutdownTimeoutMs, isAppEnvironmentKind, appEnvironmentKindOrNull, appEnvironmentKinds, appEnvironmentKindCount, appHasEnvironmentKinds, appFirstEnvironmentKind, appFirstEnvironmentKindOrNull, appName, appEnvironment, appEnvironmentIs, appEnvironmentKind, appEnvironmentKindIs, appTimezone, appBackgroundShutdownTimeoutMs, appBackgroundShutdownImmediate, appIsCustom, appIsDevelopment, appIsProduction, appIsProductionLike, appIsStaging, appIsTesting, type AppEnvironmentKind, type AppManifestShape } from "./AppManifest";
export { AuthManifest, AuthGuardManifest, AuthPolicyManifest, AuthRuntimeManifest, AuthGuardKinds, AuthGuardIds, AuthPolicyIds, DefaultAuthGuard, ConfiguredDefaultAuthGuard, AuthSessionCookieName, AuthMfaPendingTokenTtlMinutes, authBearerPrefix, authDefaultGuardName, authConfiguredDefaultGuardName, authTokenConfig, authTokenGuardManifestEntry, authTokenGuardNames, authTokenGuardEntries, authTokenGuardCount, authHasTokenGuards, authFirstTokenGuard, authFirstTokenGuardOrNull, authFirstTokenGuardName, authFirstTokenGuardNameOrNull, authAccessTokenTtlMinutes, authRefreshTokenTtlDays, authRotateRefreshTokens, authSessionConfig, authSessionTtlMinutes, authSessionCookieName, authSessionCookieSecure, authSessionCookiePath, authSessionCookieSameSite, authSessionCookieDomainConfigured, authSessionSlidingExpiry, authSessionRememberTtlDays, authLockoutConfig, authLockoutEnabled, authLockoutMaxFailures, authLockoutMinutes, authLockoutWindowMinutes, authMfaConfig, authMfaEnabled, authMfaIssuer, authMfaPendingTokenTtlMinutes, authMfaRecoveryCodes, authMfaRequiredRoles, authMfaGuardsRequiringRoles, authMfaGuardCountRequiringRoles, authMfaTotalRequiredRoleCount, authHasMfaGuardsRequiringRoles, authFirstMfaGuardRequiringRoles, authFirstMfaGuardRequiringRolesOrNull, authMfaRequiredRolesForGuard, authMfaRequiredRoleCountForGuard, authMfaFirstRequiredRoleForGuard, authMfaFirstRequiredRoleForGuardOrNull, authMfaGuardRequiresRole, authMfaGuardRequiresRoles, authPasswordResetExpiryMinutes, authEmailVerificationExpiryMinutes, isAuthGuardKind, isAuthGuardName, isAuthPolicyName, isAuthTokenGuardName, authGuardKindOrNull, authGuardNameOrNull, authPolicyNameOrNull, authTokenGuardNameOrNull, authGuardKinds, authGuardNames, authPolicyNames, authPolicies, authGuardKindCount, authHasGuardKinds, authFirstGuardKind, authFirstGuardKindOrNull, authGuardCount, authHasGuards, authFirstGuard, authFirstGuardOrNull, authFirstGuardName, authFirstGuardNameOrNull, authPolicyCount, authHasPolicies, authFirstPolicy, authFirstPolicyOrNull, authFirstPolicyName, authFirstPolicyNameOrNull, authGuardManifestEntry, authGuardManifestEntryOrNull, authPolicyManifestEntry, authPolicyManifestEntryOrNull, authGuards, authDefaultGuardManifestEntry, authHasDefaultGuard, authNonDefaultGuardNames, authNonDefaultGuards, authNonDefaultGuardCount, authHasNonDefaultGuards, authFirstNonDefaultGuard, authFirstNonDefaultGuardOrNull, authFirstNonDefaultGuardName, authFirstNonDefaultGuardNameOrNull, authGuardKind, authGuardIsDefault, authGuardHasAuthenticatable, authGuardNamesByKind, authGuardsByKind, authGuardCountByKind, authHasGuardsByKind, authFirstGuardByKind, authFirstGuardByKindOrNull, authFirstGuardNameByKind, authFirstGuardNameByKindOrNull, authGuardNamesWithoutKind, authGuardsWithoutKind, authGuardCountWithoutKind, authHasGuardsWithoutKind, authFirstGuardWithoutKind, authFirstGuardWithoutKindOrNull, authFirstGuardNameWithoutKind, authFirstGuardNameWithoutKindOrNull, authAuthenticatableGuardNames, authAuthenticatableGuards, authAuthenticatableGuardCount, authHasAuthenticatableGuards, authFirstAuthenticatableGuard, authFirstAuthenticatableGuardOrNull, authFirstAuthenticatableGuardName, authFirstAuthenticatableGuardNameOrNull, authNonAuthenticatableGuardNames, authNonAuthenticatableGuards, authNonAuthenticatableGuardCount, authHasNonAuthenticatableGuards, authFirstNonAuthenticatableGuard, authFirstNonAuthenticatableGuardOrNull, authFirstNonAuthenticatableGuardName, authFirstNonAuthenticatableGuardNameOrNull, type AuthAuthenticatableGuardName, type AuthNonAuthenticatableGuardName, type AuthGuardKind, type AuthGuardManifestEntry, type AuthGuardName, type AuthLockoutManifestShape, type AuthMfaManifestShape, type AuthPolicyManifestEntry, type AuthPolicyName, type AuthRuntimeManifestShape, type AuthSessionManifestShape, type AuthTokenGuardManifestEntry, type AuthTokenGuardName, type AuthTokenManifestShape } from "./AuthManifest";
export { AuditManifest, AuditEventTypes, AuditRedactedValue, AuditSensitiveFields, AuditSensitiveFieldSegments, auditRedactsSensitiveFields, auditSensitiveFieldRedactionDisabled, auditEventTypes, auditEventTypeOrNull, auditEventTypeCount, auditHasEventTypes, auditFirstEventType, auditFirstEventTypeOrNull, auditRedactedValue, auditFieldIsSensitive, auditFieldIsConfiguredSensitive, auditFieldMatchesSensitiveSegment, auditSensitiveFields, auditSensitiveFieldCount, auditHasSensitiveFields, auditFirstSensitiveField, auditFirstSensitiveFieldOrNull, auditSensitiveFieldSegments, auditSensitiveFieldSegmentCount, auditHasSensitiveFieldSegments, auditFirstSensitiveFieldSegment, auditFirstSensitiveFieldSegmentOrNull, isAuditEventType, isAuditSensitiveFieldName, isAuditSensitiveFieldSegment, auditSensitiveFieldNameOrNull, auditSensitiveFieldSegmentOrNull, normalizeAuditFieldName, type AuditEventType, type AuditSensitiveFieldName, type AuditSensitiveFieldSegment, type AuditManifestShape } from "./AuditManifest";
export { LoggingManifest, LoggingFormats, LoggingLogDir, LoggingRetentionDays, isLoggingFormat, loggingFormatOrNull, loggingFormats, loggingFormatCount, loggingHasFormats, loggingFirstFormat, loggingFirstFormatOrNull, loggingLevel, loggingFormat, loggingUsesJson, loggingUsesText, loggingLogDirectory, loggingHasLogDirectory, loggingWritesFiles, loggingFileOutputDisabled, loggingRetentionDays, loggingRetentionEnabled, loggingRetentionDisabled, type LoggingLevel, type LoggingFormat, type LoggingManifestShape } from "./LoggingManifest";
export { StorageManifest, StorageRuntimeManifest, StorageDiskIds, DefaultStorageDisk, ConfiguredDefaultStorageDisk, StorageMaxUploadSizeBytes, StorageMaxUploadFileSizeBytes, StorageMaxUploadFiles, isStorageDiskName, isStorageDiskDriverName, isStorageDiskVisibility, storageDiskNameOrNull, storageDiskDriverNameOrNull, storageDiskVisibilityOrNull, storageDiskManifestEntry, storageDiskManifestEntryOrNull, storageDiskNames, storageDisks, storageDiskCount, storageDiskHasEntries, storageDiskFirstEntry, storageDiskFirstEntryOrNull, storageDiskFirstName, storageDiskFirstNameOrNull, storageDefaultDiskName, storageConfiguredDefaultDiskName, storageDefaultDiskManifestEntry, storageHasDefaultDisk, storageNonDefaultDiskNames, storageNonDefaultDisks, storageNonDefaultDiskCount, storageHasNonDefaultDisks, storageFirstNonDefaultDisk, storageFirstNonDefaultDiskOrNull, storageFirstNonDefaultDiskName, storageFirstNonDefaultDiskNameOrNull, storageDiskDriver, storageDiskDriverNames, storageDiskDriverNameCount, storageHasDiskDriverNames, storageFirstDiskDriverName, storageFirstDiskDriverNameOrNull, storageDiskNamesByDriver, storageDisksByDriver, storageDiskCountByDriver, storageHasDisksByDriver, storageFirstDiskByDriver, storageFirstDiskByDriverOrNull, storageFirstDiskNameByDriver, storageFirstDiskNameByDriverOrNull, storageDiskNamesWithoutDriver, storageDisksWithoutDriver, storageDiskCountWithoutDriver, storageHasDisksWithoutDriver, storageFirstDiskWithoutDriver, storageFirstDiskWithoutDriverOrNull, storageFirstDiskNameWithoutDriver, storageFirstDiskNameWithoutDriverOrNull, storageDiskVisibility, storageDiskVisibilityNames, storageDiskVisibilityNameCount, storageHasDiskVisibilityNames, storageFirstDiskVisibilityName, storageFirstDiskVisibilityNameOrNull, storageDiskIsDefault, storageDiskNamesByVisibility, storageDisksByVisibility, storageDiskCountByVisibility, storageHasDisksByVisibility, storageFirstDiskByVisibility, storageFirstDiskByVisibilityOrNull, storageFirstDiskNameByVisibility, storageFirstDiskNameByVisibilityOrNull, storagePublicDiskNames, storagePublicDisks, storagePublicDiskCount, storageHasPublicDisks, storageFirstPublicDisk, storageFirstPublicDiskOrNull, storageFirstPublicDiskName, storageFirstPublicDiskNameOrNull, storagePrivateDiskNames, storagePrivateDisks, storagePrivateDiskCount, storageHasPrivateDisks, storageFirstPrivateDisk, storageFirstPrivateDiskOrNull, storageFirstPrivateDiskName, storageFirstPrivateDiskNameOrNull, storageMaxUploadSizeBytes, storageMaxUploadFileSizeBytes, storageMaxUploadFiles, storageUploadLimits, storageImageLimits, storageAttachmentOrphanPolicy, type StorageAttachmentOrphanManifestShape, type StorageDiskDriverName, type StorageDiskManifestEntry, type StorageDiskName, type StorageDiskVisibility, type StorageImageLimitsManifestShape, type StorageRuntimeManifestShape, type StorageUploadLimitsManifestShape } from "./StorageManifest";
export { EmailManifest, EmailRuntimeManifest, EmailMailerIds, DefaultEmailMailer, ConfiguredDefaultEmailMailer, EmailDefaultQueue, EmailMaxAttachmentBytes, EmailMaxTotalAttachmentBytes, isEmailMailerName, isEmailMailerDriverName, emailMailerNameOrNull, emailMailerDriverNameOrNull, emailMailerManifestEntry, emailMailerManifestEntryOrNull, emailMailerNames, emailMailers, emailMailerCount, emailMailerHasEntries, emailMailerFirstEntry, emailMailerFirstEntryOrNull, emailMailerFirstName, emailMailerFirstNameOrNull, emailDefaultMailerName, emailConfiguredDefaultMailerName, emailDefaultMailerManifestEntry, emailHasDefaultMailer, emailNonDefaultMailerNames, emailNonDefaultMailers, emailNonDefaultMailerCount, emailHasNonDefaultMailers, emailFirstNonDefaultMailer, emailFirstNonDefaultMailerOrNull, emailFirstNonDefaultMailerName, emailFirstNonDefaultMailerNameOrNull, emailMailerDriver, emailMailerDriverNames, emailMailerDriverNameCount, emailHasMailerDriverNames, emailFirstMailerDriverName, emailFirstMailerDriverNameOrNull, emailMailerIsDefault, emailMailerNamesByDriver, emailMailersByDriver, emailMailerCountByDriver, emailHasMailersByDriver, emailFirstMailerByDriver, emailFirstMailerByDriverOrNull, emailFirstMailerNameByDriver, emailFirstMailerNameByDriverOrNull, emailMailerNamesWithoutDriver, emailMailersWithoutDriver, emailMailerCountWithoutDriver, emailHasMailersWithoutDriver, emailFirstMailerWithoutDriver, emailFirstMailerWithoutDriverOrNull, emailFirstMailerNameWithoutDriver, emailFirstMailerNameWithoutDriverOrNull, emailDefaultQueue, emailMaxAttachmentBytes, emailMaxTotalAttachmentBytes, emailAttachmentLimits, type EmailAttachmentLimitsManifestShape, type EmailMailerDriverName, type EmailMailerManifestEntry, type EmailMailerName, type EmailRuntimeManifestShape } from "./EmailManifest";
export { I18nManifest, I18nLocaleIds, I18nLocaleMap, I18nDefaultLocale, I18nFallbackLocale, isI18nLocaleName, i18nLocaleNameOrNull, i18nLocaleManifestEntry, i18nLocaleManifestEntryOrNull, i18nLocaleMap, i18nDefaultLocale, i18nFallbackLocale, i18nLocaleNames, i18nLocales, i18nLocaleCount, i18nLocaleHasEntries, i18nLocaleFirstEntry, i18nLocaleFirstEntryOrNull, i18nLocaleFirstName, i18nLocaleFirstNameOrNull, i18nDefaultLocaleManifestEntry, i18nFallbackLocaleManifestEntry, i18nDefaultLocaleLoaded, i18nFallbackLocaleLoaded, i18nLocaleIsDefault, i18nLocaleIsFallback, i18nNonDefaultLocaleNames, i18nNonDefaultLocales, i18nNonDefaultLocaleCount, i18nHasNonDefaultLocales, i18nFirstNonDefaultLocale, i18nFirstNonDefaultLocaleOrNull, i18nFirstNonDefaultLocaleName, i18nFirstNonDefaultLocaleNameOrNull, i18nNonFallbackLocaleNames, i18nNonFallbackLocales, i18nNonFallbackLocaleCount, i18nHasNonFallbackLocales, i18nFirstNonFallbackLocale, i18nFirstNonFallbackLocaleOrNull, i18nFirstNonFallbackLocaleName, i18nFirstNonFallbackLocaleNameOrNull, i18nResolveLocale, type I18nLocaleManifestEntry, type I18nLocaleName, type I18nResolvedLocale, type I18nManifestShape } from "./I18nManifest";
export { ReadinessManifest, ReadinessProbeIds, isReadinessProbeName, isBuiltInReadinessProbeName, isCustomReadinessProbeName, readinessProbeNameOrNull, builtInReadinessProbeNameOrNull, customReadinessProbeNameOrNull, readinessProbeManifestEntry, readinessProbeManifestEntryOrNull, readinessProbeNames, readinessProbeEntries, readinessProbeCount, readinessProbeHasEntries, readinessProbeFirstEntry, readinessProbeFirstEntryOrNull, readinessProbeFirstName, readinessProbeFirstNameOrNull, readinessProbeIsBuiltIn, readinessProbeIsCustom, builtInReadinessProbeNames, builtInReadinessProbes, builtInReadinessProbeCount, readinessHasBuiltInProbes, firstBuiltInReadinessProbe, firstBuiltInReadinessProbeOrNull, firstBuiltInReadinessProbeName, firstBuiltInReadinessProbeNameOrNull, customReadinessProbeNames, customReadinessProbes, customReadinessProbeCount, readinessHasCustomProbes, firstCustomReadinessProbe, firstCustomReadinessProbeOrNull, firstCustomReadinessProbeName, firstCustomReadinessProbeNameOrNull, type BuiltInReadinessProbeName, type CustomReadinessProbeName, type ReadinessProbeManifestEntry, type ReadinessProbeName } from "./ReadinessManifest";
export { SettingManifest, SettingGroups, SettingIds, isSettingName, isSettingGroupName, settingNameOrNull, settingGroupNameOrNull, settingManifestEntry, settingManifestEntryOrNull, settingNames, settingGroupNames, settingEntries, settingCount, settingHasEntries, settingFirstEntry, settingFirstEntryOrNull, settingFirstName, settingFirstNameOrNull, settingGroupCount, settingHasGroups, settingFirstGroupName, settingFirstGroupNameOrNull, settingNamesInGroup, settingsInGroup, settingCountInGroup, settingHasGroupEntries, settingFirstEntryInGroup, settingFirstEntryInGroupOrNull, settingFirstNameInGroup, settingFirstNameInGroupOrNull, settingNamesWithoutGroup, settingsWithoutGroup, settingCountWithoutGroup, settingHasSettingsWithoutGroup, firstSettingWithoutGroup, firstSettingWithoutGroupOrNull, firstSettingNameWithoutGroup, firstSettingNameWithoutGroupOrNull, publicSettingNames, publicSettings, publicSettingCount, settingHasPublicSettings, firstPublicSetting, firstPublicSettingOrNull, firstPublicSettingName, firstPublicSettingNameOrNull, privateSettingNames, privateSettings, privateSettingCount, settingHasPrivateSettings, firstPrivateSetting, firstPrivateSettingOrNull, firstPrivateSettingName, firstPrivateSettingNameOrNull, settingNamesByType, settingsByType, settingCountByType, settingHasTypeEntries, firstSettingByType, firstSettingByTypeOrNull, firstSettingNameByType, firstSettingNameByTypeOrNull, settingNamesWithoutType, settingsWithoutType, settingCountWithoutType, settingHasSettingsWithoutType, firstSettingWithoutType, firstSettingWithoutTypeOrNull, firstSettingNameWithoutType, firstSettingNameWithoutTypeOrNull, settingGroup, settingType, settingLabel, settingDescription, settingParameters, settingParameterNames, settingParameterNameCount, settingTotalParameterNameCount, settingHasParameters, settingFirstParameterName, settingFirstParameterNameOrNull, settingOptions, settingOptionValues, settingOptionCount, settingTotalOptionCount, settingHasOptions, settingFirstOption, settingFirstOptionOrNull, settingFirstOptionValue, settingFirstOptionValueOrNull, settingOptionLabels, settingFirstOptionLabel, settingFirstOptionLabelOrNull, settingOptionForValue, settingOptionLabelForValue, settingHasOptionValue, settingNamesWithOptions, settingsWithOptions, settingCountWithOptions, settingHasSettingsWithOptions, firstSettingWithOptions, firstSettingWithOptionsOrNull, firstSettingNameWithOptions, firstSettingNameWithOptionsOrNull, settingNamesWithoutOptions, settingsWithoutOptions, settingCountWithoutOptions, settingHasSettingsWithoutOptions, firstSettingWithoutOptions, firstSettingWithoutOptionsOrNull, firstSettingNameWithoutOptions, firstSettingNameWithoutOptionsOrNull, settingSortOrder, settingIsPublic, type SettingGroupName, type SettingManifestEntry, type SettingName, type SettingOption, type SettingOptionValue, type SettingParameters, type SettingValueFor, type SettingValues } from "./SettingManifest";
export { CacheManifest, CacheDrivers, CacheErrorModes, CacheUsesRedis, CacheRedisNamespace, isCacheDriver, isCacheErrorMode, cacheDriverOrNull, cacheErrorModeOrNull, cacheDrivers, cacheDriverCount, cacheHasDrivers, cacheFirstDriver, cacheFirstDriverOrNull, cacheErrorModes, cacheErrorModeCount, cacheHasErrorModes, cacheFirstErrorMode, cacheFirstErrorModeOrNull, cacheDriver, cacheErrorMode, cacheUsesRedis, cacheUsesMemory, cacheRedisConfigured, cacheRedisNamespace, cacheHasRedisNamespace, cacheDefaultTtlSeconds, cacheKeyPrefix, cacheHasKeyPrefix, cacheMaxEntries, cacheKeyMaxLength, cacheIsStrict, cacheIsFailOpen, cacheRememberUsesSingleflight, cacheRememberUsesDistributedLock, cacheRememberLockTtlMs, cacheRememberLockWaitTimeoutMs, cacheRememberLockPollMs, cacheRememberLockTiming, type CacheDriver, type CacheErrorMode, type CacheManifestShape, type CacheRememberLockManifestShape } from "./CacheManifest";
export { DatabaseManifest, DatabaseDefaultPage, DatabaseDefaultPerPage, DatabasePaginationQueryParamNames, databaseDefaultPage, databaseDefaultPerPage, databaseDefaultPagination, databaseNormalizePagination, databasePaginationWithDefaults, databasePaginationQueryParamNameMap, databasePaginationPageQueryParamName, databasePaginationPerPageQueryParamName, databasePaginationQueryParamNames, databasePaginationQueryParamNameCount, databaseHasPaginationQueryParamNames, databaseHasPaginationQueryParamName, databasePaginationQueryParamNameOrNull, databaseFirstPaginationQueryParamName, databaseFirstPaginationQueryParamNameOrNull, databasePaginationQueryParams, databasePaginationFromQueryParams, type DatabaseManifestShape, type DatabasePaginationManifestShape, type DatabasePaginationQueryParamMap, type DatabasePaginationQueryParamName, type DatabasePaginationQueryParamRecord, type DatabasePaginationQueryParamSource, type DatabasePaginationQueryParamValue, type DatabasePaginationQueryParams } from "./DatabaseManifest";
export { ObservabilityManifest, ObservabilityBasePath, ObservabilityServiceName, ObservabilityStaticEndpointNames, ObservabilityChannelEndpointNames, ObservabilityEndpointNames, ObservabilityEndpointPaths, isObservabilityStaticEndpointName, isObservabilityChannelEndpointName, isObservabilityEndpointName, observabilityStaticEndpointNameOrNull, observabilityChannelEndpointNameOrNull, observabilityEndpointNameOrNull, observabilityBasePath, observabilityServiceName, observabilityEnabled, observabilityDisabled, observabilityCaptureEnabled, observabilityCaptureDisabled, observabilityTracingEnabled, observabilityTracingDisabled, observabilityHttpSampleRetention, observabilityWebSocketChannelRetention, observabilityWebSocketPayloadsIncluded, observabilityWebSocketPayloadsExcluded, observabilityStaticEndpointNames, observabilityStaticEndpointNameCount, observabilityHasStaticEndpointNames, observabilityFirstStaticEndpointName, observabilityFirstStaticEndpointNameOrNull, observabilityChannelEndpointNames, observabilityChannelEndpointNameCount, observabilityHasChannelEndpointNames, observabilityFirstChannelEndpointName, observabilityFirstChannelEndpointNameOrNull, observabilityEndpointNames, observabilityEndpointNameCount, observabilityHasEndpointNames, observabilityFirstEndpointName, observabilityFirstEndpointNameOrNull, observabilityEndpointPaths, observabilityPathTemplate, observabilityPathTemplateOrNull, observabilityPath, observabilityPathOrNull, type ObservabilityChannelEndpointName, type ObservabilityEndpointName, type ObservabilityEndpointPathMap, type ObservabilityManifestShape, type ObservabilityPathParams, type ObservabilityStaticEndpointName, type ObservabilityWebSocketManifestShape } from "./ObservabilityManifest";
export { RouteManifest, RouteIds, RouteHttpMethods, RouteRequestTransports, RouteRequestMediaTypes, RouteResponseMediaTypes, appendRouteQuery, createRouteUrlBuilder, createRouteUrlBuilderOrNull, clientExportedRoutes, isRouteHttpMethod, isRouteRequestTransport, isRouteRequestMediaType, isRouteResponseMediaType, routeHttpMethodOrNull, routeRequestTransportOrNull, routeRequestMediaTypeOrNull, routeResponseMediaTypeOrNull, isRouteName, publicRoutes, routeHasPermission, routeHasTag, routeMatch, routeMatchAny, routeMatches, routeManifestEntry, routeManifestEntryOrNull, routeEntries, routeNames, routeCount, routeHasEntries, routeFirstEntry, routeFirstEntryOrNull, routeFirstName, routeFirstNameOrNull, routePath, routeMethod, routeHasMethod, routeParamNames, routeParamCount, routeHasParams, routeHasParam, routeFirstParamName, routeFirstParamNameOrNull, routeClientExported, routeGuard, routeHasGuard, routeDescription, routeHasDescription, routeIsDeprecated, routeOperationId, routeHasOperationId, routeRequestMediaType, routeHasRequestMediaType, routeRequestSchema, routeHasRequestSchema, routeRequestTransport, routeHasRequestTransport, routeHasRequestMetadata, routePermissions, routePermissionCount, routeHasPermissions, routeFirstPermission, routeFirstPermissionOrNull, routeRequiresAuth, routeResponseByStatus, routeResponsesWithSchema, routeResponseCountWithSchema, routeHasResponsesWithSchema, routeFirstResponseWithSchema, routeFirstResponseWithSchemaOrNull, routeResponsesWithMediaType, routeResponseCountWithMediaType, routeHasResponsesWithMediaType, routeFirstResponseWithMediaType, routeFirstResponseWithMediaTypeOrNull, routeResponsesWithBody, routeResponseCountWithBody, routeHasResponsesWithBody, routeFirstResponseWithBody, routeFirstResponseWithBodyOrNull, routeResponseCount, routeHasResponses, routeFirstResponse, routeFirstResponseOrNull, routeResponseBodyFlags, routeResponseBodyFlagCount, routeHasResponseBodyFlags, routeHasResponseBody, routeFirstResponseBodyFlag, routeFirstResponseBodyFlagOrNull, routeResponseMediaTypes, routeResponseMediaTypeCount, routeHasResponseMediaTypes, routeHasResponseMediaType, routeFirstResponseMediaType, routeFirstResponseMediaTypeOrNull, routeResponseSchemas, routeResponseSchemaCount, routeHasResponseSchemas, routeHasResponseSchema, routeFirstResponseSchema, routeFirstResponseSchemaOrNull, routeResponseStatuses, routeResponseStatusCount, routeHasResponseStatuses, routeHasResponseStatus, routeFirstResponseStatus, routeFirstResponseStatusOrNull, routeResponses, routeSummary, routeHasSummary, routeHasDocumentMetadata, routeTags, routeTagCount, routeHasTags, routeFirstTag, routeFirstTagOrNull, routeUrl, routeUrlOrNull, deprecatedRoutes, deprecatedRouteNames, routesWithDocumentMetadata, routeNamesWithDocumentMetadata, routesWithoutDocumentMetadata, routeNamesWithoutDocumentMetadata, routesByMethod, routeNamesByMethod, routeCountByMethod, routeHasRoutesByMethod, firstRouteByMethod, firstRouteNameByMethod, routesWithMethod, routeNamesWithMethod, routeCountWithMethod, routeHasRoutesWithMethod, firstRouteWithMethod, firstRouteNameWithMethod, routesWithoutMethod, routeNamesWithoutMethod, routeCountWithoutMethod, routeHasRoutesWithoutMethod, firstRouteWithoutMethod, firstRouteNameWithoutMethod, routesWithParams, routeNamesWithParams, routeCountWithParams, routeHasRoutesWithParams, firstRouteWithParams, firstRouteNameWithParams, routesWithoutParams, routeNamesWithoutParams, routeCountWithoutParams, routeHasRoutesWithoutParams, firstRouteWithoutParams, firstRouteNameWithoutParams, routesRequiringAuth, routeNamesRequiringAuth, authRequiredRouteCount, routeHasAuthRequiredRoutes, firstAuthRequiredRoute, firstAuthRequiredRouteName, publicRouteNames, publicRouteCount, routeHasPublicRoutes, firstPublicRoute, firstPublicRouteName, clientExportedRouteNames, clientExportedRouteCount, routeHasClientExportedRoutes, firstClientExportedRoute, firstClientExportedRouteName, nonClientExportedRoutes, nonClientExportedRouteNames, nonClientExportedRouteCount, routeHasNonClientExportedRoutes, firstNonClientExportedRoute, firstNonClientExportedRouteName, deprecatedRouteCount, routeHasDeprecatedRoutes, firstDeprecatedRoute, firstDeprecatedRouteName, nonDeprecatedRoutes, nonDeprecatedRouteNames, nonDeprecatedRouteCount, routeHasNonDeprecatedRoutes, firstNonDeprecatedRoute, firstNonDeprecatedRouteName, routeCountWithDocumentMetadata, routeHasRoutesWithDocumentMetadata, firstRouteWithDocumentMetadata, firstRouteNameWithDocumentMetadata, routeCountWithoutDocumentMetadata, routeHasRoutesWithoutDocumentMetadata, firstRouteWithoutDocumentMetadata, firstRouteNameWithoutDocumentMetadata, routesWithOperationIdMetadata, routeNamesWithOperationIdMetadata, routeCountWithOperationIdMetadata, routeHasRoutesWithOperationIdMetadata, firstRouteWithOperationIdMetadata, firstRouteNameWithOperationIdMetadata, routesWithoutOperationIdMetadata, routeNamesWithoutOperationIdMetadata, routeCountWithoutOperationIdMetadata, routeHasRoutesWithoutOperationIdMetadata, firstRouteWithoutOperationIdMetadata, firstRouteNameWithoutOperationIdMetadata, routesWithOperationId, routeNamesWithOperationId, routeCountWithOperationId, routeHasRoutesWithOperationId, firstRouteWithOperationId, firstRouteNameWithOperationId, routesWithoutOperationId, routeNamesWithoutOperationId, routeCountWithoutOperationId, routeHasRoutesWithoutOperationId, firstRouteWithoutOperationId, firstRouteNameWithoutOperationId, routesWithSummary, routeNamesWithSummary, routeCountWithSummary, routeHasRoutesWithSummary, firstRouteWithSummary, firstRouteNameWithSummary, routesWithoutSummary, routeNamesWithoutSummary, routeCountWithoutSummary, routeHasRoutesWithoutSummary, firstRouteWithoutSummary, firstRouteNameWithoutSummary, routesWithDescription, routeNamesWithDescription, routeCountWithDescription, routeHasRoutesWithDescription, firstRouteWithDescription, firstRouteNameWithDescription, routesWithoutDescription, routeNamesWithoutDescription, routeCountWithoutDescription, routeHasRoutesWithoutDescription, firstRouteWithoutDescription, firstRouteNameWithoutDescription, routesWithGuard, routeNamesWithGuard, routeCountWithGuard, routeHasRoutesWithGuard, firstRouteWithGuard, firstRouteNameWithGuard, routesWithAnyGuard, routeNamesWithAnyGuard, routeCountWithAnyGuard, routeHasRoutesWithAnyGuard, firstRouteWithAnyGuard, firstRouteNameWithAnyGuard, routesWithoutGuard, routeNamesWithoutGuard, routeCountWithoutGuard, routeHasRoutesWithoutGuard, firstRouteWithoutGuard, firstRouteNameWithoutGuard, routesWithPermission, routeNamesWithPermission, routeCountWithPermission, routeHasRoutesWithPermission, firstRouteWithPermission, firstRouteNameWithPermission, routesWithoutPermission, routeNamesWithoutPermission, routeCountWithoutPermission, routeHasRoutesWithoutPermission, firstRouteWithoutPermission, firstRouteNameWithoutPermission, routesWithParam, routeNamesWithParam, routeCountWithParam, routeHasRoutesWithParam, firstRouteWithParam, firstRouteNameWithParam, routesWithoutParam, routeNamesWithoutParam, routeCountWithoutParam, routeHasRoutesWithoutParam, firstRouteWithoutParam, firstRouteNameWithoutParam, routesWithRequestMetadata, routeNamesWithRequestMetadata, routeCountWithRequestMetadata, routeHasRoutesWithRequestMetadata, firstRouteWithRequestMetadata, firstRouteNameWithRequestMetadata, routesWithoutRequestMetadata, routeNamesWithoutRequestMetadata, routeCountWithoutRequestMetadata, routeHasRoutesWithoutRequestMetadata, firstRouteWithoutRequestMetadata, firstRouteNameWithoutRequestMetadata, routesWithRequestMediaType, routeNamesWithRequestMediaType, routeCountWithRequestMediaType, routeHasRoutesWithRequestMediaType, firstRouteWithRequestMediaType, firstRouteNameWithRequestMediaType, routesWithoutRequestMediaType, routeNamesWithoutRequestMediaType, routeCountWithoutRequestMediaType, routeHasRoutesWithoutRequestMediaType, firstRouteWithoutRequestMediaType, firstRouteNameWithoutRequestMediaType, routesWithRequestSchema, routeNamesWithRequestSchema, routeCountWithRequestSchema, routeHasRoutesWithRequestSchema, firstRouteWithRequestSchema, firstRouteNameWithRequestSchema, routesWithoutRequestSchema, routeNamesWithoutRequestSchema, routeCountWithoutRequestSchema, routeHasRoutesWithoutRequestSchema, firstRouteWithoutRequestSchema, firstRouteNameWithoutRequestSchema, routesWithRequestTransport, routeNamesWithRequestTransport, routeCountWithRequestTransport, routeHasRoutesWithRequestTransport, firstRouteWithRequestTransport, firstRouteNameWithRequestTransport, routesWithoutRequestTransport, routeNamesWithoutRequestTransport, routeCountWithoutRequestTransport, routeHasRoutesWithoutRequestTransport, firstRouteWithoutRequestTransport, firstRouteNameWithoutRequestTransport, routesWithResponses, routeNamesWithResponses, routeCountWithResponses, routeHasRoutesWithResponses, firstRouteWithResponses, firstRouteNameWithResponses, routesWithoutResponses, routeNamesWithoutResponses, routeCountWithoutResponses, routeHasRoutesWithoutResponses, firstRouteWithoutResponses, firstRouteNameWithoutResponses, routesWithResponseSchema, routeNamesWithResponseSchema, routeCountWithResponseSchema, routeHasRoutesWithResponseSchema, firstRouteWithResponseSchema, firstRouteNameWithResponseSchema, routesWithoutResponseSchema, routeNamesWithoutResponseSchema, routeCountWithoutResponseSchema, routeHasRoutesWithoutResponseSchema, firstRouteWithoutResponseSchema, firstRouteNameWithoutResponseSchema, routesWithResponseBody, routeNamesWithResponseBody, routeCountWithResponseBody, routeHasRoutesWithResponseBody, firstRouteWithResponseBody, firstRouteNameWithResponseBody, routesWithoutResponseBody, routeNamesWithoutResponseBody, routeCountWithoutResponseBody, routeHasRoutesWithoutResponseBody, firstRouteWithoutResponseBody, firstRouteNameWithoutResponseBody, routesWithResponseMediaType, routeNamesWithResponseMediaType, routeCountWithResponseMediaType, routeHasRoutesWithResponseMediaType, firstRouteWithResponseMediaType, firstRouteNameWithResponseMediaType, routesWithoutResponseMediaType, routeNamesWithoutResponseMediaType, routeCountWithoutResponseMediaType, routeHasRoutesWithoutResponseMediaType, firstRouteWithoutResponseMediaType, firstRouteNameWithoutResponseMediaType, routesWithResponseStatus, routeNamesWithResponseStatus, routeCountWithResponseStatus, routeHasRoutesWithResponseStatus, firstRouteWithResponseStatus, firstRouteNameWithResponseStatus, routesWithoutResponseStatus, routeNamesWithoutResponseStatus, routeCountWithoutResponseStatus, routeHasRoutesWithoutResponseStatus, firstRouteWithoutResponseStatus, firstRouteNameWithoutResponseStatus, routesWithTag, routeNamesWithTag, routeCountWithTag, routeHasRoutesWithTag, firstRouteWithTag, firstRouteNameWithTag, routesWithoutTag, routeNamesWithoutTag, routeCountWithoutTag, routeHasRoutesWithoutTag, firstRouteWithoutTag, firstRouteNameWithoutTag, type EmptyRouteParams, type RouteHttpMethod, type RouteMatch, type RouteMatchOptions, type RouteManifestEntry, type RouteManifestResponse, type RouteName, type RouteParams, type RouteParamValue, type RoutePathInput, type RouteQueryParams, type RouteQueryValue, type RouteRequestMediaType, type RouteRequestTransport, type RouteResponseMediaType, type RouteUrlOptions } from "./RouteManifest";
export { HttpManifest, HttpCsrfCookieName, HttpCsrfHeaderName, HttpRateLimitMaxRequests, HttpRateLimitWindowSeconds, HttpRateLimitByValues, isHttpRateLimitBy, httpRateLimitByOrNull, httpMaxBodySizeBytes, httpRequestTimeoutMs, httpSecurityHeaders, httpSecurityHeadersEnabled, httpCors, httpCorsEnabled, httpCorsAllowsAnyOrigin, httpCorsAllowedMethods, httpCorsAllowedMethodCount, httpCorsHasAllowedMethods, httpCorsFirstAllowedMethod, httpCorsFirstAllowedMethodOrNull, httpCorsAllowedHeaders, httpCorsAllowedHeaderCount, httpCorsHasAllowedHeaders, httpCorsFirstAllowedHeader, httpCorsFirstAllowedHeaderOrNull, httpCorsAllowsMethod, httpCorsAllowsHeader, httpCorsAllowCredentials, httpCorsMaxAgeSeconds, httpCsrf, httpCsrfEnabled, httpCsrfCookieName, httpCsrfHeaderName, httpCsrfCookieSecure, httpCsrfCookiePath, httpCsrfCookieSameSite, httpCsrfTokenFromCookie, httpCsrfHeaders, httpRateLimit, httpRateLimitEnabled, httpRateLimitMaxRequests, httpRateLimitWindowSeconds, httpRateLimitBy, httpRateLimitByValues, httpRateLimitByValueCount, httpRateLimitHasByValues, httpRateLimitFirstByValue, httpRateLimitFirstByValueOrNull, type HttpCsrfHeaderMap, type HttpCorsManifestShape, type HttpCsrfManifestShape, type HttpManifestShape, type HttpRateLimitBy, type HttpRateLimitManifestShape, type HttpSecurityHeadersManifestShape } from "./HttpManifest";
export { WebSocketChannelManifest, WebSocketChannelIds, WebSocketPayloadManifest, WebSocketProtocol, WebSocketRuntimeManifest, WebSocketPath, WebSocketHeartbeatIntervalSeconds, WebSocketHeartbeatTimeoutSeconds, WebSocketQueryTokenName, webSocketPath, webSocketHeartbeatIntervalSeconds, webSocketHeartbeatTimeoutSeconds, webSocketHeartbeat, webSocketLimits, webSocketQueryToken, webSocketQueryTokenName, webSocketHistory, webSocketUrl, webSocketQueryTokenEnabled, isWebSocketPayloadName, webSocketPayloadNameOrNull, webSocketPayloadManifestEntry, webSocketPayloadManifestEntryByName, webSocketPayloadManifestEntryByNameOrNull, webSocketPayloadEntries, webSocketPayloadNames, webSocketPayloadCount, webSocketHasPayloads, webSocketFirstPayload, webSocketFirstPayloadOrNull, webSocketFirstPayloadName, webSocketFirstPayloadNameOrNull, webSocketPayloadsByDirection, webSocketPayloadNamesByDirection, webSocketPayloadCountByDirection, webSocketHasPayloadsByDirection, webSocketFirstPayloadByDirection, webSocketFirstPayloadByDirectionOrNull, webSocketFirstPayloadNameByDirection, webSocketFirstPayloadNameByDirectionOrNull, webSocketServerPayloads, webSocketServerPayloadNames, webSocketServerPayloadCount, webSocketHasServerPayloads, webSocketFirstServerPayload, webSocketFirstServerPayloadOrNull, webSocketFirstServerPayloadName, webSocketFirstServerPayloadNameOrNull, webSocketClientEventPayloads, webSocketClientEventPayloadNames, webSocketClientEventPayloadCount, webSocketHasClientEventPayloads, webSocketFirstClientEventPayload, webSocketFirstClientEventPayloadOrNull, webSocketFirstClientEventPayloadName, webSocketFirstClientEventPayloadNameOrNull, webSocketPayloadsForChannel, webSocketPayloadNamesForChannel, webSocketPayloadCountForChannel, webSocketHasPayloadsForChannel, webSocketFirstPayloadForChannel, webSocketFirstPayloadForChannelOrNull, webSocketFirstPayloadNameForChannel, webSocketFirstPayloadNameForChannelOrNull, webSocketPayloadsForEvent, webSocketPayloadNamesForEvent, webSocketPayloadCountForEvent, webSocketHasPayloadsForEvent, webSocketFirstPayloadForEvent, webSocketFirstPayloadForEventOrNull, webSocketFirstPayloadNameForEvent, webSocketFirstPayloadNameForEventOrNull, webSocketPayloadsForChannelEvent, webSocketPayloadNamesForChannelEvent, webSocketPayloadCountForChannelEvent, webSocketHasPayloadsForChannelEvent, webSocketFirstPayloadForChannelEvent, webSocketFirstPayloadForChannelEventOrNull, webSocketFirstPayloadNameForChannelEvent, webSocketFirstPayloadNameForChannelEventOrNull, webSocketPayloadSchema, webSocketChannelManifestEntry, webSocketChannelManifestEntryOrNull, webSocketChannels, webSocketChannelNames, webSocketChannelCount, webSocketHasChannels, webSocketFirstChannel, webSocketFirstChannelOrNull, webSocketFirstChannelName, webSocketFirstChannelNameOrNull, webSocketPresenceChannels, webSocketPresenceChannelNames, webSocketPresenceChannelCount, webSocketHasPresenceChannels, webSocketFirstPresenceChannel, webSocketFirstPresenceChannelOrNull, webSocketFirstPresenceChannelName, webSocketFirstPresenceChannelNameOrNull, webSocketNonPresenceChannels, webSocketNonPresenceChannelNames, webSocketNonPresenceChannelCount, webSocketHasNonPresenceChannels, webSocketFirstNonPresenceChannel, webSocketFirstNonPresenceChannelOrNull, webSocketFirstNonPresenceChannelName, webSocketFirstNonPresenceChannelNameOrNull, webSocketClientEventChannels, webSocketClientEventChannelNames, webSocketClientEventChannelCount, webSocketHasClientEventChannels, webSocketFirstClientEventChannel, webSocketFirstClientEventChannelOrNull, webSocketFirstClientEventChannelName, webSocketFirstClientEventChannelNameOrNull, webSocketChannelsAllowingClientEvents, webSocketChannelNamesAllowingClientEvents, webSocketChannelsDisallowingClientEvents, webSocketChannelNamesDisallowingClientEvents, webSocketDisallowingClientEventChannelCount, webSocketHasChannelsDisallowingClientEvents, webSocketFirstChannelDisallowingClientEvents, webSocketFirstChannelDisallowingClientEventsOrNull, webSocketFirstChannelNameDisallowingClientEvents, webSocketFirstChannelNameDisallowingClientEventsOrNull, webSocketChannelsRequiringAuth, webSocketChannelNamesRequiringAuth, webSocketAuthRequiredChannelCount, webSocketHasAuthRequiredChannels, webSocketFirstAuthRequiredChannel, webSocketFirstAuthRequiredChannelOrNull, webSocketFirstAuthRequiredChannelName, webSocketFirstAuthRequiredChannelNameOrNull, webSocketPublicChannels, webSocketPublicChannelNames, webSocketPublicChannelCount, webSocketHasPublicChannels, webSocketFirstPublicChannel, webSocketFirstPublicChannelOrNull, webSocketFirstPublicChannelName, webSocketFirstPublicChannelNameOrNull, webSocketChannelRequiresAuth, webSocketChannelGuard, webSocketChannelHasGuard, webSocketChannelsWithGuard, webSocketChannelNamesWithGuard, webSocketChannelCountWithGuard, webSocketHasChannelsWithGuard, webSocketFirstChannelWithGuard, webSocketFirstChannelWithGuardOrNull, webSocketFirstChannelNameWithGuard, webSocketFirstChannelNameWithGuardOrNull, webSocketChannelsWithAnyGuard, webSocketChannelNamesWithAnyGuard, webSocketChannelCountWithAnyGuard, webSocketHasChannelsWithAnyGuard, webSocketFirstChannelWithAnyGuard, webSocketFirstChannelWithAnyGuardOrNull, webSocketFirstChannelNameWithAnyGuard, webSocketFirstChannelNameWithAnyGuardOrNull, webSocketChannelsWithoutGuard, webSocketChannelNamesWithoutGuard, webSocketChannelCountWithoutGuard, webSocketHasChannelsWithoutGuard, webSocketFirstChannelWithoutGuard, webSocketFirstChannelWithoutGuardOrNull, webSocketFirstChannelNameWithoutGuard, webSocketFirstChannelNameWithoutGuardOrNull, webSocketChannelIsPresence, webSocketChannelAllowsClientEvents, webSocketChannelReplayCount, webSocketChannelTotalReplayCount, webSocketChannelHasReplay, webSocketChannelsWithReplay, webSocketChannelNamesWithReplay, webSocketChannelCountWithReplay, webSocketHasChannelsWithReplay, webSocketFirstChannelWithReplay, webSocketFirstChannelWithReplayOrNull, webSocketFirstChannelNameWithReplay, webSocketFirstChannelNameWithReplayOrNull, webSocketChannelsWithoutReplay, webSocketChannelNamesWithoutReplay, webSocketChannelCountWithoutReplay, webSocketHasChannelsWithoutReplay, webSocketFirstChannelWithoutReplay, webSocketFirstChannelWithoutReplayOrNull, webSocketFirstChannelNameWithoutReplay, webSocketFirstChannelNameWithoutReplayOrNull, webSocketChannelPermissions, webSocketChannelPermissionCount, webSocketChannelTotalPermissionCount, webSocketChannelHasPermissions, webSocketChannelFirstPermission, webSocketChannelFirstPermissionOrNull, webSocketChannelHasPermission, webSocketChannelsWithPermissions, webSocketChannelNamesWithPermissions, webSocketChannelCountWithPermissions, webSocketHasChannelsWithPermissions, webSocketFirstChannelWithPermissions, webSocketFirstChannelWithPermissionsOrNull, webSocketFirstChannelNameWithPermissions, webSocketFirstChannelNameWithPermissionsOrNull, webSocketChannelsWithoutPermissions, webSocketChannelNamesWithoutPermissions, webSocketChannelCountWithoutPermissions, webSocketHasChannelsWithoutPermissions, webSocketFirstChannelWithoutPermissions, webSocketFirstChannelWithoutPermissionsOrNull, webSocketFirstChannelNameWithoutPermissions, webSocketFirstChannelNameWithoutPermissionsOrNull, webSocketChannelsWithPermission, webSocketChannelNamesWithPermission, webSocketChannelCountWithPermission, webSocketHasChannelsWithPermission, webSocketFirstChannelWithPermission, webSocketFirstChannelWithPermissionOrNull, webSocketFirstChannelNameWithPermission, webSocketFirstChannelNameWithPermissionOrNull, webSocketChannelClientEvents, webSocketChannelClientEventCount, webSocketChannelTotalClientEventCount, webSocketChannelHasClientEvents, webSocketChannelFirstClientEvent, webSocketChannelFirstClientEventOrNull, webSocketChannelHasClientEvent, webSocketChannelsWithClientEvents, webSocketChannelNamesWithClientEvents, webSocketChannelCountWithClientEvents, webSocketHasChannelsWithClientEvents, webSocketFirstChannelWithClientEvents, webSocketFirstChannelWithClientEventsOrNull, webSocketFirstChannelNameWithClientEvents, webSocketFirstChannelNameWithClientEventsOrNull, webSocketChannelsWithoutClientEvents, webSocketChannelNamesWithoutClientEvents, webSocketChannelCountWithoutClientEvents, webSocketHasChannelsWithoutClientEvents, webSocketFirstChannelWithoutClientEvents, webSocketFirstChannelWithoutClientEventsOrNull, webSocketFirstChannelNameWithoutClientEvents, webSocketFirstChannelNameWithoutClientEventsOrNull, webSocketChannelAllowsClientEvent, webSocketClientEventNameOrNull, webSocketChannelServerEvents, webSocketChannelServerEventCount, webSocketChannelTotalServerEventCount, webSocketChannelHasServerEvents, webSocketChannelFirstServerEvent, webSocketChannelFirstServerEventOrNull, webSocketChannelHasServerEvent, webSocketChannelsWithServerEvents, webSocketChannelNamesWithServerEvents, webSocketChannelCountWithServerEvents, webSocketHasChannelsWithServerEvents, webSocketFirstChannelWithServerEvents, webSocketFirstChannelWithServerEventsOrNull, webSocketFirstChannelNameWithServerEvents, webSocketFirstChannelNameWithServerEventsOrNull, webSocketChannelsWithoutServerEvents, webSocketChannelNamesWithoutServerEvents, webSocketChannelCountWithoutServerEvents, webSocketHasChannelsWithoutServerEvents, webSocketFirstChannelWithoutServerEvents, webSocketFirstChannelWithoutServerEventsOrNull, webSocketFirstChannelNameWithoutServerEvents, webSocketFirstChannelNameWithoutServerEventsOrNull, webSocketChannelAllowsServerEvent, webSocketServerEventNameOrNull, subscribeToChannel, unsubscribeFromChannel, messageToChannel, clientEventToChannel, typedClientEventToChannel, isWebSocketChannelName, webSocketChannelNameOrNull, isWebSocketPresenceChannelName, webSocketPresenceChannelNameOrNull, isWebSocketClientEventChannelName, webSocketClientEventChannelNameOrNull, isWebSocketChannelProtocolEventName, isWebSocketSystemEventName, isWebSocketProtocolEventName, isWebSocketServerEventName, webSocketChannelProtocolEventNameOrNull, webSocketSystemEventNameOrNull, webSocketProtocolEventNameOrNull, isWebSocketProtocolFrame, isWebSocketServerFrame, parseWebSocketServerFrame, hasWebSocketServerPayload, hasWebSocketClientEventPayload, type WebSocketChannelManifestEntry, type WebSocketClientLimitManifestShape, type WebSocketHeartbeatManifestShape, type WebSocketHistoryManifestShape, type WebSocketPayloadDirection, type WebSocketPayloadManifestEntry, type WebSocketPayloadName, type WebSocketQueryTokenManifestShape, type WebSocketRuntimeManifestShape, type WebSocketChannelName, type WebSocketPresenceChannelName, type WebSocketSystemChannel, type WebSocketSystemEventName, type WebSocketChannelProtocolEventName, type WebSocketProtocolEventName, type WebSocketClientEventChannelName, type WebSocketClientEventName, type WebSocketServerEventName, type WebSocketAppServerEventName, type WebSocketClientAction, type WebSocketFrameOptions, type WebSocketMessageOptions, type WebSocketSubscribeFrame, type WebSocketUnsubscribeFrame, type WebSocketMessageFrame, type WebSocketClientEventFrame, type WebSocketClientFrame, type WebSocketServerPayloadMap, type WebSocketClientEventPayloadMap, type WebSocketServerPayload, type WebSocketClientEventPayload, type WebSocketServerFrame, type WebSocketAppServerFrame, type TypedWebSocketAppServerFrame, type TypedWebSocketClientEventFrame, type WebSocketErrorPayload, type WebSocketErrorFrame, type WebSocketAckFrame, type WebSocketSubscribedFrame, type WebSocketUnsubscribedFrame, type WebSocketPresenceJoinFrame, type WebSocketPresenceLeaveFrame, type WebSocketProtocolFrame, type WebSocketInboundFrame } from "./WebSocketChannelManifest";
export { NotificationManifest, NotificationChannelIds, NotificationTypes, NotificationBroadcastChannel, NotificationBroadcastEvent, isNotificationType, isNotificationChannelName, isNotificationPayloadName, notificationTypeOrNull, notificationChannelNameOrNull, notificationPayloadNameOrNull, isTypedNotificationBroadcastPayload, isRegisteredNotificationBroadcastPayload, notificationManifestEntry, notificationManifestEntryOrNull, notificationBroadcastPayloadType, notificationBroadcastPayloadManifestEntry, notificationBroadcastPayloadName, notificationEmailChannelName, notificationDatabaseChannelName, notificationBroadcastDeliveryChannelName, notificationBroadcastChannel, notificationBroadcastEvent, notificationChannelNames, notificationChannelCount, notificationHasChannels, notificationFirstChannelName, notificationFirstChannelNameOrNull, notificationChannelIsEmail, notificationChannelIsDatabase, notificationChannelIsBroadcast, notificationIsBroadcastChannel, notificationIsBroadcastEvent, notificationEntries, notificationCount, notificationHasEntries, notificationFirstEntry, notificationFirstEntryOrNull, notificationPayloadName, notificationPayloadNames, notificationPayloadNameCount, notificationHasPayloadNames, notificationFirstPayloadName, notificationFirstPayloadNameOrNull, notificationTypes, notificationFirstType, notificationFirstTypeOrNull, notificationTypesWithPayload, notificationTypeCountWithPayload, notificationHasTypesWithPayload, notificationFirstTypeWithPayload, notificationFirstTypeWithPayloadOrNull, notificationTypesWithoutPayload, notificationTypeCountWithoutPayload, notificationHasTypesWithoutPayload, notificationFirstTypeWithoutPayload, notificationFirstTypeWithoutPayloadOrNull, notificationUsesPayload, notificationsWithPayload, notificationCountWithPayload, notificationHasEntriesWithPayload, notificationFirstEntryWithPayload, notificationFirstEntryWithPayloadOrNull, notificationsWithoutPayload, notificationCountWithoutPayload, notificationHasEntriesWithoutPayload, notificationFirstEntryWithoutPayload, notificationFirstEntryWithoutPayloadOrNull, type NotificationBuiltInChannelName, type NotificationManifestEntry, type NotificationPayloadName, type NotificationPayloadMap, type NotificationType, type TypedNotificationBroadcastPayload } from "./NotificationManifest";
export * from "./routes/UserPortalLogin";
```

Route modules also re-export route-specific field-state and prepare-submit
aliases such as `UserPortalLoginFieldState`, `UserPortalLoginFieldStates`,
`UserPortalLoginPrepareSubmitOptions`, and
`UserPortalLoginPreparedSubmitRequest`.

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
