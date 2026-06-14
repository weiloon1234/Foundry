# http

HTTP: routes, middleware (CORS, CSRF, rate limit, etc.), cookies, resources

[Back to index](../index.md)

## foundry::http

```rust
pub type HttpAuthorizeCallback = Arc<dyn Fn(HttpAuthorizeContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;
pub type HttpRouter = Router<AppContext>;
pub type RouteRegistrar = Arc<dyn Fn(&mut HttpRegistrar) -> Result<()> + Send + Sync>;
struct HttpAuthorizeContext
  fn app(&self) -> &AppContext
  fn actor(&self) -> &Actor
  async fn resolve_actor<M: Authenticatable>(&self) -> Result<Option<M>>
struct HttpRegistrar
  fn new() -> Self
  fn route( &mut self, path: &str, method_router: MethodRouter<AppContext>, ) -> &mut Self
  fn route_with_options( &mut self, path: &str, method_router: MethodRouter<AppContext>, options: HttpRouteOptions, ) -> &mut Self
  fn route_named<I>( &mut self, name: I, path: &str, method_router: MethodRouter<AppContext>, ) -> &mut Self
  fn route_named_with_options<I>( &mut self, name: I, path: &str, method_router: MethodRouter<AppContext>, options: HttpRouteOptions, ) -> &mut Self
  fn scope( &mut self, path: &str, f: impl FnOnce(&mut HttpScope<'_>) -> Result<()>, ) -> Result<&mut Self>
  fn nest(&mut self, path: &str, router: HttpRouter) -> &mut Self
  fn merge(&mut self, router: HttpRouter) -> &mut Self
  fn group( &mut self, prefix: &str, f: impl FnOnce(&mut HttpRegistrar) -> Result<()>, ) -> Result<&mut Self>
  fn group_with_options( &mut self, prefix: &str, options: HttpRouteOptions, f: impl FnOnce(&mut HttpRegistrar) -> Result<()>, ) -> Result<&mut Self>
  fn api_version( &mut self, version: u32, f: impl FnOnce(&mut HttpRegistrar) -> Result<()>, ) -> Result<&mut Self>
  fn resource( &mut self, name: &str, path: &str, routes: HttpResourceRoutes, ) -> &mut Self
  fn resource_with_options( &mut self, name: &str, path: &str, routes: HttpResourceRoutes, options: HttpRouteOptions, ) -> &mut Self
  fn into_router(self, app: AppContext) -> Router
  fn into_router_with_middlewares( self, app: AppContext, middlewares: Vec<MiddlewareConfig>, ) -> Router
  fn collect_route_manifest(&self) -> Result<Vec<RouteManifestEntry>>
struct HttpResourceRoutes
  fn new() -> Self
  fn index(self, route: MethodRouter<AppContext>) -> Self
  fn store(self, route: MethodRouter<AppContext>) -> Self
  fn show(self, route: MethodRouter<AppContext>) -> Self
  fn update(self, route: MethodRouter<AppContext>) -> Self
  fn destroy(self, route: MethodRouter<AppContext>) -> Self
  fn id_param(self, id_param: impl Into<String>) -> Self
struct HttpRouteBuilder
  fn public(&mut self) -> &mut Self
  fn guard<I>(&mut self, guard: I) -> &mut Self
  fn permission<I>(&mut self, permission: I) -> &mut Self
  fn permissions<I, P>(&mut self, permissions: I) -> &mut Self
  fn authorize<F, Fut>(&mut self, f: F) -> &mut Self
  fn middleware(&mut self, config: MiddlewareConfig) -> &mut Self
  fn middleware_group(&mut self, name: impl Into<String>) -> &mut Self
  fn audit_area(&mut self, area: &str) -> &mut Self
  fn audit_disabled(&mut self) -> &mut Self
  fn rate_limit(&mut self, rate_limit: RateLimit) -> &mut Self
  fn tag(&mut self, tag: &str) -> &mut Self
  fn summary(&mut self, summary: &str) -> &mut Self
  fn description(&mut self, description: &str) -> &mut Self
  fn request<T: ApiSchema>(&mut self) -> &mut Self
  fn response<T: ApiSchema>(&mut self, status: u16) -> &mut Self
  fn deprecated(&mut self) -> &mut Self
struct HttpRouteOptions
  fn new() -> Self
  fn guard<I>(self, guard: I) -> Self
  fn permission<I>(self, permission: I) -> Self
  fn permissions<I, P>(self, permissions: I) -> Self
  fn authorize<F, Fut>(self, f: F) -> Self
  fn middleware(self, config: MiddlewareConfig) -> Self
  fn allow_mfa_pending_token(self) -> Self
  fn audit_area(self, area: &str) -> Self
  fn audit_disabled(self) -> Self
  fn middleware_group(self, name: impl Into<String>) -> Self
  fn rate_limit(self, rate_limit: RateLimit) -> Self
  fn document(self, doc: RouteDoc) -> Self
  fn tag(self, tag: &str) -> Self
  fn summary(self, summary: &str) -> Self
  fn description(self, description: &str) -> Self
  fn request<T: ApiSchema>(self) -> Self
  fn response<T: ApiSchema>(self, status: u16) -> Self
  fn deprecated(self) -> Self
struct HttpScope
  fn scope( &mut self, path: &str, f: impl FnOnce(&mut HttpScope<'_>) -> Result<()>, ) -> Result<&mut Self>
  fn name_prefix(&mut self, prefix: &str) -> &mut Self
  fn public(&mut self) -> &mut Self
  fn guard<I>(&mut self, guard: I) -> &mut Self
  fn permission<I>(&mut self, permission: I) -> &mut Self
  fn permissions<I, P>(&mut self, permissions: I) -> &mut Self
  fn authorize<F, Fut>(&mut self, f: F) -> &mut Self
  fn middleware(&mut self, config: MiddlewareConfig) -> &mut Self
  fn middleware_group(&mut self, name: impl Into<String>) -> &mut Self
  fn audit_area(&mut self, area: &str) -> &mut Self
  fn audit_disabled(&mut self) -> &mut Self
  fn rate_limit(&mut self, rate_limit: RateLimit) -> &mut Self
  fn tag(&mut self, tag: &str) -> &mut Self
  fn summary(&mut self, summary: &str) -> &mut Self
  fn description(&mut self, description: &str) -> &mut Self
  fn deprecated(&mut self) -> &mut Self
  fn get<H, T>( &mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder), ) -> &mut Self
  fn post<H, T>( &mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder), ) -> &mut Self
  fn put<H, T>( &mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder), ) -> &mut Self
  fn patch<H, T>( &mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder), ) -> &mut Self
  fn delete<H, T>( &mut self, path: &str, name: &str, handler: H, configure: impl FnOnce(&mut HttpRouteBuilder), ) -> &mut Self
struct RouteManifestEntry
struct RouteManifestResponse
```

## foundry::http::cookie

```rust
enum SameSite { Strict, Lax, None }
  fn is_strict(&self) -> bool
  fn is_lax(&self) -> bool
  fn is_none(&self) -> bool
struct Cookie
  fn new<N, V>(name: N, value: V) -> Cookie<'c>
  fn named<N>(name: N) -> Cookie<'c>
  fn build<C>(base: C) -> CookieBuilder<'c>
  fn parse<S>(s: S) -> Result<Cookie<'c>, ParseError>
  fn parse_encoded<S>(s: S) -> Result<Cookie<'c>, ParseError>
  fn split_parse<S>(string: S) -> SplitCookies<'c>
  fn split_parse_encoded<S>(string: S) -> SplitCookies<'c>
  fn into_owned(self) -> Cookie<'static>
  fn name(&self) -> &str
  fn value(&self) -> &str
  fn value_trimmed(&self) -> &str
  fn name_value(&self) -> (&str, &str)
  fn name_value_trimmed(&self) -> (&str, &str)
  fn http_only(&self) -> Option<bool>
  fn secure(&self) -> Option<bool>
  fn same_site(&self) -> Option<SameSite>
  fn partitioned(&self) -> Option<bool>
  fn max_age(&self) -> Option<Duration>
  fn path(&self) -> Option<&str>
  fn domain(&self) -> Option<&str>
  fn expires(&self) -> Option<Expiration>
  fn expires_datetime(&self) -> Option<OffsetDateTime>
  fn set_name<N>(&mut self, name: N)
  fn set_value<V>(&mut self, value: V)
  fn set_http_only<T>(&mut self, value: T)
  fn set_secure<T>(&mut self, value: T)
  fn set_same_site<T>(&mut self, value: T)
  fn set_partitioned<T>(&mut self, value: T)
  fn set_max_age<D>(&mut self, value: D)
  fn set_path<P>(&mut self, path: P)
  fn unset_path(&mut self)
  fn set_domain<D>(&mut self, domain: D)
  fn unset_domain(&mut self)
  fn set_expires<T>(&mut self, time: T)
  fn unset_expires(&mut self)
  fn make_permanent(&mut self)
  fn make_removal(&mut self)
  fn name_raw(&self) -> Option<&'c str>
  fn value_raw(&self) -> Option<&'c str>
  fn path_raw(&self) -> Option<&'c str>
  fn domain_raw(&self) -> Option<&'c str>
  fn encoded<'a>(&'a self) -> Display<'a, 'c>
  fn stripped<'a>(&'a self) -> Display<'a, 'c>
struct CookieJar
  fn from_headers(headers: &HeaderMap) -> CookieJar
  fn new() -> CookieJar
  fn get(&self, name: &str) -> Option<&Cookie<'static>>
  fn remove<C>(self, cookie: C) -> CookieJar
  fn add<C>(self, cookie: C) -> CookieJar
  fn iter(&self) -> impl Iterator<Item = &Cookie<'static>>
struct SessionCookie
  fn build<'a>(name: &'a str, value: &'a str, secure: bool) -> Cookie<'a>
  fn clear(name: &str) -> Cookie<'_>
fn extract_cookie_value(headers: &HeaderMap, name: &str) -> Option<String>
```

## foundry::http::download

```rust
enum ContentDispositionType { Attachment, Inline }
fn attachment_content_disposition(filename: impl AsRef<str>) -> HeaderValue
fn content_disposition_header( disposition: ContentDispositionType, filename: impl AsRef<str>, ) -> HeaderValue
fn content_disposition_value( disposition: ContentDispositionType, filename: &str, ) -> String
fn inline_content_disposition(filename: impl AsRef<str>) -> HeaderValue
```

## foundry::http::middleware

```rust
enum MiddlewareConfig { TrustedProxy, MaintenanceMode, Cors, SecurityHeaders, Csrf, RateLimit, MaxBodySize, RequestTimeout, ETag, Compression }
enum RateLimitBy { Ip, Actor, ActorOrIp }
enum RateLimitWindow { Second, Minute, Hour }
struct Compression
  fn new() -> Self
  fn build(self) -> MiddlewareConfig
struct Cors
  fn new() -> Self
  fn allow_origin(self, origin: &str) -> Self
  fn allow_origins<I, O>(self, origins: I) -> Self
  fn allow_any_origin(self) -> Self
  fn allow_method(self, method: Method) -> Self
  fn allow_methods<I>(self, methods: I) -> Self
  fn allow_any_method(self) -> Self
  fn allow_header(self, hdr: HeaderName) -> Self
  fn allow_headers<I>(self, headers: I) -> Self
  fn allow_any_header(self) -> Self
  fn allow_credentials(self) -> Self
  fn max_age(self, seconds: u64) -> Self
  fn build(self) -> MiddlewareConfig
struct Csrf
  fn new() -> Self
  fn from_config(config: &HttpCsrfConfig) -> Result<Self>
  fn cookie_name(self, name: &str) -> Self
  fn header_name(self, name: HeaderName) -> Self
  fn secure(self, secure: bool) -> Self
  fn path(self, path: &str) -> Self
  fn same_site(self, same_site: &str) -> Self
  fn exclude(self, path: &str) -> Self
  fn exclude_paths<'a, I>(self, paths: I) -> Self
  fn build(self) -> MiddlewareConfig
struct CsrfToken
  fn value(&self) -> &str
struct ETag
  fn new() -> Self
  fn build(self) -> MiddlewareConfig
struct MaintenanceMode
  fn new() -> Self
  fn bypass_secret(self, secret: impl Into<String>) -> Self
  fn allow_query_bypass(self) -> Self
  fn build(self) -> MiddlewareConfig
struct MaxBodySize
  fn bytes(n: usize) -> Self
  fn kb(n: usize) -> Self
  fn mb(n: usize) -> Self
  fn build(self) -> MiddlewareConfig
struct MiddlewareGroups
  fn get(&self, name: &str) -> Option<&Vec<MiddlewareConfig>>
struct RateLimit
  fn new(max: u32) -> Self
  fn per_second(self) -> Self
  fn per_minute(self) -> Self
  fn per_hour(self) -> Self
  fn key_prefix(self, prefix: &str) -> Self
  fn by_actor(self) -> Self
  fn by_actor_or_ip(self) -> Self
  fn rate_limit_by(&self) -> RateLimitBy
  fn max(&self) -> u32
  fn window(&self) -> RateLimitWindow
  fn key_prefix_str(&self) -> &str
  fn build(self) -> MiddlewareConfig
struct RealIp
struct RequestTimeout
  fn millis(n: u64) -> Self
  fn secs(n: u64) -> Self
  fn mins(n: u64) -> Self
  fn duration(d: Duration) -> Self
  fn build(self) -> MiddlewareConfig
struct SecurityHeaders
  fn new() -> Self
  fn disable_hsts(self) -> Self
  fn frame_options(self, value: &str) -> Self
  fn content_security_policy(self, policy: &str) -> Self
  fn referrer_policy(self, policy: &str) -> Self
  fn header(self, name: HeaderName, value: HeaderValue) -> Self
  fn build(self) -> MiddlewareConfig
struct TrustedProxy
  fn new() -> Self
  fn cloudflare() -> Self
  fn trust_all(self) -> Self
  fn with_header(self, hdr: HeaderName) -> Self
  fn trusted_cidr(self, cidr: &str) -> Self
  fn build(self) -> MiddlewareConfig
```

## foundry::http::resource

```rust
trait ApiResource
  fn transform(item: &T) -> Value
  fn make(item: &T) -> Value
  fn collection(items: &[T]) -> Vec<Value>
  fn paginated(paginated: &Paginated<T>, base_url: &str) -> Value
```

## foundry::http::response

```rust
struct MessageResponse
  fn new(message: impl Into<String>) -> Self
  fn ok() -> Self
```

## foundry::http::routes

```rust
struct RouteRegistry
  fn new() -> Self
  fn register(&mut self, name: impl Into<RouteId>, pattern: impl Into<String>)
  fn url<I>(&self, name: I, params: &[(&str, &str)]) -> Result<String>
  fn has<I>(&self, name: I) -> bool
  fn iter(&self) -> impl Iterator<Item = (&RouteId, &String)>
  fn signed_url( &self, name: impl Into<RouteId>, params: &[(&str, &str)], signing_key: &[u8], expires_at: DateTime, ) -> Result<String>
  fn verify_signature(url: &str, signing_key: &[u8]) -> Result<()>
```

## Notes

- `HttpConfig.security_headers` is applied globally by default with HSTS disabled until explicitly enabled.
- `HttpConfig.trusted_proxy` honors forwarded client IP headers only from configured CIDRs; the default CIDR set trusts Cloudflare ranges, and `TrustedProxy::new()` uses the same Cloudflare-safe default.
- Config-derived CORS validates origins, methods, and headers at boot; wildcard origins with credentials are rejected.
- Config-derived CSRF is opt-in; code-registered `Csrf` remains source-compatible and path exclusions are segment-aware.
- Config-derived body-limit, request-timeout, and rate-limit rejections return JSON `ErrorResponse` bodies with HTTP 413, 408, and 429.
- Actor-only rate limits require an authenticated actor; use `actor_or_ip` when a global rate limit needs an IP fallback.
- IP rate limits use `TrustedProxy` real IP when available and otherwise fall back to TCP peer connect info on the real server path.

