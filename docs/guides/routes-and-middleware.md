# Routes & Middleware Guide

HTTP routing, middleware stack, named URLs, API versioning, rate limiting, and more.

> For auth guards, permissions, and policies on routes, see [Auth Guide](auth.md).

---

## Quick Start

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route("/health", get(health));
    r.route("/posts", get(list_posts));
    r.route("/posts", post(create_post));
    Ok(())
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}
```

Register in bootstrap:

```rust
App::builder()
    .register_routes(routes)
    .run_http()?;
```

---

## Route Registration

### Basic Routes

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route("/posts", get(list_posts));
    r.route("/posts", post(create_post));
    r.route("/posts/:id", get(show_post));
    r.route("/posts/:id", put(update_post));
    r.route("/posts/:id", delete(delete_post));
    Ok(())
}
```

### Routes with Options

Attach guards, permissions, middleware, and rate limits to individual routes:

```rust
r.route_with_options("/posts", post(create_post),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .permission(Permission::PostsWrite)
        .rate_limit(RateLimit::new(10).per_minute().by_actor()));
```

For MFA completion routes, opt in to pending tokens explicitly. This is what allows the short-lived
`auth:mfa_pending` token to reach `/auth/mfa/verify` while every other guarded route continues to
reject it.

```rust
r.route_with_options("/auth/mfa/verify", post(foundry::auth::mfa::routes::verify),
    HttpRouteOptions::new()
        .guard(Guard::Admin)
        .allow_mfa_pending_token());
```

Generated `RouteManifest.ts` exposes this exception as `allowsMfaPendingToken`,
with helpers such as `routeAllowsMfaPendingToken(name)`,
`routesAllowingMfaPendingToken()`, and `routesRejectingMfaPendingToken()`.
MFA flows and route guards can read the backend-owned exception instead of
duplicating the verification-route list.
Scoped MFA route groups can set the same default with
`scope.allow_mfa_pending_token()` and still override individual child routes
through the route builder.

Built-in audit logging is activated the same way: opt an admin route tree into an audit area once,
then let child routes inherit it.

```rust
r.route_with_options("/admin/users", post(create_admin_user),
    HttpRouteOptions::new()
        .guard(Guard::Admin)
        .audit_area("admin"));
```

See [Auth Guide](auth.md) for setting up `Guard` and `Permission` enums.

### Scope DSL

For portal-style route modules, prefer `scope()` so path prefixes, relative route names, shared tags, and shared access rules live in one place:

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.api_version(1, |r| {
        r.scope("/admin", |admin| {
            admin.name_prefix("admin").audit_area("admin");

            admin.scope("/auth", |auth| {
                auth.name_prefix("auth").tag("admin:auth");

                auth.post("/login", "login", login, |route| {
                    route.public();
                    route.summary("Admin login");
                    route.request::<AdminLoginRequest>();
                    route.response::<TokenPair>(200);
                });

                auth.get("/me", "me", me, |route| {
                    route.guard(Guard::Admin);
                    route.summary("Get authenticated admin profile");
                    route.response::<AdminMeResponse>(200);
                });

                Ok(())
            })?;

            admin.scope("/profile", |profile| {
                profile
                    .name_prefix("profile")
                    .tag("admin:profile")
                    .guard(Guard::Admin);

                profile.put("", "update", update_profile, |route| {
                    route.summary("Update admin profile");
                    route.request::<UpdateAdminProfileRequest>();
                    route.response::<AdminMeResponse>(200);
                });

                Ok(())
            })?;

            Ok(())
        })?;

        Ok(())
    })?;

    Ok(())
}
```

This registers named routes such as `admin.auth.login`, `admin.auth.me`, and `admin.profile.update` automatically.

### Area-Gated Audit Logging

Foundry's built-in audit writer is area-gated by default. A mutation is only written to `audit_logs`
when all of the following are true:

- the current HTTP route/scope/group resolves to an audit area
- the model has not opted out with `#[foundry(audit = false)]`

Audit activation is code-driven; config only controls payload redaction. `config:publish` and
`env:publish` emit `[audit]` / `AUDIT__*` defaults for credential-like field redaction.

The usual project-level setup is one line on the outer admin scope:

```rust
r.scope("/admin", |admin| {
    admin
        .name_prefix("admin")
        .guard(Guard::Admin)
        .audit_area("admin");

    admin.post("/users", "store", create_admin_user, |_| {});

    admin.scope("/sensitive", |sensitive| {
        sensitive.name_prefix("sensitive").audit_disabled();
        sensitive.post("/exports", "exports", export_sensitive_data, |_| {});
        Ok(())
    })?;

    Ok(())
})?;
```

Use `audit_disabled()` for exceptions inside an audited parent scope, or override with a different
area such as `support.audit_area("support")`.

Generated `RouteManifest.ts` exposes the resolved area as `auditArea`, with helpers such as
`routeAuditArea(name)`, `routesWithAuditArea("admin")`, `routesWithAnyAuditArea()`, and
`routesWithoutAuditArea()`. This keeps route audit dashboards, admin navigation, and generated docs
on the backend-owned audit policy instead of a duplicated frontend map.

Handlers can also read the resolved area through the public `CurrentRequest` extractor:

```rust
async fn create_admin_user(
    request: CurrentRequest,
    State(app): State<AppContext>,
) -> Result<StatusCode> {
    assert_eq!(request.audit_area.as_deref(), Some("admin"));
    Ok(StatusCode::CREATED)
}
```

### Route Groups

Prefix a set of routes without nesting into a separate router:

```rust
r.group("/admin", |r| {
    r.route("/dashboard", get(dashboard));     // /admin/dashboard
    r.route("/users", get(admin_users));        // /admin/users
    r.route("/users/:id", get(admin_user));     // /admin/users/:id
    Ok(())
})?;
```

Groups can nest:

```rust
r.group("/api", |r| {
    r.group("/v1", |r| {
        r.route("/posts", get(v1_posts));       // /api/v1/posts
        Ok(())
    })?;
    Ok(())
})?;
```

If a whole group shares the same guard, middleware, or documentation tags, use `group_with_options()` to define those defaults once:

```rust
r.group_with_options(
    "/api/admin",
    HttpRouteOptions::new()
        .guard(Guard::Admin)
        .audit_area("admin")
        .middleware_group("api")
        .tag("admin"),
    |r| {
        r.route("/users", get(list_admin_users));
        r.route_with_options(
            "/stats",
            get(admin_stats),
            HttpRouteOptions::new().summary("Admin stats"),
        );
        Ok(())
    },
)?;
```

Per-route options inside the group are merged with the group defaults.

Use `group()` and `group_with_options()` when you want the low-level API directly.
Use `scope()` when you also want relative route names and the higher-level route
builder. Scope defaults cover access, authorization callbacks, middleware,
middleware groups, audit areas, rate limits, documentation tags,
request/response/validation-error documentation, MFA pending-token allowance,
and endpoint helper export toggles such as `scope.client_export(false)` /
`scope.without_client_export()`. Scope-level responses are inherited by every
child route, so use them only for shared responses such as a common validation
envelope or an intentionally uniform route group response.

### API Versioning

Shorthand for `/api/v{N}` groups:

```rust
r.api_version(1, |r| {
    r.route("/users", get(list_users_v1));     // /api/v1/users
    r.route("/posts", get(list_posts_v1));     // /api/v1/posts
    Ok(())
})?;

r.api_version(2, |r| {
    r.route("/users", get(list_users_v2));     // /api/v2/users
    Ok(())
})?;
```

### Nest & Merge

For integrating external Axum routers:

```rust
// Nest under a prefix
let admin_router = Router::new().route("/stats", get(stats));
r.nest("/admin", admin_router);  // /admin/stats

// Merge at the same level (no prefix)
let health_router = Router::new().route("/healthz", get(healthz));
r.merge(health_router);  // /healthz
```

---

## Named Routes & URL Generation

### Registering Named Routes

With `scope()`, route names are usually relative and compose automatically:

```rust
r.scope("/admin", |admin| {
    admin.name_prefix("admin");

    admin.scope("/users", |users| {
        users.name_prefix("users");
        users.get("", "index", list_users, |_| {});
        users.get("/:id", "show", show_user, |_| {});
        Ok(())
    })?;

    Ok(())
})?;
```

The example above registers `admin.users.index` and `admin.users.show`.

The lower-level named route APIs remain available when you want to register names manually:

```rust
#[derive(Clone, Copy, FoundryId)]
#[foundry(id = RouteId)]
enum Route {
    #[foundry(value = "posts.list")]
    PostsList,
    #[foundry(value = "posts.show")]
    PostsShow,
    #[foundry(value = "password.reset")]
    PasswordReset,
    #[foundry(value = "posts.create")]
    PostsCreate,
}

r.get(Route::PostsList, "/posts", list_posts, |_| {});
r.get(Route::PostsShow, "/posts/:id", show_post, |_| {});
r.get(Route::PasswordReset, "/reset/:token", reset_form, |_| {});

// Named + options
r.post(Route::PostsCreate, "/posts", create_post, |route| {
    route.guard(Guard::User);
});

// Lower-level escape hatch when you need to pass a pre-built MethodRouter.
// Add method docs if the route should generate a submittable frontend helper.
r.route_named_with_options(
    Route::PostsCreate,
    "/posts",
    post(create_post),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .document(RouteDoc::new().post()),
);
```

### Resource Routes

For common CRUD route sets, use `resource()` or `resource_with_options()`:

```rust
r.resource_with_options(
    "posts",
    "/posts",
    HttpResourceRoutes::new()
        .index(get(list_posts))
        .store(post(create_post))
        .show(get(show_post))
        .update(put(update_post))
        .destroy(delete(delete_post)),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .middleware_group("api")
        .tag("posts"),
);
```

This registers conventional named routes such as `posts.index`, `posts.store`, `posts.show`, `posts.update`, and `posts.destroy`.

### Generating URLs

In a handler:

```rust
async fn some_handler(State(app): State<AppContext>) -> Result<impl IntoResponse> {
    let url = app.route_url(Route::PostsShow, &[("id", "42")])?;
    // → "/posts/42"

    Ok(Json(json!({ "url": url })))
}
```

### Frontend Route URLs

Named routes are also exported to TypeScript during `types:export`. This lets
frontend code use the same route id SSOT instead of hand-writing internal API
paths:

```typescript
import { RouteIds, createRouteUrlBuilder } from "@shared/types/generated";

const adminRouteUrl = createRouteUrlBuilder({ basePath: "/api/v1/admin" });

api.get(adminRouteUrl(RouteIds.admin.users.show, { id: userId }));
// -> "/users/123" for an Axios client whose baseURL is "/api/v1/admin"
```

`RouteManifest.ts` is generated from registered named routes. It supports Axum
`{id}` / `{*path}` params and legacy `:id` params, URL-encodes substituted
values with the same RFC3986 path-param encoding as Rust `RouteRegistry::url`,
including escaping `!'()*`, preserves `/` separators for catch-all `{*path}`
params, and fails fast for duplicate route ids during export. Routes without
path params use an exact empty params type in TypeScript, so accidental param
objects are rejected and route options can be passed directly as the second
argument. Extra Rust params that are not used by the path become query params,
matching Laravel-style route helpers, and generated TypeScript route helpers use
`RouteUrlOptions.query` for the same purpose. `createRouteUrlBuilder(...)`
preserves builder-level query defaults and merges per-call query values over
them, so shared filters or tenant hints stay attached unless the route call
overrides them. Rust route URL generation rejects duplicate path parameter keys
instead of letting one value silently replace another; repeated non-path query
keys remain supported for repeated query strings such as `tag=rust&tag=dx`.
`routeMatch(...)`, `routeMatches(...)`, and `routeMatchAny(...)`
use the same generated path templates for frontend active-route checks and
decoded path-param extraction, including base-path stripping through
`RouteMatchOptions.basePath`. Legacy `:id` params are recognized only when the
whole path segment starts with `:`, matching Rust route parsing, so literal text
such as `v1:beta` stays literal in both generated URL builders and route
matching. `routeNameOrNull(...)` safely normalizes runtime route strings into
the generated `RouteName` union for menus, guards, and route audits that receive
backend-owned route metadata.

Generated endpoint helpers under `routes/` use the same path-param metadata:
helpers for parameterized routes require params when the endpoint is created, for
example `AdminUsersShow(api, { id: userId })`, and then reuse those params on
`submitForm()` unless a submit call overrides them.

OpenAPI output uses the same path-parameter source, but renders standard
OpenAPI templates: `{*path}` and `:id` become `{path}` and `{id}` and each path
parameter is emitted as a required `in: path` parameter. Catch-all params carry
an `x-foundry-catch-all` marker for tooling that wants to preserve that
framework-specific meaning. GET and HEAD routes with plain object-shaped request
DTOs whose fields are scalar values or scalar arrays emit those DTO fields as
`in: query` parameters instead of a request body, with array fields marked
`style: form` and `explode: true` to document repeated query keys such as
`tag=rust&tag=dx`, matching generated TypeScript helper submission behavior.
Wrapper request schemas such as `Collection<T>`, nested object DTOs, and DTOs
containing `UploadedFile` stay body/multipart requests. Operations with a
typed request also include `x-foundry-request-transport` (`body` or `query`),
and the generated `RouteManifest` exposes the same decision as
`requestTransport`, so frontend tooling can tell whether a route helper submits
its typed request as body or query params. Generated endpoint helpers use the
same repeated-key route query encoder for query-transport DTOs that `routeUrl`
uses for `RouteUrlOptions.query`, so custom clients receive a fully encoded URL
instead of adapter-specific array params. Extra URL query values should use
generated submit `query` options or `route.query`; query-transport helpers reject
`options.request.params` so custom adapter params cannot override the typed DTO
query payload. Body-bearing routes also expose `requestMediaType`
(`application/json` or `multipart/form-data`), matching the OpenAPI request body
content type, the `x-foundry-request-media-type` operation extension, and
generated TypeScript submit defaults. Frontend schema explorers and client
adapters can read these through generated helpers such as
`routeRequestSchema()`, `routeRequestTransport()`, `routeRequestMediaType()`,
`routeResponseSchemas()`, `routeResponseByStatus()`,
`routeResponsesWithSchema()`, `routeResponsesWithMediaType()`, and
`routeResponsesWithBody()`. Route-specific TypeScript modules also export
static request metadata predicates such as `{RouteName}HasRequestSchema`,
`{RouteName}HasRequestTransport`, `{RouteName}HasRequestMediaType`, and
`{RouteName}HasRequestMetadata` for docs panels and custom clients that already
import the generated route helper module.
Direct `Vec<T>`, `BTreeSet<T>`, and `HashSet<T>` route roots are documented as
arrays, while string-keyed `BTreeMap<String, T>` and `HashMap<String, T>` roots
are documented as object maps.
Direct `UploadedFile` and `Vec<UploadedFile>` request roots are documented as a
multipart object with the same repeated `file` field used by generated endpoint
helpers. Nullable direct file roots document that `file` field as optional.
Named routes also default OpenAPI `operationId` to the route id so generated
clients use stable names from the same route SSOT.
Override it with `RouteDoc::operation_id(...)` or
`HttpRouteOptions::operation_id(...)` only when client tooling needs a different
method name. OpenAPI operations for named routes also include
`x-foundry-route-id`, so docs, audits, and generated clients can still map an
operation back to the canonical Foundry route id when `operationId` is
overridden.
Documented route methods must be one of Foundry's typed HTTP method set:
`get`, `post`, `put`, `patch`, `delete`, `head`, or `options`. OpenAPI and
TypeScript route manifest generation reject unsupported method strings instead
of emitting invalid client contracts.
OpenAPI generation rejects blank, padded, relative, or malformed route paths
with empty parameter tokens such as `{}`, `{*}`, or `:`, duplicate normalized
method/path docs, duplicate `operationId` values, and route ids that would
collide in generated `RouteIds` after dotted camelCase normalization instead of
letting later registrations overwrite earlier client contracts.
Direct route-doc metadata must also keep `operationId`, `x-foundry-route-id`,
`summary`, `description`, and tags non-empty and trimmed when present, with
unique tags per route. Route ids must use non-empty dotted segments and cannot
mix parent route ids with child route-id groups.
It also rejects conflicting component schemas with the same `ApiSchema::schema_name()`;
types that intentionally have different JSON shapes should expose distinct schema
names. Component schema names must be non-empty and contain only ASCII letters,
digits, dots, hyphens, or underscores, matching OpenAPI component-key
requirements.
Each route included in OpenAPI should document at least one response with
`response::<T>(status)` or `validation_errors()`. Each route should document at
most one response schema per status; duplicate response statuses are rejected
instead of allowing OpenAPI or TypeScript status maps to overwrite one contract
with another. Documented response statuses must also be valid HTTP status codes
in the `100` through `599` range. Client-exported endpoint helpers additionally
need at least one documented `2xx` response, such as `response::<()>(204)` for a
bodyless success route; otherwise disable client export for that route.
Use `make:response` for new backend-owned response DTOs; it scaffolds
`serde::Serialize`, `foundry::ts_rs::TS`, and `ApiSchema` so the DTO can be used
directly in `route.response::<T>(status)`, generated TypeScript, and OpenAPI.

Guarded routes also emit Foundry auth metadata under `x-foundry-auth`, including
whether auth is required, the selected guard and permissions,
`allowsMfaPendingToken`, and `hasAuthorizeCallback`. Use this extension for
generated clients, docs, or test tooling that needs to understand Foundry route
auth without maintaining a second copy of those rules. OpenAPI generation rejects
blank, padded, or duplicate guard/permission metadata and rejects guard,
permission, or authorize-callback metadata when auth is not required, matching
the generated route manifest contract.
Routes with policy metadata also emit `x-foundry-route-policy`, including the
resolved `middlewareGroup`, resolved `auditArea`, and route-level `rateLimits`
rows with `maxRequests`, `windowSeconds`, and `by`. Backend-only rate-limit key
prefixes are not exported. OpenAPI generation rejects blank or padded
`middlewareGroup` / `auditArea` values, route policy limiter rows with zero
max/window values or `windowSeconds` values above JavaScript's safe integer
range, unknown `by` values, and actor-only limiter rows on routes that do not
require authentication, matching the generated route manifest contract.
Guarded routes with a typed request or response contract also include
framework-owned `401` and `403` `ErrorResponse` entries in the generated route
manifest, and guarded OpenAPI operations document the same responses
automatically.
The generated TypeScript `RouteManifest` exposes the same auth-required signal as
`requiresAuth`, including dynamic `authorize(...)` routes that do not name a
guard or permission, and `hasAuthorizeCallback` for routes with dynamic backend
authorization callbacks. It also exports `RouteManifestEntry` and
`RouteManifestResponse`, plus `RouteHttpMethod`, `RouteHttpMethods`,
`RouteRequestTransports`, `RouteRequestMediaTypes`, `RouteResponseMediaTypes`,
and the matching `isRoute...(...)` / `route...OrNull(...)` helpers for route
method, request transport, request media type, and response media type strings.
Frontend route guards, menus, and route-audit tools can use generated selectors
such as `routeManifestEntry()`, `routeEntries()`, `routeNames()`, `routePath()`,
`routeMethod()`, `routeParamNames()`, `routeGuard()`, `routeMiddlewareGroup()`,
`routeHasAuthorizeCallback()`, `routeHasAuthorizeCallbackOrNull()`,
`routesRequiringAuth()`, `routeNamesRequiringAuth()`, `publicRoutes()`,
`routeHasAuthRequiredRoutes()`, `routesWithAuthorizeCallback()`,
`routesWithoutAuthorizeCallback()`, `firstAuthRequiredRoute()`,
`firstAuthRequiredRouteOrNull()`, `publicRouteNames()`,
`routeHasPublicRoutes()`, `firstPublicRoute()`, `firstPublicRouteOrNull()`,
`clientExportedRoutes()`, `clientExportedRouteNames()`,
`routeHasClientExportedRoutes()`, `firstClientExportedRoute()`,
`firstClientExportedRouteOrNull()`,
`routesByMethod()`, `routeNamesByMethod()`, `routesWithGuard()`,
`routeNamesWithGuard()`, `routesWithMiddlewareGroup()`,
`routeNamesWithMiddlewareGroup()`, `routesWithPermission()`,
`routeNamesWithPermission()`, `routesWithTag()`, and `routeNamesWithTag()`
instead of hand-scanning the manifest object. Route docs and deprecation
tooling can also use `routeOperationId()`, `routeSummary()`,
`routeDescription()`, `routeIsDeprecated()`, `deprecatedRoutes()`,
`deprecatedRouteNames()`, `routeHasDeprecatedRoutes()`,
`firstDeprecatedRoute()`, `firstDeprecatedRouteOrNull()`,
`routesWithOperationId()`, and
`routeNamesWithOperationId()`, plus document/guard group summaries such as
`routeCountWithDocumentMetadata()`, `routeHasRoutesWithGuard(...)`,
`firstRouteWithoutGuard()`, `routeHasRoutesWithMiddlewareGroup(...)`,
`firstRouteNameWithPermission(...)`, and `routeHasRoutesWithTag(...)` from the
same generated manifest. Schema-aware
tooling can filter with `routesWithRequestSchema()`,
`routeNamesWithRequestSchema()`, `routesWithRequestTransport()`,
`routesWithRequestMediaType()`, `routesWithResponseSchema()`,
request group summaries such as `routeCountWithRequestSchema(...)`,
`routeHasRoutesWithRequestTransport(...)`,
`firstRouteWithRequestMediaType(...)`, `routesWithResponseStatus()`,
`routeNamesWithResponseStatus()`, and response
group summaries such as `routeCountWithResponseStatus(...)`,
`routeHasRoutesWithResponseSchema(...)`, `firstRouteWithResponseBody(...)`, and
`firstRouteNameWithResponses()`. Generic route detail screens can also filter
one route's backend-owned response rows with `routeResponsesWithSchema()`,
`routeResponsesWithMediaType()`, `routeResponsesWithBody()`, and their count,
presence, first-response, and route-local `OrNull` first-response variants
instead of local `routeResponses(name).filter(...)` or `... ?? null` wrappers.
Core route group `OrNull` first selectors such as
`firstRouteByMethodOrNull(...)`, `firstRouteWithParamsOrNull()`,
`firstAuthRequiredRouteOrNull()`, `firstPublicRouteOrNull()`, and
`firstDeprecatedRouteOrNull()` provide the same nullable contract for first
matches from backend-owned route groups.
Document metadata route group `OrNull` selectors such as
`firstRouteWithDocumentMetadataOrNull()`, `firstRouteWithOperationIdOrNull(...)`,
`firstRouteWithSummaryOrNull()`, and `firstRouteWithDescriptionOrNull()` give
route docs and OpenAPI audits the same null-for-missing contract.
Guard, permission, tag, and named-param route group `OrNull` selectors such as
`firstRouteWithGuardOrNull(...)`, `firstRouteWithPermissionOrNull(...)`,
`firstRouteWithParamOrNull(...)`, and `firstRouteWithTagOrNull(...)` keep
navigation and policy tooling on the same generated nullable contract.
Request and response route group `OrNull` selectors such as
`firstRouteWithRequestSchemaOrNull(...)`,
`firstRouteWithRequestTransportOrNull(...)`,
`firstRouteWithResponseSchemaOrNull(...)`, and
`firstRouteWithResponseStatusOrNull(...)` give schema explorers and generated
client dashboards the same null-for-missing contract.
Each `RouteManifestResponse` includes `hasBody` and `mediaType`, using the same
no-content status and `Unit` response rule that OpenAPI uses when deciding
whether to emit response content; OpenAPI response objects expose the same
decision through `x-foundry-response-has-body` and
`x-foundry-response-media-type`. `UploadedFile`, `Vec<UploadedFile>`, and
nullable `Option<_>` wrappers around those response roots use
`application/octet-stream` media metadata. Directly constructed route manifest
entries must use non-empty,
trimmed request and response schema names whose plain atoms contain only ASCII
letters, digits, dots, hyphens, or underscores. Generic schema expressions must
use Foundry's supported wrappers (`Array<T>`, `Map<T>`, `Nullable<T>`,
`PaginatedResponse<T>`, `CursorPaginated<T>`, or `Collection<T>`) with a
non-empty inner schema. Direct entries must also use a non-empty trimmed `path`
that starts with `/`, and a `params` list that exactly matches the params
declared by `path`. Guard or permission metadata requires `requires_auth: true`,
and authorize callback metadata also requires `requires_auth: true`. Middleware
group and audit area metadata must be non-empty and trimmed when present.
Guard ids, permission ids, `operationId`, `summary`, `description`, and route
tags must also be non-empty and trimmed when present. Direct permissions and
tags must be unique per route, and `operationId` values must be unique across
the generated manifest to match OpenAPI. Route ids must use non-empty dotted
segments and avoid camelCase-normalized segment collisions so `RouteIds` and
OpenAPI `x-foundry-route-id` metadata stay aligned.
Request transport/media metadata must match the backend-collected shape: only
routes with request schemas carry request transport metadata, query requests
leave `requestMediaType` empty, and body requests include it. Direct
`RouteManifestResponse` values must keep `has_body` and `media_type` aligned
with Foundry's status/schema response metadata rules.
OpenAPI-style route docs metadata (`operationId`, `description`, `tags`, and
`deprecated`) is included alongside `summary`, so frontend route guards and
navigation builders can be typed against the generated route metadata instead
of maintaining their own manifest interfaces. Text metadata is trimmed before
export, blank values are ignored, and repeated route tags are kept in first-seen
order and emitted once in both OpenAPI and the generated route manifest when
routes are collected through Foundry's registrar.
Generated endpoint helpers require that method metadata when submitting; prefer
the typed registrar or scoped helpers such as `routes.get(...)` /
`routes.post(...)`, or provide a documented method on generic
`route_named(...)` registrations before exporting a submittable frontend helper.
URL-only named routes still export through `RouteManifest` and `RouteIds`, but
they do not generate per-route endpoint helper files until the backend documents
a request or response schema.

This does not replace request validation. Handlers should keep accepting trusted
input through Foundry extractors such as `JsonValidated<T>` and `Validated<T>`.
Routes using those extractors can document the standard backend-owned `422`
validation envelope with `validation_errors()`.

### Signed URLs

Generate tamper-proof URLs with expiry (for password resets, email verification, etc.):

```rust
let url = app.signed_route_url(
    Route::PasswordReset,
    &[("token", &reset_token)],
    DateTime::now().add_days(1),  // expires in 24 hours
)?;
// → "/reset/abc123?expires=1704067200&signature=hmac_sha256..."
```

Verify in a handler:

```rust
async fn reset_form(State(app): State<AppContext>, request: Request) -> Result<impl IntoResponse> {
    app.verify_signed_url(&request.uri().to_string())?;
    // Returns Error if expired, tampered, duplicated, or appended after signing
    Ok(Html("reset form"))
}
```

Signed URL generation reserves extra query parameters named `expires` or
`signature`, because Foundry appends and signs those values internally. Signed
URL verification rejects duplicate `expires` or `signature` parameters and
rejects query parameters appended after `signature`, so only the originally
signed URL shape is accepted.
`app.signing_key` must be a base64 key that decodes to at least 32 bytes. Generate one with
`key:generate`; `doctor --deploy` reports missing keys as warnings and invalid or weak configured
keys as failures.

---

## Middleware Stack

### Global Middleware

Register middleware that runs on every request:

```rust
App::builder()
    .register_middleware(MiddlewareConfig::from(Compression))
    .register_middleware(MiddlewareConfig::from(
        SecurityHeaders::new()
            .content_security_policy("default-src 'self'")
    ))
    .register_middleware(MiddlewareConfig::from(
        Cors::new()
            .allow_origins(["https://app.example.com"])
            .allow_credentials()
    ))
    .run_http()?;
```

### Global HTTP Config

Foundry can also derive global edge middleware from split files under `config/`, for example
`config/10-http.toml`. Foundry loads every direct `config/*.toml` file in lexical order, merges
them in memory, then applies `.env` overrides. This is additive: existing route middleware and
code-registered global middleware keep working, and an explicitly registered middleware kind wins
over the config-derived duplicate.

```toml
[http]
max_body_size_bytes = 0        # 0 = no global cap
request_timeout_ms = 0         # 0 = no global timeout

[http.security_headers]
enabled = true
hsts = false                   # enable only after HTTPS is guaranteed
frame_options = "DENY"
referrer_policy = "strict-origin-when-cross-origin"
content_security_policy = ""

[http.trusted_proxy]
enabled = true
# Defaults to Cloudflare ranges. Add loopback/private CIDRs if a local reverse proxy
# sits between Cloudflare and Foundry.
trusted_cidrs = ["173.245.48.0/20", "103.21.244.0/22", "..."]
headers = ["cf-connecting-ip", "x-real-ip", "x-forwarded-for"]

[http.cors]
enabled = false
allowed_origins = []
allowed_methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
allowed_headers = ["authorization", "content-type", "x-request-id", "x-csrf-token"]
allow_credentials = false
max_age_seconds = 600

[http.csrf]
enabled = false
cookie_name = "foundry_csrf"
header_name = "x-csrf-token"
cookie_secure = true
cookie_path = "/"
cookie_same_site = "lax"       # lax | strict | none; none requires secure cookies
exclude_paths = []

[http.rate_limit]
enabled = true
max_requests = 600
window_seconds = 60
by = "actor_or_ip"             # ip | actor | actor_or_ip
key_prefix = "http:"
```

Compatibility defaults keep hard caps, CORS, and CSRF opt-in. Trusted proxy is enabled by default
for Cloudflare CIDRs only, rate limiting is enabled by default with `actor_or_ip`, and security
headers are enabled by default with HSTS off, so local HTTP and first deploys stay usable while
still publishing the production hardening knobs.
Runtime manifest export preserves the documented `0` sentinels for disabled
global body-size caps and request timeouts. It also normalizes backend-effective
security header fallbacks, CSRF header names, and CSRF SameSite values, and
rejects invalid CSRF cookie/header settings, impossible wildcard-origin CORS
credentials, duplicate CORS allow-list entries, enabled global rate-limit zero
values, global `by = "actor"` rate limits, and numbers above JavaScript's safe
integer range before frontend HTTP helpers are generated.

### Execution Order

Middleware runs in **priority order** — lower numbers run first (outermost layer):

```
Request
  │
  ├─ 0  TrustedProxy         ← extract real IP from proxy headers
  ├─ 1  MaintenanceMode      ← return 503 if in maintenance
  ├─ 10 Cors                 ← handle CORS preflight
  ├─ 20 SecurityHeaders      ← add security headers
  ├─ 25 Csrf                 ← validate CSRF tokens
  ├─ 30 RateLimit            ← check rate limits
  ├─ 40 MaxBodySize          ← enforce body size
  ├─ 50 RequestTimeout       ← enforce timeout
  ├─ 55 ETag                 ← conditional response (304)
  ├─ 60 Compression          ← compress response
  │
  ├─ [per-route middleware]   ← from HttpRouteOptions
  ├─ [auth middleware]        ← if route requires guard
  │
  └─ Handler
```

You don't need to worry about order — the framework sorts by priority automatically.

---

## Middleware Reference

### Compression

Gzip + Brotli based on `Accept-Encoding`:

```rust
MiddlewareConfig::from(Compression)
```

### CORS

```rust
MiddlewareConfig::from(
    Cors::new()
        .allow_origins(["https://app.example.com", "https://admin.example.com"])
        .allow_any_method()
        .allow_any_header()
        .allow_credentials()
        .max_age(3600)
)
```

For development:

```rust
MiddlewareConfig::from(Cors::new().allow_any_origin())
```

### Security Headers

Adds HSTS, CSP, X-Frame-Options, X-Content-Type-Options, Referrer-Policy:

```rust
MiddlewareConfig::from(
    SecurityHeaders::new()
        .content_security_policy("default-src 'self'; script-src 'self' 'unsafe-inline'")
        .frame_options("SAMEORIGIN")
)
```

Defaults (applied without any builder calls):

| Header | Default Value |
|--------|--------------|
| `Strict-Transport-Security` | `max-age=31536000; includeSubDomains` |
| `X-Content-Type-Options` | `nosniff` |
| `X-Frame-Options` | `DENY` |
| `Referrer-Policy` | `strict-origin-when-cross-origin` |
| `X-XSS-Protection` | `0` |

### CSRF

Double-submit cookie pattern for state-changing requests:

```rust
MiddlewareConfig::from(
    Csrf::new()
        .exclude("/api")       // skip CSRF for API routes (use token auth instead)
)
```

**How it works:**

- GET/HEAD/OPTIONS → generates CSRF token, sets a `foundry_csrf` cookie readable by JS
- POST/PUT/PATCH/DELETE → validates `X-CSRF-Token` header matches cookie
- Returns 403 if mismatch

The CSRF cookie uses `Path=/` and `SameSite=Lax`. It is intentionally not
`HttpOnly`, because browser JavaScript must read the token and echo it in the
request header. Exclusions are segment-aware: `.exclude("/api")` skips `/api`
and `/api/...`, but does not skip `/apiary`.

**Frontend integration:**

```javascript
import { httpCsrfHeaders } from "@shared/types/generated";

fetch('/form', {
    method: 'POST',
    headers: httpCsrfHeaders(document.cookie),
    body: formData,
});
```

**Extract token in handler** (e.g., to embed in HTML form):

```rust
async fn form(CsrfToken(token): CsrfToken) -> impl IntoResponse {
    Html(format!(r#"<input type="hidden" name="_token" value="{token}">"#))
}
```

SPAs that expose a token helper endpoint can return the backend-owned
`CsrfTokenResponse` payload, which generated TypeScript exports as `{ token }`.
`types:export` also emits `HttpManifest.ts` with the configured CSRF cookie and
header names plus `httpCsrfCookieName()`, `httpCsrfHeaderName()`,
`httpCsrfCookieSecure()`, `httpCsrfCookiePath()`,
`httpCsrfCookieSameSite()`, `httpCsrfTokenFromCookie()`, and
`httpCsrfHeaders()`, so browser clients do not need to copy those constants,
cookie attributes, or cookie parsing from TOML.
The same generated file exposes safe CORS/security-header selectors such as
`httpCorsAllowedMethods()`, `httpCorsAllowedHeaders()`, and
`httpSecurityHeadersEnabled()` for client diagnostics and admin tooling.

### Rate Limiting

```rust
// Global: 1000 requests per hour per IP
MiddlewareConfig::from(RateLimit::new(1000).per_hour())

// Per-route: 10 per minute per authenticated user
HttpRouteOptions::new()
    .rate_limit(RateLimit::new(10).per_minute().by_actor())
```

**Strategies:**

| Method | Key | Auth required? | Use case |
|--------|-----|---------------|----------|
| (default) | Client IP | No | Global rate limit |
| `.by_actor()` | Actor ID | Yes | Per-user limits |
| `.by_actor_or_ip()` | Actor ID or IP | No | Actor if authenticated, IP as fallback |

**Response headers on rate-limited requests:**

```
X-RateLimit-Limit: 1000
X-RateLimit-Remaining: 42
X-RateLimit-Reset: 1704067200
```

On limit exceeded: **429 Too Many Requests** with `Retry-After` header.
Foundry returns a JSON error body:

```json
{ "message": "Rate limit exceeded", "status": 429 }
```

Config-derived body-limit, timeout, CSRF, maintenance-mode, and rate-limit
rejections use the standard generated `ErrorResponse` contract.
Generated `HttpManifest.ts` includes the global rate-limit shape through
`HttpManifest.rateLimit`, `HttpRateLimitMaxRequests`,
`HttpRateLimitWindowSeconds`, `httpRateLimitEnabled()`,
`httpRateLimitMaxRequests()`, `httpRateLimitWindowSeconds()`, and
`httpRateLimitBy()`. Runtime mode strings can be normalized with
`httpRateLimitByOrNull()` while keeping backend-only key prefixes out of the
frontend bundle.
Generated `RouteManifest.ts` also exposes effective route-level limiter policy
through `rateLimits`, with helpers such as `routeRateLimits(name)`,
`routeHasRateLimitBy(name, "actor")`, `routesWithRateLimits()`,
`routesWithoutRateLimits()`, and `routesWithRateLimitBy("actor_or_ip")`.
Route manifests include max requests, window seconds, and key strategy; key
prefixes stay backend-only. Route-level limiter windows must be positive and fit
within JavaScript's safe integer range before OpenAPI or frontend route helpers
are generated, and actor-only limiter metadata must stay on authenticated routes.

Actor-only limits run only after an authenticated actor exists, so use route-level rate limits for
strict per-actor limits. `actor_or_ip` uses the actor once authenticated and the client IP
otherwise, which is the safer global fallback. IP keys use `RealIp` from `TrustedProxy` when
available and fall back to TCP peer connect info on the real server path.

### Max Body Size

```rust
MiddlewareConfig::from(MaxBodySize::mb(10))    // 10 MB limit
MiddlewareConfig::from(MaxBodySize::kb(512))   // 512 KB limit
MiddlewareConfig::from(MaxBodySize::bytes(1024)) // 1024 bytes
```

Returns **413 Payload Too Large** with a JSON error body if exceeded.

### Request Timeout

```rust
MiddlewareConfig::from(RequestTimeout::secs(30))
MiddlewareConfig::from(RequestTimeout::mins(5))
```

Returns **408 Request Timeout** with a JSON error body if exceeded.

### File Downloads

Use Foundry's download helpers when returning user-facing filenames:

```rust
use axum::http::header::CONTENT_DISPOSITION;
use foundry::http::download::attachment_content_disposition;

Response::builder()
    .header(CONTENT_DISPOSITION, attachment_content_disposition("report.xlsx"));
```

The helper strips path-like/control-character input, prevents header injection, emits a safe ASCII
`filename`, and includes RFC 5987 `filename*` for Unicode clients.

### ETag

Automatic conditional responses — returns 304 Not Modified when content hasn't changed:

```rust
MiddlewareConfig::from(ETag::new())
```

Computes SHA-256 of response body. If client sends `If-None-Match` header matching the ETag, returns 304 with no body. Skips responses larger than 10 MB.

### Trusted Proxy

Extract real client IP from proxy headers:

```rust
MiddlewareConfig::from(TrustedProxy::cloudflare())
```

Resolution order: `CF-Connecting-IP` → `X-Real-IP` → `X-Forwarded-For` (first entry).
When configured through `[http.trusted_proxy]`, Foundry only honors those headers if the TCP peer IP
matches `trusted_cidrs`. The default CIDR set trusts Cloudflare proxy ranges, so
code-registered `TrustedProxy::new()` and `TrustedProxy::cloudflare()` accept Cloudflare by
default. Add `trusted_cidr("127.0.0.1/32")` or config loopback CIDRs when Nginx/Caddy sits on the
same host between Cloudflare and Foundry, and reserve `trust_all()` for controlled tests.

The resolved IP is available via the `RealIp` extractor:

```rust
async fn handler(RealIp(ip): RealIp) -> impl IntoResponse {
    Json(json!({ "your_ip": ip.to_string() }))
}
```

### Maintenance Mode

Returns 503 for all requests (bypassed with secret):

```rust
MiddlewareConfig::from(
    MaintenanceMode::new()
        .bypass_secret("my-secret")
)
```

**CLI commands:**

```bash
cargo run -- down --secret=my-secret    # enter maintenance mode
cargo run -- up                          # exit maintenance mode
```

**Bypass via header:**

```bash
curl -H "X-Maintenance-Bypass: my-secret" https://app.example.com
```

---

## Middleware Groups

Define a named bundle of middleware once, apply to multiple routes by name:

### Define

```rust
App::builder()
    .middleware_group("api", vec![
        MiddlewareConfig::from(RateLimit::new(1000).per_hour()),
        MiddlewareConfig::from(Compression),
    ])
    .middleware_group("web", vec![
        MiddlewareConfig::from(Csrf::new()),
        MiddlewareConfig::from(SecurityHeaders::new()),
        MiddlewareConfig::from(Compression),
    ])
```

### Apply

```rust
fn routes(r: &mut HttpRegistrar) -> Result<()> {
    // API routes get "api" group middleware
    r.api_version(1, |r| {
        r.route_with_options("/users", get(list_users),
            HttpRouteOptions::new()
                .guard(Guard::User)
                .middleware_group("api"));
        Ok(())
    })?;

    // Web routes get "web" group middleware
    r.group("/dashboard", |r| {
        r.route_with_options("/", get(dashboard),
            HttpRouteOptions::new()
                .guard(Guard::Admin)
                .middleware_group("web"));
        Ok(())
    })?;

    Ok(())
}
```

Group middleware is prepended before any per-route middleware. You can combine a group with additional per-route middleware:

```rust
HttpRouteOptions::new()
    .middleware_group("api")
    .middleware(MiddlewareConfig::from(MaxBodySize::mb(50)))  // on top of group
```

---

## Per-Route Middleware

Apply middleware to a single route via `HttpRouteOptions`:

```rust
r.route_with_options("/upload", post(upload_file),
    HttpRouteOptions::new()
        .guard(Guard::User)
        .middleware(MiddlewareConfig::from(MaxBodySize::mb(100)))
        .middleware(MiddlewareConfig::from(RequestTimeout::mins(5))));
```

Per-route middleware runs **after** global middleware and **before** the auth check.

---

## SPA Serving

Serve a frontend SPA (React, Vue, etc.) with client-side routing fallback:

```rust
App::builder()
    .serve_spa("frontend/dist")
    .register_routes(api_routes)
    .run_http()?;
```

All requests not matched by API routes fall back to `frontend/dist/index.html`. Static assets (JS, CSS, images) are served directly from the directory.

---

## Route Listing

See all named routes:

```bash
cargo run -- routes:list
```

```
NAME                           PATH
posts.list                     /api/v1/posts
posts.show                     /api/v1/posts/:id
posts.create                   /api/v1/posts
password.reset                 /reset/:token
admin.dashboard                /admin/dashboard
```

---

## API Resources

Transform models into consistent JSON response shapes:

```rust
struct UserResource;

impl ApiResource<User> for UserResource {
    fn transform(user: &User) -> Value {
        json!({
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "joined": user.created_at.format(),
        })
    }
}
```

Use in handlers:

```rust
async fn list_users(State(app): State<AppContext>) -> impl IntoResponse {
    let db = app.database()?;
    let users = User::model_query().all(&*db).await?;
    Json(UserResource::collection(&users))
}

async fn show_user(State(app): State<AppContext>, Path(id): Path<String>) -> impl IntoResponse {
    let db = app.database()?;
    let user = User::model_query()
        .where_col(User::ID, &id)
        .first(&*db).await?
        .ok_or_else(|| Error::not_found("user not found"))?;
    Json(UserResource::make(&user))
}

// Paginated response with meta + links
async fn paginated_users(State(app): State<AppContext>, pagination: Pagination) -> impl IntoResponse {
    let db = app.database()?;
    let paginated = User::model_query()
        .paginate(&*db, pagination).await?;
    Json(UserResource::paginated(&paginated, "/api/v1/users"))
}
```

---

## Complete Example

```rust
use foundry::prelude::*;

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(AppServiceProvider)

        // Global middleware
        .register_middleware(MiddlewareConfig::from(TrustedProxy::cloudflare()))
        .register_middleware(MiddlewareConfig::from(Compression))
        .register_middleware(MiddlewareConfig::from(
            SecurityHeaders::new()
                .content_security_policy("default-src 'self'")
        ))

        // Middleware groups
        .middleware_group("api", vec![
            MiddlewareConfig::from(RateLimit::new(1000).per_hour()),
        ])
        .middleware_group("web", vec![
            MiddlewareConfig::from(Csrf::new().exclude("/api")),
        ])

        // SPA frontend
        .serve_spa("frontend/dist")

        // Routes
        .register_routes(routes)
        .run_http()
}

fn routes(r: &mut HttpRegistrar) -> Result<()> {
    // Public
    r.route("/health", get(|| async { Json(json!({ "ok": true })) }));

    // API v1
    r.api_version(1, |r| {
        r.get(Route::PostsList, "/posts", list_posts, |_| {});

        r.post(Route::PostsCreate, "/posts", create_post, |route| {
            route.guard(Guard::User);
            route.permission(Permission::PostsWrite);
            route.middleware_group("api");
            route.rate_limit(RateLimit::new(30).per_minute().by_actor());
        });

        r.get(Route::PostsShow, "/posts/:id", show_post, |route| {
            route.guard(Guard::User);
            route.middleware_group("api");
        });

        Ok(())
    })?;

    // Admin dashboard
    r.group("/admin", |r| {
        r.route_with_options("/", get(dashboard),
            HttpRouteOptions::new()
                .guard(Guard::Admin)
                .permission(Permission::AdminAccess)
                .middleware_group("web"));
        Ok(())
    })?;

    Ok(())
}
```
