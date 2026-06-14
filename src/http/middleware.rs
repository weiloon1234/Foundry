use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::HttpBody as _;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

use crate::config::{
    HttpConfig, HttpCorsConfig, HttpCsrfConfig, HttpRateLimitByConfig, HttpRateLimitConfig,
    HttpSecurityHeadersConfig, HttpTrustedProxyConfig, CLOUDFLARE_TRUSTED_CIDRS,
};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::RuntimeBackendKind;
use crate::support::runtime::RuntimeBackend;

// ---------------------------------------------------------------------------
// RealIp extension
// ---------------------------------------------------------------------------

/// Extension stored by `TrustedProxy` middleware carrying the resolved client IP.
#[derive(Clone, Debug)]
pub struct RealIp(pub IpAddr);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HttpEdgeRejection {
    RateLimited,
    PayloadTooLarge,
    Timeout,
    Cors,
}

// ---------------------------------------------------------------------------
// MiddlewareConfig — enum of all middleware types
// ---------------------------------------------------------------------------

/// Enumerates all Foundry middleware types with their configuration.
///
/// Each variant knows its priority for ordering and can be applied to a router.
/// Consumers never construct this directly — they use the individual builder
/// types (`Cors`, `SecurityHeaders`, etc.) which convert into `MiddlewareConfig`.
#[derive(Clone, Debug)]
pub enum MiddlewareConfig {
    TrustedProxy(TrustedProxy),
    MaintenanceMode(MaintenanceMode),
    Cors(Cors),
    SecurityHeaders(SecurityHeaders),
    Csrf(Csrf),
    RateLimit(RateLimit),
    MaxBodySize(MaxBodySize),
    RequestTimeout(RequestTimeout),
    ETag(ETag),
    Compression(Compression),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MiddlewareKind {
    TrustedProxy,
    MaintenanceMode,
    Cors,
    SecurityHeaders,
    Csrf,
    RateLimit,
    MaxBodySize,
    RequestTimeout,
    ETag,
    Compression,
}

impl MiddlewareConfig {
    pub(crate) fn kind(&self) -> MiddlewareKind {
        match self {
            Self::TrustedProxy(_) => MiddlewareKind::TrustedProxy,
            Self::MaintenanceMode(_) => MiddlewareKind::MaintenanceMode,
            Self::Cors(_) => MiddlewareKind::Cors,
            Self::SecurityHeaders(_) => MiddlewareKind::SecurityHeaders,
            Self::Csrf(_) => MiddlewareKind::Csrf,
            Self::RateLimit(_) => MiddlewareKind::RateLimit,
            Self::MaxBodySize(_) => MiddlewareKind::MaxBodySize,
            Self::RequestTimeout(_) => MiddlewareKind::RequestTimeout,
            Self::ETag(_) => MiddlewareKind::ETag,
            Self::Compression(_) => MiddlewareKind::Compression,
        }
    }

    /// Priority for ordering: lower values are applied first (outermost layer).
    pub(crate) fn priority(&self) -> u8 {
        match self {
            Self::TrustedProxy(_) => 0,
            Self::MaintenanceMode(_) => 1,
            Self::Cors(_) => 10,
            Self::SecurityHeaders(_) => 20,
            Self::Csrf(_) => 25,
            Self::RateLimit(_) => 30,
            Self::MaxBodySize(_) => 40,
            Self::RequestTimeout(_) => 50,
            Self::ETag(_) => 55,
            Self::Compression(_) => 60,
        }
    }

    /// Apply this middleware to the given router.
    pub(crate) fn apply(
        self,
        router: axum::Router<AppContext>,
        app: &AppContext,
    ) -> axum::Router<AppContext> {
        match self {
            Self::TrustedProxy(config) => config.apply(router, app),
            Self::MaintenanceMode(config) => config.apply(router, app),
            Self::Cors(config) => config.apply(router),
            Self::SecurityHeaders(config) => config.apply(router),
            Self::Csrf(config) => config.apply(router),
            Self::RateLimit(config) => config.apply(router, app),
            Self::MaxBodySize(config) => config.apply(router),
            Self::RequestTimeout(config) => config.apply(router),
            Self::ETag(config) => config.apply(router),
            Self::Compression(config) => config.apply(router),
        }
    }
}

// ---------------------------------------------------------------------------
// apply_ordered_middlewares
// ---------------------------------------------------------------------------

/// Sort middleware configs by priority (ascending) and apply them to the router.
///
/// Lower priority values wrap the router first, so they become the outermost
/// layers and run first on incoming requests.
pub(crate) fn apply_ordered_middlewares(
    mut router: axum::Router<AppContext>,
    mut middlewares: Vec<MiddlewareConfig>,
    app: &AppContext,
) -> axum::Router<AppContext> {
    middlewares.sort_by_key(|m| m.priority());
    for mw in middlewares {
        router = mw.apply(router, app);
    }
    router
}

pub(crate) fn configured_global_middlewares(
    config: &HttpConfig,
    explicit: &[MiddlewareConfig],
) -> Result<Vec<MiddlewareConfig>> {
    let mut middlewares = Vec::new();

    let has = |kind| explicit.iter().any(|middleware| middleware.kind() == kind);

    if config.trusted_proxy.enabled && !has(MiddlewareKind::TrustedProxy) {
        middlewares.push(TrustedProxy::from_config(&config.trusted_proxy)?.build());
    }

    if config.cors.enabled && !has(MiddlewareKind::Cors) {
        middlewares.push(Cors::from_config(&config.cors)?.build());
    }

    if config.csrf.enabled && !has(MiddlewareKind::Csrf) {
        middlewares.push(Csrf::from_config(&config.csrf)?.build());
    }

    if config.security_headers.enabled && !has(MiddlewareKind::SecurityHeaders) {
        middlewares.push(SecurityHeaders::from_config(&config.security_headers)?.build());
    }

    if config.rate_limit.enabled && !has(MiddlewareKind::RateLimit) {
        if matches!(config.rate_limit.by, HttpRateLimitByConfig::Actor) {
            tracing::warn!(
                "foundry: http.rate_limit.by = \"actor\" only applies after an authenticated actor is available; use route-level rate limits or actor_or_ip for global fallback"
            );
        }
        middlewares.push(RateLimit::from_config(&config.rate_limit)?.build());
    }

    if config.max_body_size_bytes > 0 && !has(MiddlewareKind::MaxBodySize) {
        middlewares.push(MaxBodySize::bytes(config.max_body_size_bytes).build());
    }

    if config.request_timeout_ms > 0 && !has(MiddlewareKind::RequestTimeout) {
        middlewares.push(RequestTimeout::millis(config.request_timeout_ms).build());
    }

    Ok(middlewares)
}

pub(crate) fn normalize_edge_rejection_response(response: Response) -> Response {
    if response.extensions().get::<HttpEdgeRejection>().is_some() {
        return response;
    }

    if response_is_json(&response) {
        return response;
    }

    let Some(rejection) = edge_rejection_from_status(response.status()) else {
        return response;
    };
    let status = response.status();
    let original_headers = response.headers().clone();
    let mut normalized = edge_error_response(status, rejection);
    for (name, value) in original_headers {
        let Some(name) = name else {
            continue;
        };
        if name == header::CONTENT_TYPE || name == header::CONTENT_LENGTH {
            continue;
        }
        normalized.headers_mut().insert(name, value);
    }
    normalized.extensions_mut().insert(rejection);
    normalized
}

pub(crate) fn edge_rejection_from_response(response: &Response) -> Option<HttpEdgeRejection> {
    response.extensions().get::<HttpEdgeRejection>().copied()
}

fn response_is_json(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"))
}

fn edge_rejection_from_status(status: StatusCode) -> Option<HttpEdgeRejection> {
    match status {
        StatusCode::PAYLOAD_TOO_LARGE => Some(HttpEdgeRejection::PayloadTooLarge),
        StatusCode::REQUEST_TIMEOUT => Some(HttpEdgeRejection::Timeout),
        StatusCode::TOO_MANY_REQUESTS => Some(HttpEdgeRejection::RateLimited),
        _ => None,
    }
}

fn edge_error_response(status: StatusCode, rejection: HttpEdgeRejection) -> Response {
    let message = match rejection {
        HttpEdgeRejection::RateLimited => "Rate limit exceeded",
        HttpEdgeRejection::PayloadTooLarge => "Payload too large",
        HttpEdgeRejection::Timeout => "Request timed out",
        HttpEdgeRejection::Cors => "CORS request rejected",
    };

    (
        status,
        axum::Json(serde_json::json!({
            "message": message,
            "status": status.as_u16(),
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Cors
// ---------------------------------------------------------------------------

/// CORS middleware configuration.
///
/// Wraps `tower_http::cors::CorsLayer` with a builder API.
///
/// ```
/// use foundry::http::middleware::Cors;
///
/// let cors = Cors::new()
///     .allow_any_origin()
///     .allow_any_method()
///     .allow_headers([axum::http::header::CONTENT_TYPE]);
/// ```
#[derive(Clone, Debug)]
pub struct Cors {
    origins: CorsOrigins,
    methods: CorsMethods,
    headers: CorsHeaders,
    credentials: bool,
    max_age: Option<Duration>,
}

#[derive(Clone, Debug)]
enum CorsOrigins {
    None,
    Any,
    List(Vec<String>),
}

#[derive(Clone, Debug)]
enum CorsMethods {
    None,
    Any,
    List(Vec<Method>),
}

#[derive(Clone, Debug)]
enum CorsHeaders {
    None,
    Any,
    List(Vec<HeaderName>),
}

impl Cors {
    /// Create a new CORS configuration with no origins, methods, or headers allowed.
    pub fn new() -> Self {
        Self {
            origins: CorsOrigins::None,
            methods: CorsMethods::None,
            headers: CorsHeaders::None,
            credentials: false,
            max_age: None,
        }
    }

    /// Allow a single origin.
    pub fn allow_origin(mut self, origin: &str) -> Self {
        self.origins = CorsOrigins::List(vec![origin.to_string()]);
        self
    }

    /// Allow multiple origins.
    pub fn allow_origins<I, O>(mut self, origins: I) -> Self
    where
        I: IntoIterator<Item = O>,
        O: AsRef<str>,
    {
        self.origins = CorsOrigins::List(
            origins
                .into_iter()
                .map(|o| o.as_ref().to_string())
                .collect(),
        );
        self
    }

    /// Allow any origin.
    pub fn allow_any_origin(mut self) -> Self {
        self.origins = CorsOrigins::Any;
        self
    }

    /// Allow a single HTTP method.
    pub fn allow_method(mut self, method: Method) -> Self {
        self.methods = CorsMethods::List(vec![method]);
        self
    }

    /// Allow multiple HTTP methods.
    pub fn allow_methods<I>(mut self, methods: I) -> Self
    where
        I: IntoIterator<Item = Method>,
    {
        self.methods = CorsMethods::List(methods.into_iter().collect());
        self
    }

    /// Allow any HTTP method.
    pub fn allow_any_method(mut self) -> Self {
        self.methods = CorsMethods::Any;
        self
    }

    /// Allow a single request header.
    pub fn allow_header(mut self, hdr: HeaderName) -> Self {
        self.headers = CorsHeaders::List(vec![hdr]);
        self
    }

    /// Allow multiple request headers.
    pub fn allow_headers<I>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = HeaderName>,
    {
        self.headers = CorsHeaders::List(headers.into_iter().collect());
        self
    }

    /// Allow any request header.
    pub fn allow_any_header(mut self) -> Self {
        self.headers = CorsHeaders::Any;
        self
    }

    /// Include `Access-Control-Allow-Credentials: true`.
    pub fn allow_credentials(mut self) -> Self {
        self.credentials = true;
        self
    }

    /// Set `Access-Control-Max-Age` in seconds.
    pub fn max_age(mut self, seconds: u64) -> Self {
        self.max_age = Some(Duration::from_secs(seconds));
        self
    }

    pub(crate) fn from_config(config: &HttpCorsConfig) -> Result<Self> {
        let mut cors = Self::new();

        if config.allowed_origins.iter().any(|origin| origin == "*") {
            if config.allow_credentials {
                return Err(Error::message(
                    "http.cors.allow_credentials cannot be true when allowed_origins contains `*`",
                ));
            }
            cors = cors.allow_any_origin();
        } else if !config.allowed_origins.is_empty() {
            for origin in &config.allowed_origins {
                HeaderValue::from_str(origin).map_err(|error| {
                    Error::message(format!(
                        "invalid http.cors.allowed_origins value `{origin}`: {error}"
                    ))
                })?;
            }
            cors = cors.allow_origins(&config.allowed_origins);
        }

        let mut methods = Vec::with_capacity(config.allowed_methods.len());
        for method in &config.allowed_methods {
            methods.push(Method::from_bytes(method.as_bytes()).map_err(|error| {
                Error::message(format!(
                    "invalid http.cors.allowed_methods value `{method}`: {error}"
                ))
            })?);
        }
        if !methods.is_empty() {
            cors = cors.allow_methods(methods);
        }

        let mut headers = Vec::with_capacity(config.allowed_headers.len());
        for header in &config.allowed_headers {
            headers.push(HeaderName::from_str(header).map_err(|error| {
                Error::message(format!(
                    "invalid http.cors.allowed_headers value `{header}`: {error}"
                ))
            })?);
        }
        if !headers.is_empty() {
            cors = cors.allow_headers(headers);
        }

        if config.allow_credentials {
            cors = cors.allow_credentials();
        }
        if config.max_age_seconds > 0 {
            cors = cors.max_age(config.max_age_seconds);
        }

        Ok(cors)
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::Cors(self)
    }

    fn apply(self, router: axum::Router<AppContext>) -> axum::Router<AppContext> {
        let mut layer = CorsLayer::new();

        layer = match self.origins {
            CorsOrigins::None => layer,
            CorsOrigins::Any => layer.allow_origin(tower_http::cors::Any),
            CorsOrigins::List(ref origins) if origins.len() == 1 => {
                if let Ok(value) = HeaderValue::from_str(&origins[0]) {
                    layer.allow_origin(value)
                } else {
                    tracing::warn!(
                        origin = %origins[0],
                        "foundry: skipping invalid CORS origin"
                    );
                    layer
                }
            }
            CorsOrigins::List(ref origins) => {
                let values: Vec<HeaderValue> = origins
                    .iter()
                    .filter_map(|origin| match HeaderValue::from_str(origin) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            tracing::warn!(
                                origin = %origin,
                                error = %error,
                                "foundry: skipping invalid CORS origin"
                            );
                            None
                        }
                    })
                    .collect();
                layer.allow_origin(values)
            }
        };

        layer = match self.methods {
            CorsMethods::None => layer,
            CorsMethods::Any => layer.allow_methods(tower_http::cors::Any),
            CorsMethods::List(methods) => layer.allow_methods(methods),
        };

        layer = match self.headers {
            CorsHeaders::None => layer,
            CorsHeaders::Any => layer.allow_headers(tower_http::cors::Any),
            CorsHeaders::List(headers) => layer.allow_headers(headers),
        };

        if self.credentials {
            layer = layer.allow_credentials(true);
        }

        if let Some(duration) = self.max_age {
            layer = layer.max_age(duration);
        }

        router
            .layer(layer)
            .layer(middleware::from_fn(cors_edge_rejection_marker))
    }
}

async fn cors_edge_rejection_marker(request: Request, next: Next) -> Response {
    let is_cors_request = request.headers().contains_key(header::ORIGIN)
        || request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    let mut response = next.run(request).await;
    if is_cors_request
        && matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN
        )
    {
        response.extensions_mut().insert(HttpEdgeRejection::Cors);
    }
    response
}

impl Default for Cors {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SecurityHeaders
// ---------------------------------------------------------------------------

const HSTS_HEADER: HeaderName = header::STRICT_TRANSPORT_SECURITY;
const X_CONTENT_TYPE_OPTIONS: HeaderName = header::X_CONTENT_TYPE_OPTIONS;
const X_FRAME_OPTIONS: HeaderName = header::X_FRAME_OPTIONS;
const REFERRER_POLICY: HeaderName = header::REFERRER_POLICY;
const X_XSS_PROTECTION: HeaderName = HeaderName::from_static("x-xss-protection");

/// Security headers middleware.
///
/// Adds security-related headers to every response. All defaults are applied
/// on construction and can be customised via builder methods.
///
/// Default headers:
/// - `X-Content-Type-Options: nosniff`
/// - `X-Frame-Options: DENY`
/// - `Strict-Transport-Security: max-age=31536000; includeSubDomains`
/// - `Referrer-Policy: strict-origin-when-cross-origin`
/// - `X-XSS-Protection: 0`
#[derive(Clone, Debug)]
pub struct SecurityHeaders {
    headers: Vec<(HeaderName, HeaderValue)>,
}

impl SecurityHeaders {
    /// Create with all default security headers.
    pub fn new() -> Self {
        Self {
            headers: vec![
                (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
                (X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
                (
                    HSTS_HEADER,
                    HeaderValue::from_static("max-age=31536000; includeSubDomains"),
                ),
                (
                    REFERRER_POLICY,
                    HeaderValue::from_static("strict-origin-when-cross-origin"),
                ),
                (X_XSS_PROTECTION, HeaderValue::from_static("0")),
            ],
        }
    }

    /// Disable the `Strict-Transport-Security` header.
    pub fn disable_hsts(mut self) -> Self {
        self.headers.retain(|(name, _)| *name != HSTS_HEADER);
        self
    }

    pub(crate) fn from_config(config: &HttpSecurityHeadersConfig) -> Result<Self> {
        let mut headers = Self::new();
        if !config.hsts {
            headers = headers.disable_hsts();
        }
        if !config.frame_options.trim().is_empty() {
            HeaderValue::from_str(&config.frame_options).map_err(|error| {
                Error::message(format!(
                    "invalid http.security_headers.frame_options: {error}"
                ))
            })?;
            headers = headers.frame_options(&config.frame_options);
        }
        if !config.referrer_policy.trim().is_empty() {
            HeaderValue::from_str(&config.referrer_policy).map_err(|error| {
                Error::message(format!(
                    "invalid http.security_headers.referrer_policy: {error}"
                ))
            })?;
            headers = headers.referrer_policy(&config.referrer_policy);
        }
        if !config.content_security_policy.trim().is_empty() {
            HeaderValue::from_str(&config.content_security_policy).map_err(|error| {
                Error::message(format!(
                    "invalid http.security_headers.content_security_policy: {error}"
                ))
            })?;
            headers = headers.content_security_policy(&config.content_security_policy);
        }
        Ok(headers)
    }

    /// Set the `X-Frame-Options` value.
    pub fn frame_options(mut self, value: &str) -> Self {
        if let Ok(hv) = HeaderValue::from_str(value) {
            if let Some(entry) = self.headers.iter_mut().find(|(n, _)| *n == X_FRAME_OPTIONS) {
                entry.1 = hv;
            }
        }
        self
    }

    /// Add a `Content-Security-Policy` header. Invalid values are silently skipped.
    pub fn content_security_policy(mut self, policy: &str) -> Self {
        if let Ok(hv) = HeaderValue::from_str(policy) {
            self = self.header(header::CONTENT_SECURITY_POLICY, hv);
        }
        self
    }

    /// Set the `Referrer-Policy` value.
    pub fn referrer_policy(mut self, policy: &str) -> Self {
        if let Ok(hv) = HeaderValue::from_str(policy) {
            if let Some(entry) = self.headers.iter_mut().find(|(n, _)| *n == REFERRER_POLICY) {
                entry.1 = hv;
            }
        }
        self
    }

    /// Add a custom header to every response.
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.push((name, value));
        self
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::SecurityHeaders(self)
    }

    fn apply(self, router: axum::Router<AppContext>) -> axum::Router<AppContext> {
        let headers = self.headers;
        router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let headers = headers.clone();
            async move { security_headers_fn(request, next, &headers).await }
        }))
    }
}

async fn security_headers_fn(
    request: Request,
    next: Next,
    headers: &[(HeaderName, HeaderValue)],
) -> Response {
    let mut response = next.run(request).await;
    let response_headers = response.headers_mut();
    for (name, value) in headers {
        response_headers.insert(name.clone(), value.clone());
    }
    response
}

impl Default for SecurityHeaders {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Csrf
// ---------------------------------------------------------------------------

/// CSRF protection middleware using the double-submit cookie pattern.
///
/// Generates a token stored in a cookie and validates that state-changing
/// requests (POST/PUT/PATCH/DELETE) include the matching token in a header
/// or form field. GET/HEAD/OPTIONS requests are exempt.
#[derive(Clone, Debug)]
pub struct Csrf {
    cookie_name: String,
    header_name: HeaderName,
    secure: bool,
    path: String,
    same_site: String,
    exclude: Vec<String>,
}

impl Csrf {
    pub fn new() -> Self {
        Self {
            cookie_name: "foundry_csrf".to_string(),
            header_name: HeaderName::from_static("x-csrf-token"),
            secure: true,
            path: "/".to_string(),
            same_site: "lax".to_string(),
            exclude: Vec::new(),
        }
    }

    pub fn from_config(config: &HttpCsrfConfig) -> Result<Self> {
        let header_name =
            HeaderName::from_bytes(config.header_name.as_bytes()).map_err(|error| {
                Error::message(format!(
                    "invalid http.csrf.header_name value `{}`: {error}",
                    config.header_name
                ))
            })?;
        let same_site = super::cookie::parse_same_site(&config.cookie_same_site)?;
        if matches!(same_site, super::cookie::SameSite::None) && !config.cookie_secure {
            return Err(Error::message(
                "http.csrf.cookie_same_site = \"none\" requires cookie_secure = true",
            ));
        }
        let csrf = Self::new()
            .cookie_name(&config.cookie_name)
            .header_name(header_name)
            .secure(config.cookie_secure)
            .path(&config.cookie_path)
            .same_site(&config.cookie_same_site)
            .exclude_paths(config.exclude_paths.iter().map(String::as_str));

        super::cookie::build_cookie_header_value(super::cookie::CookieHeaderOptions {
            name: &csrf.cookie_name,
            value: "probe",
            http_only: false,
            secure: csrf.secure,
            path: &csrf.path,
            same_site,
            domain: None,
            max_age_secs: None,
        })?;

        Ok(csrf)
    }

    pub fn cookie_name(mut self, name: &str) -> Self {
        self.cookie_name = name.to_string();
        self
    }

    pub fn header_name(mut self, name: HeaderName) -> Self {
        self.header_name = name;
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn path(mut self, path: &str) -> Self {
        self.path = path.to_string();
        self
    }

    pub fn same_site(mut self, same_site: &str) -> Self {
        self.same_site = same_site.to_string();
        self
    }

    /// Add a path prefix to exclude from CSRF validation (e.g., "/api").
    pub fn exclude(mut self, path: &str) -> Self {
        self.exclude.push(path.to_string());
        self
    }

    pub fn exclude_paths<'a, I>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.exclude.extend(paths.into_iter().map(str::to_string));
        self
    }

    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::Csrf(self)
    }

    fn apply(self, router: axum::Router<AppContext>) -> axum::Router<AppContext> {
        let state = CsrfState {
            cookie_name: self.cookie_name,
            header_name: self.header_name,
            secure: self.secure,
            path: self.path,
            same_site: self.same_site,
            exclude: self.exclude,
        };
        router.layer(middleware::from_fn_with_state(state, csrf_middleware))
    }
}

impl Default for Csrf {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct CsrfState {
    cookie_name: String,
    header_name: HeaderName,
    secure: bool,
    path: String,
    same_site: String,
    exclude: Vec<String>,
}

/// Extension inserted by the CSRF middleware containing the current token.
#[derive(Clone, Debug)]
pub struct CsrfToken(String);

impl CsrfToken {
    /// The CSRF token value (for rendering in forms or meta tags).
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl<S> axum::extract::FromRequestParts<S> for CsrfToken
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        parts.extensions.get::<CsrfToken>().cloned().ok_or_else(|| {
            Error::message("CSRF middleware not active on this route").into_response()
        })
    }
}

async fn csrf_middleware(
    State(state): State<CsrfState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    // Check if path is excluded
    if state
        .exclude
        .iter()
        .any(|prefix| path_matches_csrf_exclusion(&path, prefix))
    {
        return next.run(request).await;
    }

    let method = request.method().clone();
    let is_safe = matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);

    // Extract existing token from cookie
    let existing_token = extract_cookie_value(request.headers(), &state.cookie_name);

    if is_safe {
        // Safe methods: ensure token cookie exists, set extension
        let token = match existing_token {
            Some(ref token) if is_valid_csrf_token(token) => token.clone(),
            Some(_) => {
                return csrf_forbidden("CSRF token malformed");
            }
            None => {
                // Generate new token
                match crate::support::Token::base64(32) {
                    Ok(t) => t,
                    Err(_) => {
                        return Error::message("Failed to generate CSRF token").into_response();
                    }
                }
            }
        };

        request.extensions_mut().insert(CsrfToken(token.clone()));
        let mut response = next.run(request).await;

        // Set cookie if it wasn't present
        if existing_token.is_none() {
            match build_csrf_cookie(&state.cookie_name, &token, &state) {
                Ok(hv) => {
                    response.headers_mut().append(header::SET_COOKIE, hv);
                }
                Err(error) => {
                    return error.into_response();
                }
            }
        }

        response
    } else {
        // State-changing methods: validate token
        let Some(cookie_token) = existing_token else {
            return csrf_forbidden("CSRF token cookie missing");
        };
        if !is_valid_csrf_token(&cookie_token) {
            return csrf_forbidden("CSRF token malformed");
        }

        // Check header first, then query for the form field value won't work easily
        // without consuming the body. For API-first framework, header is primary.
        let request_token = request
            .headers()
            .get(&state.header_name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let Some(request_token) = request_token else {
            return csrf_forbidden("CSRF token missing from request header");
        };
        if !is_valid_csrf_token(&request_token) {
            return csrf_forbidden("CSRF token malformed");
        }

        if !crate::support::hmac::constant_time_eq(
            cookie_token.as_bytes(),
            request_token.as_bytes(),
        ) {
            return csrf_forbidden("CSRF token mismatch");
        }

        request.extensions_mut().insert(CsrfToken(cookie_token));
        next.run(request).await
    }
}

/// Build a CSRF cookie string. Intentionally NOT HttpOnly — the frontend JS must
/// read this cookie to include the token in the X-CSRF-TOKEN request header
/// (double-submit cookie pattern).
fn build_csrf_cookie(name: &str, value: &str, state: &CsrfState) -> Result<HeaderValue> {
    let same_site = super::cookie::parse_same_site(&state.same_site)?;
    super::cookie::build_cookie_header_value(super::cookie::CookieHeaderOptions {
        name,
        value,
        http_only: false,
        secure: state.secure,
        path: &state.path,
        same_site,
        domain: None,
        max_age_secs: None,
    })
}

fn csrf_forbidden(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(serde_json::json!({
            "message": message,
            "status": 403
        })),
    )
        .into_response()
}

fn path_matches_csrf_exclusion(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return false;
    }
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn is_valid_csrf_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

// extract_cookie_value is in crate::http::cookie (shared with session auth)
use super::cookie::extract_cookie_value;

// ---------------------------------------------------------------------------
// RateLimit
// ---------------------------------------------------------------------------

/// The time window for rate limiting.
#[derive(Clone, Copy, Debug)]
pub enum RateLimitWindow {
    Second,
    Minute,
    Hour,
}

impl RateLimitWindow {
    fn duration_secs(&self) -> u64 {
        match self {
            Self::Second => 1,
            Self::Minute => 60,
            Self::Hour => 3600,
        }
    }
}

/// Determines how rate-limit keys are derived.
#[derive(Clone, Copy, Debug, Default)]
pub enum RateLimitBy {
    /// Key by client IP address (default).
    #[default]
    Ip,
    /// Key by authenticated actor ID (requires auth middleware).
    Actor,
    /// Key by actor ID when authenticated, falling back to IP.
    ActorOrIp,
}

/// Rate-limit store backend.
#[derive(Clone)]
pub(crate) enum RateLimitStore {
    /// In-memory fixed-window counter. Used when Redis is not configured.
    Memory(Arc<Mutex<HashMap<String, (u32, u64)>>>),
    /// Redis-backed counter via `INCR` + `EXPIRE`. Used automatically when
    /// the runtime backend is Redis.
    Redis(RuntimeBackend),
}

/// Fixed-window rate limiter with Redis-backed storage.
///
/// Uses Redis automatically when configured, falls back to in-memory storage
/// for development and testing.
///
/// ```
/// use foundry::http::middleware::RateLimit;
///
/// let limiter = RateLimit::new(100)
///     .per_minute()
///     .key_prefix("my_api:");
/// ```
#[derive(Clone, Debug)]
pub struct RateLimit {
    max: u32,
    window: RateLimitWindow,
    window_secs: u64,
    key_prefix: String,
    by: RateLimitBy,
}

impl RateLimit {
    /// Create a rate limiter allowing `max` requests per minute (default window).
    pub fn new(max: u32) -> Self {
        Self {
            max,
            window: RateLimitWindow::Minute,
            window_secs: RateLimitWindow::Minute.duration_secs(),
            key_prefix: "rl:".to_string(),
            by: RateLimitBy::Ip,
        }
    }

    /// Use a per-second window.
    pub fn per_second(mut self) -> Self {
        self.window = RateLimitWindow::Second;
        self.window_secs = self.window.duration_secs();
        self
    }

    /// Use a per-minute window.
    pub fn per_minute(mut self) -> Self {
        self.window = RateLimitWindow::Minute;
        self.window_secs = self.window.duration_secs();
        self
    }

    /// Use a per-hour window.
    pub fn per_hour(mut self) -> Self {
        self.window = RateLimitWindow::Hour;
        self.window_secs = self.window.duration_secs();
        self
    }

    pub(crate) fn from_config(config: &HttpRateLimitConfig) -> Result<Self> {
        if config.max_requests == 0 {
            return Err(Error::message(
                "http.rate_limit.max_requests must be greater than 0 when rate limiting is enabled",
            ));
        }
        if config.window_seconds == 0 {
            return Err(Error::message(
                "http.rate_limit.window_seconds must be greater than 0 when rate limiting is enabled",
            ));
        }

        let window = match config.window_seconds {
            1 => RateLimitWindow::Second,
            3600 => RateLimitWindow::Hour,
            _ => RateLimitWindow::Minute,
        };
        let by = match config.by {
            HttpRateLimitByConfig::Ip => RateLimitBy::Ip,
            HttpRateLimitByConfig::Actor => RateLimitBy::Actor,
            HttpRateLimitByConfig::ActorOrIp => RateLimitBy::ActorOrIp,
        };

        Ok(Self {
            max: config.max_requests,
            window,
            window_secs: config.window_seconds,
            key_prefix: config.key_prefix.clone(),
            by,
        })
    }

    /// Set a custom key prefix for the rate-limit counter.
    pub fn key_prefix(mut self, prefix: &str) -> Self {
        self.key_prefix = prefix.to_string();
        self
    }

    /// Rate-limit by authenticated actor ID instead of IP.
    pub fn by_actor(mut self) -> Self {
        self.by = RateLimitBy::Actor;
        self
    }

    /// Rate-limit by actor ID when authenticated, falling back to IP.
    pub fn by_actor_or_ip(mut self) -> Self {
        self.by = RateLimitBy::ActorOrIp;
        self
    }

    /// Returns the configured rate-limit key strategy.
    pub fn rate_limit_by(&self) -> RateLimitBy {
        self.by
    }

    /// Returns the maximum requests allowed per window.
    pub fn max(&self) -> u32 {
        self.max
    }

    /// Returns the configured window.
    pub fn window(&self) -> RateLimitWindow {
        self.window
    }

    pub(crate) fn window_secs(&self) -> u64 {
        self.window_secs
    }

    /// Returns the key prefix.
    pub fn key_prefix_str(&self) -> &str {
        &self.key_prefix
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::RateLimit(self)
    }

    fn apply(self, router: axum::Router<AppContext>, app: &AppContext) -> axum::Router<AppContext> {
        let store = create_rate_limit_store(app);
        let state = RateLimitState {
            max: self.max,
            window_secs: self.window_secs,
            key_prefix: self.key_prefix,
            by: self.by,
            store,
        };
        router.layer(middleware::from_fn_with_state(state, rate_limit_middleware))
    }
}

/// Create a rate-limit store backend from the application context.
///
/// Uses Redis when configured, otherwise falls back to in-memory storage.
pub(crate) fn create_rate_limit_store(app: &AppContext) -> RateLimitStore {
    match app.resolve::<RuntimeBackend>() {
        Ok(backend) if matches!(backend.kind(), RuntimeBackendKind::Redis) => {
            tracing::debug!("foundry: rate limiter using Redis backend");
            RateLimitStore::Redis((*backend).clone())
        }
        _ => RateLimitStore::Memory(Arc::new(Mutex::new(HashMap::new()))),
    }
}

#[derive(Clone)]
pub(crate) struct RateLimitState {
    pub(crate) max: u32,
    pub(crate) window_secs: u64,
    pub(crate) key_prefix: String,
    pub(crate) by: RateLimitBy,
    pub(crate) store: RateLimitStore,
}

async fn rate_limit_middleware(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(key_identifier) = rate_limit_key_from_request(&state, &request) else {
        return next.run(request).await;
    };
    let info = rate_limit_info(&state, &key_identifier).await;

    if info.current > info.limit {
        return rate_limit_response(&info);
    }

    let mut response = next.run(request).await;
    let resp_headers = response.headers_mut();
    let _ = resp_headers.try_insert(
        HeaderName::from_static("x-ratelimit-limit"),
        rate_limit_header_value(info.limit),
    );
    let _ = resp_headers.try_insert(
        HeaderName::from_static("x-ratelimit-remaining"),
        rate_limit_header_value(info.remaining),
    );
    let _ = resp_headers.try_insert(
        HeaderName::from_static("x-ratelimit-reset"),
        rate_limit_header_value(info.secs_until_reset),
    );
    response
}

fn rate_limit_key_from_request(state: &RateLimitState, request: &Request) -> Option<String> {
    match state.by {
        RateLimitBy::Actor => request
            .extensions()
            .get::<crate::auth::Actor>()
            .map(|actor| format!("actor:{}", actor.id)),
        RateLimitBy::ActorOrIp => Some(
            request
                .extensions()
                .get::<crate::auth::Actor>()
                .map(|actor| format!("actor:{}", actor.id))
                .unwrap_or_else(|| format!("ip:{}", extract_client_ip(request))),
        ),
        RateLimitBy::Ip => Some(format!("ip:{}", extract_client_ip(request))),
    }
}

struct RateLimitInfo {
    current: u32,
    remaining: u32,
    limit: u32,
    secs_until_reset: u64,
}

fn rate_limit_header_value(value: impl ToString) -> HeaderValue {
    HeaderValue::from_str(&value.to_string()).unwrap_or_else(|_| HeaderValue::from_static("0"))
}

/// Increment the rate-limit counter for the given key and return current info.
async fn rate_limit_info(state: &RateLimitState, key_identifier: &str) -> RateLimitInfo {
    let window_secs = state.window_secs.max(1);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bucket = now_secs / window_secs;
    let key = format!("{}{}:{}", state.key_prefix, key_identifier, bucket);

    let (current, secs_until_reset) = match &state.store {
        RateLimitStore::Redis(backend) => {
            let count = match backend.incr_with_ttl(&key, window_secs).await {
                Ok(c) => c as u32,
                Err(_) => {
                    tracing::warn!("foundry: redis rate limit error, allowing request");
                    1
                }
            };
            let secs_until_reset = (bucket + 1) * window_secs - now_secs;
            (count, secs_until_reset)
        }
        RateLimitStore::Memory(store) => {
            let window_end_secs = (bucket + 1) * window_secs;
            let mut store = store.lock().await;
            let entry = store.entry(key).or_insert((0, window_end_secs));

            if now_secs >= entry.1 {
                *entry = (0, window_end_secs);
            }

            entry.0 += 1;
            let count = entry.0;

            if store.len() > 10_000 {
                store.retain(|_, (_, expires_at)| now_secs < *expires_at);
            }

            (count, window_end_secs.saturating_sub(now_secs))
        }
    };

    RateLimitInfo {
        current,
        remaining: state.max.saturating_sub(current),
        limit: state.max,
        secs_until_reset,
    }
}

/// Check the rate limit for a given key identifier and return a 429 response if exceeded.
///
/// Returns `Some(Response)` with a 429 status if the limit is exceeded, `None` otherwise.
/// The response includes standard rate-limit headers.
pub(crate) async fn enforce_rate_limit(
    state: &RateLimitState,
    key_identifier: &str,
) -> Option<Response> {
    let info = rate_limit_info(state, key_identifier).await;

    if info.current > info.limit {
        return Some(rate_limit_response(&info));
    }

    None
}

pub(crate) async fn enforce_rate_limit_for_actor(
    state: &RateLimitState,
    actor: &crate::auth::Actor,
    client_ip: IpAddr,
) -> Option<Response> {
    let key_identifier = match state.by {
        RateLimitBy::Ip => format!("ip:{client_ip}"),
        RateLimitBy::Actor | RateLimitBy::ActorOrIp => format!("actor:{}", actor.id),
    };

    enforce_rate_limit(state, &key_identifier).await
}

fn rate_limit_response(info: &RateLimitInfo) -> Response {
    let body = serde_json::json!({
        "message": "Rate limit exceeded",
        "status": 429
    });

    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        [
            (
                HeaderName::from_static("x-ratelimit-limit"),
                rate_limit_header_value(info.limit),
            ),
            (
                HeaderName::from_static("x-ratelimit-remaining"),
                HeaderValue::from_static("0"),
            ),
            (
                HeaderName::from_static("x-ratelimit-reset"),
                rate_limit_header_value(info.secs_until_reset),
            ),
            (
                header::RETRY_AFTER,
                rate_limit_header_value(info.secs_until_reset),
            ),
        ],
        axum::Json(body),
    )
        .into_response();
    response
        .extensions_mut()
        .insert(HttpEdgeRejection::RateLimited);
    response
}

pub(crate) fn extract_client_ip(request: &Request) -> IpAddr {
    // Prefer RealIp set by TrustedProxy middleware
    if let Some(RealIp(ip)) = request.extensions().get::<RealIp>() {
        return *ip;
    }
    // Fall back to connect info
    if let Some(addr) = request.extensions().get::<ConnectInfoAddr>() {
        return addr.0.ip();
    }
    if let Some(ConnectInfo(addr)) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip();
    }
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

/// Helper type used to inject a connect-info address in tests.
#[derive(Clone, Debug)]
pub(crate) struct ConnectInfoAddr(pub SocketAddr);

// ---------------------------------------------------------------------------
// MaxBodySize
// ---------------------------------------------------------------------------

/// Request body size limit middleware.
///
/// Wraps `tower_http::limit::RequestBodyLimitLayer` and keeps Axum's
/// extractor body limit in sync.
#[derive(Clone, Debug)]
pub struct MaxBodySize(usize);

impl MaxBodySize {
    /// Limit to `n` bytes.
    pub fn bytes(n: usize) -> Self {
        Self(n)
    }

    /// Limit to `n` kilobytes.
    pub fn kb(n: usize) -> Self {
        Self(n * 1024)
    }

    /// Limit to `n` megabytes.
    pub fn mb(n: usize) -> Self {
        Self(n * 1024 * 1024)
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::MaxBodySize(self)
    }

    fn apply(self, router: axum::Router<AppContext>) -> axum::Router<AppContext> {
        router
            .layer(DefaultBodyLimit::max(self.0))
            .layer(RequestBodyLimitLayer::new(self.0))
    }
}

// ---------------------------------------------------------------------------
// RequestTimeout
// ---------------------------------------------------------------------------

/// Request timeout middleware.
///
/// Wraps `tower_http::timeout::TimeoutLayer`.
#[derive(Clone, Debug)]
pub struct RequestTimeout(Duration);

impl RequestTimeout {
    /// Timeout after `n` milliseconds.
    pub fn millis(n: u64) -> Self {
        Self(Duration::from_millis(n))
    }

    /// Timeout after `n` seconds.
    pub fn secs(n: u64) -> Self {
        Self(Duration::from_secs(n))
    }

    /// Timeout after `n` minutes.
    pub fn mins(n: u64) -> Self {
        Self(Duration::from_secs(n * 60))
    }

    /// Timeout after the given duration.
    pub fn duration(d: Duration) -> Self {
        Self(d)
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::RequestTimeout(self)
    }

    fn apply(self, router: axum::Router<AppContext>) -> axum::Router<AppContext> {
        router.layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            self.0,
        ))
    }
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

/// Response compression middleware (gzip + brotli).
///
/// Wraps `tower_http::compression::CompressionLayer`.
#[derive(Clone, Debug)]
pub struct Compression;

impl Compression {
    pub fn new() -> Self {
        Self
    }

    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::Compression(self)
    }

    fn apply(self, router: axum::Router<AppContext>) -> axum::Router<AppContext> {
        router.layer(tower_http::compression::CompressionLayer::new())
    }
}

impl Default for Compression {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MaintenanceMode
// ---------------------------------------------------------------------------

/// Maintenance mode middleware.
///
/// When active, returns `503 Service Unavailable` for all requests unless
/// a valid bypass secret is supplied via the `X-Maintenance-Bypass` header.
///
/// Maintenance state is stored in the runtime backend (`maintenance:active` key),
/// so it works across multiple instances in a distributed setup.
///
/// ```
/// use foundry::http::middleware::MaintenanceMode;
///
/// let mw = MaintenanceMode::new()
///     .bypass_secret("my-secret-token")
///     .build();
/// ```
#[derive(Clone, Debug)]
pub struct MaintenanceMode {
    bypass_secret: Option<String>,
    allow_query_bypass: bool,
}

impl MaintenanceMode {
    pub fn new() -> Self {
        Self {
            bypass_secret: None,
            allow_query_bypass: false,
        }
    }

    /// Set a secret that allows bypassing maintenance mode.
    ///
    /// Requests can bypass maintenance by providing the secret via the
    /// `X-Maintenance-Bypass` header.
    pub fn bypass_secret(mut self, secret: impl Into<String>) -> Self {
        self.bypass_secret = Some(secret.into());
        self
    }

    /// Allow the legacy `?bypass=...` query parameter bypass.
    ///
    /// Header bypass remains the default because query strings are commonly
    /// logged by proxies, access logs, and analytics systems.
    pub fn allow_query_bypass(mut self) -> Self {
        self.allow_query_bypass = true;
        self
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::MaintenanceMode(self)
    }

    fn apply(self, router: axum::Router<AppContext>, app: &AppContext) -> axum::Router<AppContext> {
        router.layer(middleware::from_fn_with_state(
            MaintenanceState {
                app: app.clone(),
                bypass_secret: self.bypass_secret,
                allow_query_bypass: self.allow_query_bypass,
            },
            maintenance_middleware,
        ))
    }
}

impl Default for MaintenanceMode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct MaintenanceState {
    app: AppContext,
    bypass_secret: Option<String>,
    allow_query_bypass: bool,
}

async fn maintenance_middleware(
    State(state): State<MaintenanceState>,
    request: Request,
    next: Next,
) -> Response {
    // Check if maintenance mode is active and get the stored bypass secret
    let stored_secret = match state
        .app
        .resolve::<crate::support::runtime::RuntimeBackend>()
    {
        Ok(backend) => backend
            .get_value("maintenance:active")
            .await
            .unwrap_or(None),
        Err(_) => None,
    };

    // Not in maintenance mode
    if stored_secret.is_none() {
        return next.run(request).await;
    }

    // Resolve bypass secret: prefer the value stored by `foundry down --secret=...`,
    // fall back to the middleware-configured secret
    let bypass_secret = stored_secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(state.bypass_secret.as_deref());

    if let Some(secret) = bypass_secret {
        // Check X-Maintenance-Bypass header
        if let Some(header_value) = request.headers().get("x-maintenance-bypass") {
            if let Ok(value) = header_value.to_str() {
                if crate::support::hmac::constant_time_eq(value.as_bytes(), secret.as_bytes()) {
                    return next.run(request).await;
                }
            }
        }

        // Check legacy bypass query parameter when explicitly enabled.
        if state.allow_query_bypass {
            if let Some(query) = request.uri().query() {
                for param in query.split('&') {
                    if let Some(value) = param.strip_prefix("bypass=") {
                        if crate::support::hmac::constant_time_eq(
                            value.as_bytes(),
                            secret.as_bytes(),
                        ) {
                            return next.run(request).await;
                        }
                    }
                }
            }
        }
    }

    // Return 503 Service Unavailable
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({
            "message": "Service is undergoing maintenance",
            "status": 503,
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// TrustedProxy
// ---------------------------------------------------------------------------

const CF_CONNECTING_IP: &str = "cf-connecting-ip";
const X_REAL_IP: &str = "x-real-ip";
const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Trusted proxy middleware.
///
/// Resolves the real client IP from proxy headers. Headers are checked in the
/// configured priority order.
///
/// The resolved IP is stored as a [`RealIp`] extension.
#[derive(Clone, Debug)]
pub struct TrustedProxy {
    headers: Vec<HeaderName>,
    trusted_cidrs: Vec<IpCidr>,
    trust_all: bool,
}

impl TrustedProxy {
    /// Create with default header priority (CF-Connecting-IP, X-Real-IP, X-Forwarded-For)
    /// and Cloudflare proxy CIDR trust.
    ///
    /// Only Cloudflare proxy peers are trusted by default. Configure additional
    /// trusted CIDRs with [`Self::trusted_cidr`] or explicitly opt in to
    /// trusting all proxy peers with [`Self::trust_all`].
    pub fn new() -> Self {
        Self {
            headers: default_proxy_headers(),
            trusted_cidrs: default_trusted_proxy_cidrs(),
            trust_all: false,
        }
    }

    /// Alias for `new()` — documents Cloudflare header and CIDR support.
    pub fn cloudflare() -> Self {
        Self::new()
    }

    /// Trust proxy headers from any peer.
    ///
    /// This is convenient for controlled test environments, but production
    /// deployments should prefer [`Self::trusted_cidr`] or
    /// `http.trusted_proxy.trusted_cidrs` configuration.
    pub fn trust_all(mut self) -> Self {
        self.trust_all = true;
        self
    }

    /// Append a custom header to the priority list (checked after the defaults).
    pub fn with_header(mut self, hdr: HeaderName) -> Self {
        self.headers.push(hdr);
        self
    }

    /// Trust a proxy CIDR range, such as `127.0.0.1/32` or `10.0.0.0/8`.
    pub fn trusted_cidr(mut self, cidr: &str) -> Self {
        match cidr.parse::<IpCidr>() {
            Ok(cidr) => {
                self.trust_all = false;
                self.trusted_cidrs.push(cidr);
            }
            Err(error) => {
                tracing::warn!(
                    cidr = %cidr,
                    error = %error,
                    "foundry: skipping invalid trusted proxy CIDR"
                );
            }
        }
        self
    }

    pub(crate) fn from_config(config: &HttpTrustedProxyConfig) -> Result<Self> {
        let mut headers = Vec::with_capacity(config.headers.len());
        for header in &config.headers {
            headers.push(HeaderName::from_str(header).map_err(|error| {
                Error::message(format!(
                    "invalid http.trusted_proxy.headers value `{header}`: {error}"
                ))
            })?);
        }

        let mut trusted_cidrs = Vec::with_capacity(config.trusted_cidrs.len());
        for cidr in &config.trusted_cidrs {
            trusted_cidrs.push(cidr.parse::<IpCidr>().map_err(|error| {
                Error::message(format!(
                    "invalid http.trusted_proxy.trusted_cidrs value `{cidr}`: {error}"
                ))
            })?);
        }

        Ok(Self {
            headers,
            trusted_cidrs,
            trust_all: false,
        })
    }

    pub(crate) fn resolve_ip(&self, headers: &HeaderMap, peer_ip: IpAddr) -> IpAddr {
        if self.trust_all || self.trusted_cidrs.iter().any(|cidr| cidr.contains(peer_ip)) {
            resolve_real_ip(headers, &self.headers).unwrap_or(peer_ip)
        } else {
            peer_ip
        }
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::TrustedProxy(self)
    }

    fn apply(self, router: axum::Router<AppContext>, app: &AppContext) -> axum::Router<AppContext> {
        if self.trust_all
            && app
                .config()
                .app()
                .map(|config| config.environment.is_production_like())
                .unwrap_or(false)
        {
            tracing::warn!(
                "foundry: TrustedProxy middleware trusts all proxy headers; configure http.trusted_proxy.trusted_cidrs for production"
            );
        }
        let state = TrustedProxyState {
            headers: self.headers,
            trusted_cidrs: self.trusted_cidrs,
            trust_all: self.trust_all,
        };
        router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let state = state.clone();
            async move { trusted_proxy_fn(request, next, state).await }
        }))
    }
}

impl Default for TrustedProxy {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct TrustedProxyState {
    headers: Vec<HeaderName>,
    trusted_cidrs: Vec<IpCidr>,
    trust_all: bool,
}

async fn trusted_proxy_fn(mut request: Request, next: Next, state: TrustedProxyState) -> Response {
    let peer_ip = peer_ip(&request).unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let proxy = TrustedProxy {
        headers: state.headers,
        trusted_cidrs: state.trusted_cidrs,
        trust_all: state.trust_all,
    };
    let ip = proxy.resolve_ip(request.headers(), peer_ip);
    request.extensions_mut().insert(RealIp(ip));
    next.run(request).await
}

fn peer_ip(request: &Request) -> Option<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfoAddr>()
        .map(|addr| addr.0.ip())
        .or_else(|| {
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip())
        })
}

pub(crate) fn resolve_real_ip(
    headers: &HeaderMap,
    headers_to_check: &[HeaderName],
) -> Option<IpAddr> {
    for header_name in headers_to_check {
        let Some(value) = headers.get(header_name).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        let raw = if header_name.as_str().eq_ignore_ascii_case(X_FORWARDED_FOR) {
            value.split(',').next().unwrap_or(value)
        } else {
            value
        };
        if let Ok(ip) = raw.trim().parse::<IpAddr>() {
            return Some(ip);
        }
    }

    None
}

pub(crate) fn resolve_real_ip_from_trusted_proxy_config(
    headers: &HeaderMap,
    peer_ip: IpAddr,
    config: &HttpTrustedProxyConfig,
) -> Result<IpAddr> {
    if !config.enabled {
        return Ok(peer_ip);
    }
    Ok(TrustedProxy::from_config(config)?.resolve_ip(headers, peer_ip))
}

fn default_proxy_headers() -> Vec<HeaderName> {
    vec![
        HeaderName::from_static(CF_CONNECTING_IP),
        HeaderName::from_static(X_REAL_IP),
        HeaderName::from_static(X_FORWARDED_FOR),
    ]
}

fn default_trusted_proxy_cidrs() -> Vec<IpCidr> {
    CLOUDFLARE_TRUSTED_CIDRS
        .iter()
        .map(|cidr| {
            cidr.parse::<IpCidr>()
                .expect("Cloudflare trusted proxy CIDR constant is valid")
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IpCidr {
    network: IpAddr,
    prefix: u8,
}

impl IpCidr {
    pub(crate) fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                let prefix = self.prefix.min(32);
                let mask = if prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                u32::from(network) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                let prefix = self.prefix.min(128);
                let mask = if prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - prefix)
                };
                u128::from(network) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

impl FromStr for IpCidr {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() {
            return Err(Error::message("CIDR value is empty"));
        }

        let (ip, prefix) = match value.split_once('/') {
            Some((ip, prefix)) => {
                let ip = ip.parse::<IpAddr>().map_err(Error::other)?;
                let prefix = prefix.parse::<u8>().map_err(Error::other)?;
                (ip, prefix)
            }
            None => {
                let ip = value.parse::<IpAddr>().map_err(Error::other)?;
                let prefix = match ip {
                    IpAddr::V4(_) => 32,
                    IpAddr::V6(_) => 128,
                };
                (ip, prefix)
            }
        };

        let max_prefix = match ip {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > max_prefix {
            return Err(Error::message(format!(
                "CIDR prefix {prefix} is invalid for {ip}"
            )));
        }

        Ok(Self {
            network: ip,
            prefix,
        })
    }
}

// ---------------------------------------------------------------------------
// ETag — Conditional response middleware
// ---------------------------------------------------------------------------

const ETAG_MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// ETag / conditional response middleware.
///
/// Computes a SHA-256 based ETag for successful responses and returns
/// `304 Not Modified` when the client sends a matching `If-None-Match` header.
///
/// ```
/// use foundry::http::middleware::ETag;
///
/// let etag = ETag::new().build();
/// ```
#[derive(Clone, Debug)]
pub struct ETag;

impl ETag {
    pub fn new() -> Self {
        Self
    }

    /// Convert into a `MiddlewareConfig`.
    pub fn build(self) -> MiddlewareConfig {
        MiddlewareConfig::ETag(self)
    }

    fn apply(self, router: axum::Router<AppContext>) -> axum::Router<AppContext> {
        router.layer(middleware::from_fn(etag_middleware))
    }
}

impl Default for ETag {
    fn default() -> Self {
        Self::new()
    }
}

async fn etag_middleware(request: Request, next: Next) -> Response {
    let if_none_match = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let response = next.run(request).await;

    // Only compute ETag for successful responses
    if !response.status().is_success() {
        return response;
    }

    let (parts, body) = response.into_parts();
    let known_size = body.size_hint().upper();
    if !matches!(known_size, Some(size) if size <= ETAG_MAX_BODY_SIZE as u64) {
        return Response::from_parts(parts, body);
    }

    let bytes = match axum::body::to_bytes(body, ETAG_MAX_BODY_SIZE).await {
        Ok(bytes) => bytes,
        Err(_) => return Response::from_parts(parts, axum::body::Body::empty()),
    };

    // Compute ETag from body hash (truncated to 32 hex chars for compactness)
    let hash = crate::support::sha256_hex(&bytes);
    let etag = format!("\"{}\"", &hash[..32]);

    // Check If-None-Match
    if let Some(ref client_etag) = if_none_match {
        let trimmed = client_etag.trim();
        if trimmed == etag || trimmed.trim_matches('"') == &hash[..32] {
            return match Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, &etag)
                .body(axum::body::Body::empty())
            {
                Ok(response) => response,
                Err(_) => StatusCode::NOT_MODIFIED.into_response(),
            };
        }
    }

    // Build response with ETag header
    let mut response = Response::from_parts(parts, axum::body::Body::from(bytes));
    if let Ok(etag_value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, etag_value);
    }
    response
}

// ---------------------------------------------------------------------------
// MiddlewareGroups — named groups for reuse on routes
// ---------------------------------------------------------------------------

/// Named middleware groups registered on `AppBuilder`.
#[derive(Clone, Debug, Default)]
pub struct MiddlewareGroups(pub std::collections::HashMap<String, Vec<MiddlewareConfig>>);

impl MiddlewareGroups {
    pub fn get(&self, name: &str) -> Option<&Vec<MiddlewareConfig>> {
        self.0.get(name)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::{get, post};
    use tower::ServiceExt;

    use crate::config::ConfigRepository;
    use crate::foundation::Container;
    use crate::validation::RuleRegistry;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    async fn large_body_handler() -> axum::body::Body {
        axum::body::Body::from(vec![b'x'; ETAG_MAX_BODY_SIZE + 1])
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    async fn test_app_in_maintenance(secret: &str) -> AppContext {
        let app = test_app();
        let backend = RuntimeBackend::memory(&format!("http-middleware-{}", uuid::Uuid::now_v7()));
        backend
            .set_value("maintenance:active", secret, 60)
            .await
            .unwrap();
        app.container().singleton(backend).unwrap();
        app
    }

    async fn json_handler(axum::Json(_payload): axum::Json<serde_json::Value>) -> StatusCode {
        StatusCode::OK
    }

    async fn csrf_token_handler(CsrfToken(token): CsrfToken) -> axum::Json<serde_json::Value> {
        axum::Json(serde_json::json!({ "token": token }))
    }

    // ---- Csrf tests ----

    #[tokio::test]
    async fn csrf_safe_request_sets_readable_cookie_and_extension() {
        let router = Csrf::new()
            .secure(false)
            .apply(axum::Router::<AppContext>::new().route("/", get(csrf_token_handler)))
            .with_state(test_app());

        let response = router
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = payload["token"].as_str().unwrap();

        assert!(cookie.starts_with(&format!("foundry_csrf={token};")));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(!cookie.contains("HttpOnly"));
        assert!(!cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn csrf_unsafe_request_without_cookie_returns_json_403() {
        let router = Csrf::new()
            .secure(false)
            .apply(axum::Router::<AppContext>::new().route("/", post(ok_handler)))
            .with_state(test_app());

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["message"], "CSRF token cookie missing");
        assert_eq!(payload["status"], 403);
    }

    #[tokio::test]
    async fn csrf_unsafe_request_with_matching_token_passes() {
        let router = Csrf::new()
            .secure(false)
            .apply(axum::Router::<AppContext>::new().route("/", post(ok_handler)))
            .with_state(test_app());

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::COOKIE, "foundry_csrf=abc_123")
                    .header("x-csrf-token", "abc_123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn csrf_exclude_matches_path_segments_only() {
        let router = Csrf::new()
            .secure(false)
            .exclude("/api")
            .apply(
                axum::Router::<AppContext>::new()
                    .route("/api/users", post(ok_handler))
                    .route("/apiary", post(ok_handler)),
            )
            .with_state(test_app());

        let excluded = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/users")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let protected = router
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/apiary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(excluded.status(), StatusCode::OK);
        assert_eq!(protected.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn csrf_malformed_token_returns_json_403() {
        let router = Csrf::new()
            .secure(false)
            .apply(axum::Router::<AppContext>::new().route("/", post(ok_handler)))
            .with_state(test_app());

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::COOKIE, "foundry_csrf=bad token")
                    .header("x-csrf-token", "bad token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["message"], "CSRF token malformed");
        assert_eq!(payload["status"], 403);
    }

    #[test]
    fn csrf_from_config_validates_cookie_and_header_settings() {
        let csrf = Csrf::from_config(&crate::config::HttpCsrfConfig {
            enabled: true,
            cookie_name: "app_csrf".to_string(),
            header_name: "x-app-csrf".to_string(),
            cookie_secure: true,
            cookie_path: "/admin".to_string(),
            cookie_same_site: "none".to_string(),
            exclude_paths: vec!["/api".to_string()],
        })
        .unwrap();

        assert_eq!(csrf.cookie_name, "app_csrf");
        assert_eq!(csrf.header_name, HeaderName::from_static("x-app-csrf"));
        assert_eq!(csrf.path, "/admin");
        assert_eq!(csrf.same_site, "none");
        assert_eq!(csrf.exclude, vec!["/api"]);

        let error = Csrf::from_config(&crate::config::HttpCsrfConfig {
            enabled: true,
            cookie_name: "app_csrf".to_string(),
            header_name: "x-app-csrf".to_string(),
            cookie_secure: false,
            cookie_path: "/".to_string(),
            cookie_same_site: "none".to_string(),
            exclude_paths: Vec::new(),
        })
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("http.csrf.cookie_same_site = \"none\" requires"));
    }

    // ---- Cors tests ----

    #[tokio::test]
    async fn cors_preflight_returns_correct_headers() {
        let cors = Cors::new()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        let router = axum::Router::<()>::new()
            .route("/", get(ok_handler))
            .layer(build_cors_layer(cors));

        let request = HttpRequest::builder()
            .method("OPTIONS")
            .header(header::ORIGIN, "https://example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();
        // CORS layer forwards to the handler; the handler returns 200 with "ok"
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "*"
        );
    }

    #[tokio::test]
    async fn cors_actual_request_with_origin() {
        let cors = Cors::new()
            .allow_origin("https://example.com")
            .allow_any_method()
            .allow_any_header();

        let router = axum::Router::<()>::new()
            .route("/", get(ok_handler))
            .layer(build_cors_layer(cors));

        let request = HttpRequest::builder()
            .header(header::ORIGIN, "https://example.com")
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn cors_config_rejects_wildcard_with_credentials() {
        let config = crate::config::HttpCorsConfig {
            enabled: true,
            allowed_origins: vec!["*".to_string()],
            allow_credentials: true,
            ..Default::default()
        };

        let error = Cors::from_config(&config).unwrap_err();

        assert!(error
            .to_string()
            .contains("http.cors.allow_credentials cannot be true"));
    }

    fn build_cors_layer(cors: Cors) -> CorsLayer {
        let mut layer = CorsLayer::new();
        layer = match cors.origins {
            CorsOrigins::Any => layer.allow_origin(tower_http::cors::Any),
            CorsOrigins::List(ref origins) if origins.len() == 1 => {
                let v = HeaderValue::from_str(&origins[0]).unwrap();
                layer.allow_origin(v)
            }
            CorsOrigins::List(ref origins) => {
                let values: Vec<HeaderValue> = origins
                    .iter()
                    .filter_map(|o| HeaderValue::from_str(o).ok())
                    .collect();
                layer.allow_origin(values)
            }
            CorsOrigins::None => layer,
        };
        layer = match cors.methods {
            CorsMethods::Any => layer.allow_methods(tower_http::cors::Any),
            CorsMethods::List(methods) => layer.allow_methods(methods),
            CorsMethods::None => layer,
        };
        layer = match cors.headers {
            CorsHeaders::Any => layer.allow_headers(tower_http::cors::Any),
            CorsHeaders::List(headers) => layer.allow_headers(headers),
            CorsHeaders::None => layer,
        };
        layer
    }

    // ---- MaxBodySize tests ----

    #[tokio::test]
    async fn max_body_size_updates_axum_default_body_limit() {
        let body = serde_json::json!({
            "payload": "x".repeat(2 * 1024 * 1024 + 1),
        })
        .to_string();

        let router = MaxBodySize::mb(3)
            .apply(axum::Router::new().route("/", post(json_handler)))
            .with_state(test_app());

        let request = HttpRequest::builder()
            .method(Method::POST)
            .uri("/")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    // ---- SecurityHeaders tests ----

    #[tokio::test]
    async fn security_headers_adds_defaults() {
        let config = SecurityHeaders::new();
        let headers_vec = config.headers.clone();

        let router =
            axum::Router::<()>::new()
                .route("/", get(ok_handler))
                .layer(axum::middleware::from_fn(
                    move |req: Request, next: Next| {
                        let h = headers_vec.clone();
                        async move {
                            let mut resp: Response = next.run(req).await;
                            for (name, value) in &h {
                                resp.headers_mut().insert(name.clone(), value.clone());
                            }
                            resp
                        }
                    },
                ));

        let request = HttpRequest::builder().body(Body::empty()).unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(response.headers().get(X_FRAME_OPTIONS).unwrap(), "DENY");
        assert!(response.headers().get(HSTS_HEADER).is_some());
        assert!(response.headers().get(REFERRER_POLICY).is_some());
        assert_eq!(response.headers().get(X_XSS_PROTECTION).unwrap(), "0");
    }

    #[tokio::test]
    async fn security_headers_disable_hsts() {
        let config = SecurityHeaders::new().disable_hsts();
        assert!(!config.headers.iter().any(|(n, _)| *n == HSTS_HEADER));
    }

    #[tokio::test]
    async fn security_headers_custom_frame_options() {
        let config = SecurityHeaders::new().frame_options("SAMEORIGIN");
        let frame_entry = config.headers.iter().find(|(n, _)| *n == X_FRAME_OPTIONS);
        assert!(frame_entry.is_some());
        assert_eq!(frame_entry.unwrap().1, "SAMEORIGIN");
    }

    // ---- RateLimit tests ----

    #[tokio::test]
    async fn rate_limit_allows_under_limit() {
        let state = RateLimitState {
            max: 2,
            window_secs: 60,
            key_prefix: "test:".to_string(),
            by: RateLimitBy::Ip,
            store: RateLimitStore::Memory(Arc::new(Mutex::new(HashMap::new()))),
        };

        let router = axum::Router::new().route("/", get(ok_handler)).layer(
            axum::middleware::from_fn_with_state(state.clone(), rate_limit_middleware),
        );

        let request = HttpRequest::builder().body(Body::empty()).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("x-ratelimit-remaining").unwrap(),
            "1"
        );
    }

    #[tokio::test]
    async fn rate_limit_blocks_over_limit() {
        let state = RateLimitState {
            max: 1,
            window_secs: 60,
            key_prefix: "test:".to_string(),
            by: RateLimitBy::Ip,
            store: RateLimitStore::Memory(Arc::new(Mutex::new(HashMap::new()))),
        };

        let router = axum::Router::new().route("/", get(ok_handler)).layer(
            axum::middleware::from_fn_with_state(state.clone(), rate_limit_middleware),
        );

        // First request passes
        let request = HttpRequest::builder().body(Body::empty()).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Second request is blocked
        let request = HttpRequest::builder().body(Body::empty()).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().get(header::RETRY_AFTER).is_some());
    }

    #[tokio::test]
    async fn actor_rate_limit_does_not_fall_back_to_ip_before_auth() {
        let state = RateLimitState {
            max: 0,
            window_secs: 60,
            key_prefix: "test:".to_string(),
            by: RateLimitBy::Actor,
            store: RateLimitStore::Memory(Arc::new(Mutex::new(HashMap::new()))),
        };

        let router = axum::Router::new().route("/", get(ok_handler)).layer(
            axum::middleware::from_fn_with_state(state.clone(), rate_limit_middleware),
        );

        let request = HttpRequest::builder().body(Body::empty()).unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("x-ratelimit-limit").is_none());
    }

    #[test]
    fn actor_or_ip_rate_limit_uses_cloudflare_real_ip_fallback() {
        let state = RateLimitState {
            max: 1,
            window_secs: 60,
            key_prefix: "test:".to_string(),
            by: RateLimitBy::ActorOrIp,
            store: RateLimitStore::Memory(Arc::new(Mutex::new(HashMap::new()))),
        };
        let headers = HeaderMap::from_iter([(
            HeaderName::from_static(CF_CONNECTING_IP),
            HeaderValue::from_static("203.0.113.9"),
        )]);
        let cloudflare_peer = IpAddr::V4(Ipv4Addr::new(173, 245, 48, 1));
        let real_ip = TrustedProxy::new().resolve_ip(&headers, cloudflare_peer);
        let mut request = HttpRequest::builder().body(Body::empty()).unwrap();
        request.extensions_mut().insert(RealIp(real_ip));

        assert_eq!(
            rate_limit_key_from_request(&state, &request).as_deref(),
            Some("ip:203.0.113.9")
        );
    }

    // ---- MaintenanceMode tests ----

    #[tokio::test]
    async fn maintenance_query_bypass_is_disabled_by_default() {
        let app = test_app_in_maintenance("secret").await;
        let router = MaintenanceMode::new()
            .apply(
                axum::Router::<AppContext>::new().route("/", get(ok_handler)),
                &app,
            )
            .with_state(app);

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/?bypass=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn maintenance_header_bypass_remains_enabled() {
        let app = test_app_in_maintenance("secret").await;
        let router = MaintenanceMode::new()
            .apply(
                axum::Router::<AppContext>::new().route("/", get(ok_handler)),
                &app,
            )
            .with_state(app);

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .header("x-maintenance-bypass", "secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn maintenance_query_bypass_can_be_enabled_for_legacy_clients() {
        let app = test_app_in_maintenance("secret").await;
        let router = MaintenanceMode::new()
            .allow_query_bypass()
            .apply(
                axum::Router::<AppContext>::new().route("/", get(ok_handler)),
                &app,
            )
            .with_state(app);

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/?bypass=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    // ---- TrustedProxy tests ----

    #[tokio::test]
    async fn trusted_proxy_x_forwarded_for() {
        let headers = HeaderMap::from_iter([(
            HeaderName::from_static(X_FORWARDED_FOR),
            HeaderValue::from_static("1.2.3.4, 5.6.7.8"),
        )]);
        let ip = resolve_real_ip(&headers, &default_proxy_headers());

        assert_eq!(ip, Some(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))));
    }

    #[tokio::test]
    async fn trusted_proxy_cf_connecting_ip_takes_priority() {
        let ip = resolve_real_ip(
            &HeaderMap::from_iter([
                (
                    HeaderName::from_static("cf-connecting-ip"),
                    HeaderValue::from_static("10.0.0.1"),
                ),
                (
                    HeaderName::from_static("x-real-ip"),
                    HeaderValue::from_static("10.0.0.2"),
                ),
            ]),
            &default_proxy_headers(),
        );
        assert_eq!(ip, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    }

    #[tokio::test]
    async fn trusted_proxy_x_real_ip_when_no_cf() {
        let ip = resolve_real_ip(
            &HeaderMap::from_iter([
                (
                    HeaderName::from_static("x-real-ip"),
                    HeaderValue::from_static("10.0.0.3"),
                ),
                (
                    HeaderName::from_static("x-forwarded-for"),
                    HeaderValue::from_static("10.0.0.4"),
                ),
            ]),
            &default_proxy_headers(),
        );
        assert_eq!(ip, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3))));
    }

    #[tokio::test]
    async fn trusted_proxy_custom_header() {
        let custom = HeaderName::from_static("x-custom-ip");
        let ip = resolve_real_ip(
            &HeaderMap::from_iter([(custom.clone(), HeaderValue::from_static("10.0.0.5"))]),
            &[custom],
        );
        assert_eq!(ip, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))));
    }

    #[test]
    fn trusted_proxy_config_requires_valid_cidrs() {
        let error = TrustedProxy::from_config(&crate::config::HttpTrustedProxyConfig {
            enabled: true,
            trusted_cidrs: vec!["10.0.0.0/99".to_string()],
            ..Default::default()
        })
        .unwrap_err();

        assert!(error.to_string().contains("CIDR prefix 99 is invalid"));
    }

    #[test]
    fn trusted_proxy_cidr_matching_supports_ipv4_and_ipv6() {
        let proxy = TrustedProxy::from_config(&crate::config::HttpTrustedProxyConfig {
            enabled: true,
            trusted_cidrs: vec!["10.0.0.0/8".to_string(), "2001:db8::/32".to_string()],
            ..Default::default()
        })
        .unwrap();

        assert!(proxy
            .trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains(IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40)))));
        assert!(!proxy
            .trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))));
        assert!(proxy
            .trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains("2001:db8::1".parse().unwrap())));
        assert!(!proxy
            .trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains("2001:dead::1".parse().unwrap())));
    }

    #[test]
    fn trusted_proxy_new_trusts_cloudflare_peer_by_default() {
        let proxy = TrustedProxy::new();
        let peer_ip = IpAddr::V4(Ipv4Addr::new(173, 245, 48, 1));
        let headers = HeaderMap::from_iter([(
            HeaderName::from_static(X_REAL_IP),
            HeaderValue::from_static("10.0.0.5"),
        )]);

        assert_eq!(
            proxy.resolve_ip(&headers, peer_ip),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))
        );
    }

    #[test]
    fn trusted_proxy_new_ignores_headers_from_untrusted_peer() {
        let proxy = TrustedProxy::new();
        let peer_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let headers = HeaderMap::from_iter([(
            HeaderName::from_static(X_REAL_IP),
            HeaderValue::from_static("10.0.0.5"),
        )]);

        assert_eq!(proxy.resolve_ip(&headers, peer_ip), peer_ip);
    }

    #[test]
    fn trusted_proxy_trust_all_preserves_explicit_legacy_behavior() {
        let proxy = TrustedProxy::new().trust_all();
        let peer_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let headers = HeaderMap::from_iter([(
            HeaderName::from_static(X_REAL_IP),
            HeaderValue::from_static("10.0.0.5"),
        )]);

        assert_eq!(
            proxy.resolve_ip(&headers, peer_ip),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))
        );
    }

    // ---- ETag tests ----

    #[tokio::test]
    async fn etag_leaves_known_large_body_unchanged() {
        let router = ETag::new()
            .apply(axum::Router::<AppContext>::new().route("/", get(large_body_handler)))
            .with_state(test_app());

        let response = router
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::ETAG).is_none());
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.len(), ETAG_MAX_BODY_SIZE + 1);
    }

    #[test]
    fn configured_http_middlewares_skip_explicit_duplicate_kinds() {
        let http = crate::config::HttpConfig {
            max_body_size_bytes: 1024,
            security_headers: crate::config::HttpSecurityHeadersConfig {
                enabled: true,
                ..Default::default()
            },
            rate_limit: crate::config::HttpRateLimitConfig {
                enabled: true,
                ..Default::default()
            },
            csrf: crate::config::HttpCsrfConfig {
                enabled: true,
                cookie_secure: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let middlewares =
            configured_global_middlewares(&http, &[SecurityHeaders::new().build()]).unwrap();

        assert!(middlewares
            .iter()
            .any(|middleware| matches!(middleware, MiddlewareConfig::MaxBodySize(_))));
        assert!(middlewares
            .iter()
            .any(|middleware| matches!(middleware, MiddlewareConfig::RateLimit(_))));
        assert!(middlewares
            .iter()
            .any(|middleware| matches!(middleware, MiddlewareConfig::Csrf(_))));
        assert!(!middlewares
            .iter()
            .any(|middleware| matches!(middleware, MiddlewareConfig::SecurityHeaders(_))));
    }

    // ---- MiddlewareConfig ordering ----

    #[test]
    fn middleware_ordering_priorities() {
        let configs = [
            MiddlewareConfig::MaxBodySize(MaxBodySize::mb(1)),
            MiddlewareConfig::Cors(Cors::new()),
            MiddlewareConfig::TrustedProxy(TrustedProxy::new()),
            MiddlewareConfig::MaintenanceMode(MaintenanceMode::new()),
            MiddlewareConfig::Csrf(Csrf::new()),
            MiddlewareConfig::RateLimit(RateLimit::new(100)),
            MiddlewareConfig::RequestTimeout(RequestTimeout::secs(30)),
            MiddlewareConfig::ETag(ETag::new()),
            MiddlewareConfig::SecurityHeaders(SecurityHeaders::new()),
            MiddlewareConfig::Compression(Compression::new()),
        ];

        let mut with_priorities: Vec<(u8, &str)> = configs
            .iter()
            .map(|c| {
                let name = match c {
                    MiddlewareConfig::TrustedProxy(_) => "TrustedProxy",
                    MiddlewareConfig::MaintenanceMode(_) => "MaintenanceMode",
                    MiddlewareConfig::Cors(_) => "Cors",
                    MiddlewareConfig::SecurityHeaders(_) => "SecurityHeaders",
                    MiddlewareConfig::Csrf(_) => "Csrf",
                    MiddlewareConfig::RateLimit(_) => "RateLimit",
                    MiddlewareConfig::MaxBodySize(_) => "MaxBodySize",
                    MiddlewareConfig::RequestTimeout(_) => "RequestTimeout",
                    MiddlewareConfig::ETag(_) => "ETag",
                    MiddlewareConfig::Compression(_) => "Compression",
                };
                (c.priority(), name)
            })
            .collect();

        with_priorities.sort_by_key(|(p, _)| *p);

        let names: Vec<&str> = with_priorities.iter().map(|(_, n)| *n).collect();
        assert_eq!(
            names,
            vec![
                "TrustedProxy",
                "MaintenanceMode",
                "Cors",
                "SecurityHeaders",
                "Csrf",
                "RateLimit",
                "MaxBodySize",
                "RequestTimeout",
                "ETag",
                "Compression",
            ]
        );
    }
}
