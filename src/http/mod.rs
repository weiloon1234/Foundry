pub mod cookie;
pub mod download;
pub mod middleware;
pub mod resource;
pub mod response;
pub mod routes;
pub(crate) mod spa;

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::handler::Handler;
use axum::middleware::{self as axum_middleware, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use axum::Router;

use crate::auth::{token::actor_has_mfa_pending, AccessScope, Actor, AuthError, Authenticatable};
use crate::foundation::{AppContext, Error, Result};
use crate::http::middleware::MiddlewareConfig;
use crate::logging::{catch_future_panic, catch_sync_panic, panic_payload_message, AuthOutcome};
use crate::support::{GuardId, PermissionId, RouteId};
pub use crate::validation::{JsonValidated, Validated};

pub type RouteRegistrar = Arc<dyn Fn(&mut HttpRegistrar) -> Result<()> + Send + Sync>;
pub type HttpRouter = Router<AppContext>;
pub type HttpAuthorizeCallback = Arc<
    dyn Fn(HttpAuthorizeContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

pub(crate) fn build_registrar(registrars: &[RouteRegistrar]) -> Result<HttpRegistrar> {
    let mut registrar = HttpRegistrar::new();
    for routes in registrars {
        let result = run_route_registrar(routes, &mut registrar);
        registrar.take_registration_error()?;
        result?;
    }
    Ok(registrar)
}

pub(crate) fn collect_named_routes(registrars: &[RouteRegistrar]) -> Result<routes::RouteRegistry> {
    Ok(build_registrar(registrars)?.named_routes)
}

fn run_route_registrar(registrar: &RouteRegistrar, routes: &mut HttpRegistrar) -> Result<()> {
    match catch_sync_panic(|| registrar(routes)) {
        Ok(result) => result,
        Err(panic) => Err(http_registration_panic_error("route registrar", panic)),
    }
}

fn run_http_registration_callback<T>(
    subject: &'static str,
    target: &mut T,
    callback: impl FnOnce(&mut T) -> Result<()>,
) -> Result<()> {
    match catch_sync_panic(|| callback(target)) {
        Ok(result) => result,
        Err(panic) => Err(http_registration_panic_error(subject, panic)),
    }
}

fn http_registration_panic_error(
    subject: &'static str,
    panic: Box<dyn std::any::Any + Send>,
) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.http",
        subject = subject,
        panic = %message,
        "HTTP registration callback panicked"
    );
    Error::message(format!("{subject} panicked: {message}"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteManifestEntry {
    pub id: RouteId,
    pub path: String,
    pub method: Option<String>,
    pub params: Vec<String>,
    pub client_export: bool,
    pub guard: Option<GuardId>,
    pub permissions: Vec<PermissionId>,
    pub summary: Option<String>,
    pub request: Option<&'static str>,
    pub responses: Vec<RouteManifestResponse>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteManifestResponse {
    pub status: u16,
    pub schema: &'static str,
    pub(crate) schema_json: serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum AuditAreaSetting {
    #[default]
    Inherit,
    Disabled,
    Area(String),
}

#[derive(Clone)]
pub struct HttpAuthorizeContext {
    app: AppContext,
    actor: Actor,
}

impl HttpAuthorizeContext {
    fn new(app: AppContext, actor: Actor) -> Self {
        Self { app, actor }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub async fn resolve_actor<M: Authenticatable>(&self) -> Result<Option<M>> {
        self.actor.resolve::<M>(&self.app).await
    }
}

#[derive(Clone)]
pub struct HttpRouteOptions {
    pub access: AccessScope,
    middlewares: Vec<MiddlewareConfig>,
    middleware_group_name: Option<String>,
    pub(crate) authorize: Option<HttpAuthorizeCallback>,
    pub(crate) post_auth_rate_limit: Option<middleware::RateLimit>,
    pub(crate) allow_mfa_pending_token: bool,
    pub(crate) client_export: bool,
    audit_area: AuditAreaSetting,
    pub(crate) doc: Option<crate::openapi::RouteDoc>,
}

impl Default for HttpRouteOptions {
    fn default() -> Self {
        Self {
            access: AccessScope::default(),
            middlewares: Vec::new(),
            middleware_group_name: None,
            authorize: None,
            post_auth_rate_limit: None,
            allow_mfa_pending_token: false,
            client_export: true,
            audit_area: AuditAreaSetting::default(),
            doc: None,
        }
    }
}

impl HttpRouteOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn guard<I>(mut self, guard: I) -> Self
    where
        I: Into<GuardId>,
    {
        self.access = self.access.with_guard(guard);
        self
    }

    pub fn permission<I>(mut self, permission: I) -> Self
    where
        I: Into<PermissionId>,
    {
        self.access = self.access.with_permission(permission);
        self
    }

    pub fn permissions<I, P>(mut self, permissions: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PermissionId>,
    {
        self.access = self.access.with_permissions(permissions);
        self
    }

    /// Add a dynamic authorization callback for this route.
    ///
    /// Called after guard and permission checks succeed. Return `Ok(())` to
    /// allow the request or `Err(...)` to reject with a project-defined
    /// response such as 401, 403, or 404.
    pub fn authorize<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(HttpAuthorizeContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.authorize = Some(wrap_http_authorize_callback(f));
        self
    }

    /// Attach a middleware to this specific route.
    ///
    /// Per-route middleware runs between global middleware and auth middleware.
    /// Multiple calls append middleware in order.
    pub fn middleware(mut self, config: MiddlewareConfig) -> Self {
        self.middlewares.push(config);
        self
    }

    pub fn allow_mfa_pending_token(mut self) -> Self {
        self.allow_mfa_pending_token = true;
        self
    }

    /// Enable or disable generation of the TypeScript endpoint helper for this route.
    ///
    /// DTOs and route manifest metadata remain available when this is disabled.
    pub fn client_export(mut self, enabled: bool) -> Self {
        self.client_export = enabled;
        self
    }

    /// Disable generation of the TypeScript endpoint helper for this route.
    ///
    /// This is equivalent to `client_export(false)`.
    pub fn without_client_export(mut self) -> Self {
        self.client_export = false;
        self
    }

    pub fn audit_area(mut self, area: &str) -> Self {
        self.audit_area = AuditAreaSetting::Area(area.to_string());
        self
    }

    pub fn audit_disabled(mut self) -> Self {
        self.audit_area = AuditAreaSetting::Disabled;
        self
    }

    /// Apply a named middleware group to this route.
    ///
    /// The group must have been registered via `AppBuilder::middleware_group()`.
    /// Group middlewares are prepended before any per-route middlewares.
    pub fn middleware_group(mut self, name: impl Into<String>) -> Self {
        self.middleware_group_name = Some(name.into());
        self
    }

    /// Attach a rate limiter to this route.
    ///
    /// IP-based rate limiting runs as a normal middleware layer. Actor-based or
    /// actor-or-IP rate limiting is deferred until after authentication so the
    /// actor identity is available for keying.
    pub fn rate_limit(mut self, rate_limit: middleware::RateLimit) -> Self {
        match rate_limit.rate_limit_by() {
            middleware::RateLimitBy::Ip => {
                self.middlewares.push(rate_limit.build());
            }
            _ => {
                self.post_auth_rate_limit = Some(rate_limit);
            }
        }
        self
    }

    /// Attach OpenAPI documentation to this route.
    pub fn document(mut self, doc: crate::openapi::RouteDoc) -> Self {
        self.doc = Some(doc);
        self
    }

    /// Add an OpenAPI tag without building a full [`crate::openapi::RouteDoc`] manually.
    pub fn tag(mut self, tag: &str) -> Self {
        let doc = self.doc.take().unwrap_or_default().tag(tag);
        self.doc = Some(doc);
        self
    }

    /// Add an OpenAPI summary without building a full [`crate::openapi::RouteDoc`] manually.
    pub fn summary(mut self, summary: &str) -> Self {
        let doc = self.doc.take().unwrap_or_default().summary(summary);
        self.doc = Some(doc);
        self
    }

    /// Add an OpenAPI description without building a full [`crate::openapi::RouteDoc`] manually.
    pub fn description(mut self, description: &str) -> Self {
        let doc = self.doc.take().unwrap_or_default().description(description);
        self.doc = Some(doc);
        self
    }

    pub fn request<T: crate::openapi::ApiSchema>(mut self) -> Self {
        let doc = self.doc.take().unwrap_or_default().request::<T>();
        self.doc = Some(doc);
        self
    }

    pub fn response<T: crate::openapi::ApiSchema>(mut self, status: u16) -> Self {
        let doc = self.doc.take().unwrap_or_default().response::<T>(status);
        self.doc = Some(doc);
        self
    }

    pub fn deprecated(mut self) -> Self {
        let doc = self.doc.take().unwrap_or_default().deprecated();
        self.doc = Some(doc);
        self
    }

    fn requires_auth(&self) -> bool {
        self.access.requires_auth() || self.authorize.is_some()
    }

    fn guard_id(&self) -> Option<&GuardId> {
        self.access.guard()
    }

    fn permissions_set(&self) -> BTreeSet<PermissionId> {
        self.access.permissions()
    }

    fn resolved_audit_area(&self) -> Option<&str> {
        match &self.audit_area {
            AuditAreaSetting::Area(area) => Some(area.as_str()),
            AuditAreaSetting::Disabled | AuditAreaSetting::Inherit => None,
        }
    }

    fn with_defaults(mut self, defaults: &Self) -> Self {
        self.access = merge_access_scope(&self.access, &defaults.access);

        let mut middlewares = defaults.middlewares.clone();
        middlewares.extend(self.middlewares);
        self.middlewares = middlewares;

        if self.middleware_group_name.is_none() {
            self.middleware_group_name = defaults.middleware_group_name.clone();
        }
        if self.authorize.is_none() {
            self.authorize = defaults.authorize.clone();
        }
        if self.post_auth_rate_limit.is_none() {
            self.post_auth_rate_limit = defaults.post_auth_rate_limit.clone();
        }
        if defaults.allow_mfa_pending_token {
            self.allow_mfa_pending_token = true;
        }
        self.client_export = self.client_export && defaults.client_export;
        self.audit_area = merge_audit_area_setting(&self.audit_area, &defaults.audit_area);

        self.doc = match (self.doc.take(), defaults.doc.as_ref()) {
            (Some(doc), Some(default_doc)) => Some(doc.merge_defaults(default_doc)),
            (Some(doc), None) => Some(doc),
            (None, Some(default_doc)) => Some(default_doc.clone()),
            (None, None) => None,
        };

        self
    }
}

fn merge_access_scope(explicit: &AccessScope, defaults: &AccessScope) -> AccessScope {
    match (defaults, explicit) {
        (AccessScope::Public, _) => explicit.clone(),
        (AccessScope::Guarded(defaults), AccessScope::Public) => {
            AccessScope::Guarded(defaults.clone())
        }
        (AccessScope::Guarded(defaults), AccessScope::Guarded(explicit)) => {
            let mut merged = defaults.clone();
            if explicit.guard.is_some() {
                merged.guard = explicit.guard.clone();
            }
            merged.permissions.extend(explicit.permissions.clone());
            AccessScope::Guarded(merged)
        }
    }
}

fn merge_audit_area_setting(
    explicit: &AuditAreaSetting,
    defaults: &AuditAreaSetting,
) -> AuditAreaSetting {
    match explicit {
        AuditAreaSetting::Inherit => defaults.clone(),
        AuditAreaSetting::Disabled | AuditAreaSetting::Area(_) => explicit.clone(),
    }
}

#[derive(Default)]
pub struct HttpResourceRoutes {
    index: Option<MethodRouter<AppContext>>,
    store: Option<MethodRouter<AppContext>>,
    show: Option<MethodRouter<AppContext>>,
    update: Option<MethodRouter<AppContext>>,
    destroy: Option<MethodRouter<AppContext>>,
    id_param: String,
}

impl HttpResourceRoutes {
    pub fn new() -> Self {
        Self {
            id_param: "id".to_string(),
            ..Self::default()
        }
    }

    pub fn index(mut self, route: MethodRouter<AppContext>) -> Self {
        self.index = Some(route);
        self
    }

    pub fn store(mut self, route: MethodRouter<AppContext>) -> Self {
        self.store = Some(route);
        self
    }

    pub fn show(mut self, route: MethodRouter<AppContext>) -> Self {
        self.show = Some(route);
        self
    }

    pub fn update(mut self, route: MethodRouter<AppContext>) -> Self {
        self.update = Some(route);
        self
    }

    pub fn destroy(mut self, route: MethodRouter<AppContext>) -> Self {
        self.destroy = Some(route);
        self
    }

    pub fn id_param(mut self, id_param: impl Into<String>) -> Self {
        self.id_param = id_param.into();
        self
    }
}

struct RouteRegistration {
    name: Option<RouteId>,
    path: String,
    method: Option<String>,
    method_router: MethodRouter<AppContext>,
    options: HttpRouteOptions,
    inherit_parent_defaults_on_merge: bool,
}

enum HttpRegistration {
    Route(Box<RouteRegistration>),
    Nest { path: String, router: HttpRouter },
    Merge { router: HttpRouter },
}

#[derive(Clone)]
struct ResolvedHttpScopeState {
    path_prefix: String,
    name_prefix: String,
    options: HttpRouteOptions,
    explicit_tags_started: bool,
}

impl ResolvedHttpScopeState {
    fn root(path: &str, defaults: &HttpRouteOptions) -> Self {
        Self {
            path_prefix: join_path_prefix("", path),
            name_prefix: String::new(),
            options: defaults.clone(),
            explicit_tags_started: false,
        }
    }

    fn child(&self, path: &str) -> Self {
        Self {
            path_prefix: join_path_prefix(&self.path_prefix, path),
            name_prefix: self.name_prefix.clone(),
            options: self.options.clone(),
            explicit_tags_started: false,
        }
    }

    fn route_path(&self, path: &str) -> String {
        join_route_path(&self.path_prefix, path)
    }

    fn route_name(&self, name: &str) -> String {
        join_route_name(&self.name_prefix, name)
    }
}

pub struct HttpScope<'a> {
    registrar: &'a mut HttpRegistrar,
    state: ResolvedHttpScopeState,
}

impl<'a> HttpScope<'a> {
    fn new(registrar: &'a mut HttpRegistrar, state: ResolvedHttpScopeState) -> Self {
        Self { registrar, state }
    }

    pub fn scope(
        &mut self,
        path: &str,
        f: impl FnOnce(&mut HttpScope<'_>) -> Result<()>,
    ) -> Result<&mut Self> {
        let state = self.state.child(path);
        let result = {
            let mut child = HttpScope::new(self.registrar, state);
            f(&mut child)
        };
        result?;
        Ok(self)
    }

    pub fn name_prefix(&mut self, prefix: &str) -> &mut Self {
        self.state.name_prefix = join_route_name(&self.state.name_prefix, prefix);
        self
    }

    pub fn public(&mut self) -> &mut Self {
        self.state.options.access = AccessScope::Public;
        self.state.options.authorize = None;
        self
    }

    pub fn guard<I>(&mut self, guard: I) -> &mut Self
    where
        I: Into<GuardId>,
    {
        self.state.options.access = self.state.options.access.clone().with_guard(guard);
        self
    }

    pub fn permission<I>(&mut self, permission: I) -> &mut Self
    where
        I: Into<PermissionId>,
    {
        self.state.options.access = self
            .state
            .options
            .access
            .clone()
            .with_permission(permission);
        self
    }

    pub fn permissions<I, P>(&mut self, permissions: I) -> &mut Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PermissionId>,
    {
        self.state.options.access = self
            .state
            .options
            .access
            .clone()
            .with_permissions(permissions);
        self
    }

    pub fn authorize<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(HttpAuthorizeContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.state.options.authorize = Some(wrap_http_authorize_callback(f));
        self
    }

    pub fn middleware(&mut self, config: MiddlewareConfig) -> &mut Self {
        self.state.options.middlewares.push(config);
        self
    }

    pub fn middleware_group(&mut self, name: impl Into<String>) -> &mut Self {
        self.state.options.middleware_group_name = Some(name.into());
        self
    }

    pub fn audit_area(&mut self, area: &str) -> &mut Self {
        self.state.options.audit_area = AuditAreaSetting::Area(area.to_string());
        self
    }

    pub fn audit_disabled(&mut self) -> &mut Self {
        self.state.options.audit_area = AuditAreaSetting::Disabled;
        self
    }

    pub fn rate_limit(&mut self, rate_limit: middleware::RateLimit) -> &mut Self {
        apply_rate_limit(&mut self.state.options, rate_limit);
        self
    }

    pub fn tag(&mut self, tag: &str) -> &mut Self {
        apply_tag(
            &mut self.state.options,
            tag,
            &mut self.state.explicit_tags_started,
        );
        self
    }

    pub fn summary(&mut self, summary: &str) -> &mut Self {
        mutate_doc(&mut self.state.options, |doc| doc.summary(summary));
        self
    }

    pub fn description(&mut self, description: &str) -> &mut Self {
        mutate_doc(&mut self.state.options, |doc| doc.description(description));
        self
    }

    pub fn deprecated(&mut self) -> &mut Self {
        mutate_doc(&mut self.state.options, |doc| doc.deprecated());
        self
    }

    pub fn get<H, T>(
        &mut self,
        path: &str,
        name: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_route(path, name, axum::routing::get(handler), "get", configure)
    }

    pub fn post<H, T>(
        &mut self,
        path: &str,
        name: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_route(path, name, axum::routing::post(handler), "post", configure)
    }

    pub fn put<H, T>(
        &mut self,
        path: &str,
        name: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_route(path, name, axum::routing::put(handler), "put", configure)
    }

    pub fn patch<H, T>(
        &mut self,
        path: &str,
        name: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_route(
            path,
            name,
            axum::routing::patch(handler),
            "patch",
            configure,
        )
    }

    pub fn delete<H, T>(
        &mut self,
        path: &str,
        name: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_route(
            path,
            name,
            axum::routing::delete(handler),
            "delete",
            configure,
        )
    }

    fn register_route(
        &mut self,
        path: &str,
        name: &str,
        method_router: MethodRouter<AppContext>,
        method: &str,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self {
        let mut route = HttpRouteBuilder::from_scope(&self.state, method);
        if let Err(error) =
            run_http_registration_callback("http route configure callback", &mut route, |route| {
                configure(route);
                Ok(())
            })
        {
            self.registrar.record_registration_error(error);
            return self;
        }

        self.registrar.route_named_resolved(
            RouteId::owned(self.state.route_name(name)),
            &self.state.route_path(path),
            Some(method),
            method_router,
            route.finish(),
        );
        self
    }
}

pub struct HttpRouteBuilder {
    options: HttpRouteOptions,
    explicit_tags_started: bool,
}

impl HttpRouteBuilder {
    fn root(method: &str) -> Self {
        let mut options = HttpRouteOptions::default();
        mutate_doc(&mut options, |doc| doc.method(method));

        Self {
            options,
            explicit_tags_started: false,
        }
    }

    fn from_scope(scope: &ResolvedHttpScopeState, method: &str) -> Self {
        let mut options = scope.options.clone();
        mutate_doc(&mut options, |doc| doc.method(method));

        Self {
            options,
            explicit_tags_started: false,
        }
    }

    fn finish(self) -> HttpRouteOptions {
        self.options
    }

    pub fn public(&mut self) -> &mut Self {
        self.options.access = AccessScope::Public;
        self.options.authorize = None;
        self
    }

    pub fn guard<I>(&mut self, guard: I) -> &mut Self
    where
        I: Into<GuardId>,
    {
        self.options.access = self.options.access.clone().with_guard(guard);
        self
    }

    pub fn permission<I>(&mut self, permission: I) -> &mut Self
    where
        I: Into<PermissionId>,
    {
        self.options.access = self.options.access.clone().with_permission(permission);
        self
    }

    pub fn permissions<I, P>(&mut self, permissions: I) -> &mut Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PermissionId>,
    {
        self.options.access = self.options.access.clone().with_permissions(permissions);
        self
    }

    pub fn authorize<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(HttpAuthorizeContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.options.authorize = Some(wrap_http_authorize_callback(f));
        self
    }

    pub fn middleware(&mut self, config: MiddlewareConfig) -> &mut Self {
        self.options.middlewares.push(config);
        self
    }

    pub fn middleware_group(&mut self, name: impl Into<String>) -> &mut Self {
        self.options.middleware_group_name = Some(name.into());
        self
    }

    /// Enable or disable generation of the TypeScript endpoint helper for this route.
    ///
    /// DTOs and route manifest metadata remain available when this is disabled.
    pub fn client_export(&mut self, enabled: bool) -> &mut Self {
        self.options.client_export = enabled;
        self
    }

    /// Disable generation of the TypeScript endpoint helper for this route.
    ///
    /// This is equivalent to `client_export(false)`.
    pub fn without_client_export(&mut self) -> &mut Self {
        self.options.client_export = false;
        self
    }

    pub fn audit_area(&mut self, area: &str) -> &mut Self {
        self.options.audit_area = AuditAreaSetting::Area(area.to_string());
        self
    }

    pub fn audit_disabled(&mut self) -> &mut Self {
        self.options.audit_area = AuditAreaSetting::Disabled;
        self
    }

    pub fn rate_limit(&mut self, rate_limit: middleware::RateLimit) -> &mut Self {
        apply_rate_limit(&mut self.options, rate_limit);
        self
    }

    pub fn tag(&mut self, tag: &str) -> &mut Self {
        apply_tag(&mut self.options, tag, &mut self.explicit_tags_started);
        self
    }

    pub fn summary(&mut self, summary: &str) -> &mut Self {
        mutate_doc(&mut self.options, |doc| doc.summary(summary));
        self
    }

    pub fn description(&mut self, description: &str) -> &mut Self {
        mutate_doc(&mut self.options, |doc| doc.description(description));
        self
    }

    pub fn request<T: crate::openapi::ApiSchema>(&mut self) -> &mut Self {
        mutate_doc(&mut self.options, |doc| doc.request::<T>());
        self
    }

    pub fn response<T: crate::openapi::ApiSchema>(&mut self, status: u16) -> &mut Self {
        mutate_doc(&mut self.options, |doc| doc.response::<T>(status));
        self
    }

    pub fn deprecated(&mut self) -> &mut Self {
        mutate_doc(&mut self.options, |doc| doc.deprecated());
        self
    }
}

fn mutate_doc(
    options: &mut HttpRouteOptions,
    f: impl FnOnce(crate::openapi::RouteDoc) -> crate::openapi::RouteDoc,
) {
    let doc = options.doc.take().unwrap_or_default();
    options.doc = Some(f(doc));
}

fn apply_tag(options: &mut HttpRouteOptions, tag: &str, explicit_tags_started: &mut bool) {
    let mut doc = options.doc.take().unwrap_or_default();
    if !*explicit_tags_started {
        doc.tags = vec![tag.to_string()];
        *explicit_tags_started = true;
    } else if !doc.tags.iter().any(|existing| existing == tag) {
        doc.tags.push(tag.to_string());
    }
    options.doc = Some(doc);
}

fn apply_rate_limit(options: &mut HttpRouteOptions, rate_limit: middleware::RateLimit) {
    match rate_limit.rate_limit_by() {
        middleware::RateLimitBy::Ip => {
            options.middlewares.push(rate_limit.build());
        }
        _ => {
            options.post_auth_rate_limit = Some(rate_limit);
        }
    }
}

pub(crate) fn wrap_http_authorize_callback<F, Fut>(f: F) -> HttpAuthorizeCallback
where
    F: Fn(HttpAuthorizeContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    Arc::new(move |ctx| match catch_sync_panic(|| f(ctx)) {
        Ok(future) => Box::pin(async move {
            match catch_future_panic(future).await {
                Ok(result) => result,
                Err(panic) => Err(http_authorizer_panic_error(panic)),
            }
        }),
        Err(panic) => Box::pin(async move { Err(http_authorizer_panic_error(panic)) }),
    })
}

fn http_authorizer_panic_error(panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.auth",
        panic = %message,
        "http authorizer panicked"
    );
    Error::message(format!("http authorizer panicked: {message}"))
}

fn join_path_prefix(base: &str, path: &str) -> String {
    let base = base.trim_matches('/');
    let path = path.trim_matches('/');

    match (base.is_empty(), path.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("/{path}"),
        (false, true) => format!("/{base}"),
        (false, false) => format!("/{base}/{path}"),
    }
}

fn join_route_path(base: &str, path: &str) -> String {
    let joined = join_path_prefix(base, path);
    if joined.is_empty() {
        "/".to_string()
    } else {
        joined
    }
}

fn join_route_name(prefix: &str, name: &str) -> String {
    let prefix = prefix.trim_matches('.');
    let name = name.trim_matches('.');

    match (prefix.is_empty(), name.is_empty()) {
        (true, true) => String::new(),
        (true, false) => name.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}.{name}"),
    }
}

pub struct HttpRegistrar {
    registrations: Vec<HttpRegistration>,
    pub(crate) named_routes: routes::RouteRegistry,
    default_route_options: HttpRouteOptions,
    registration_error: Option<Error>,
}

impl Default for HttpRegistrar {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpRegistrar {
    pub fn new() -> Self {
        Self {
            registrations: Vec::new(),
            named_routes: routes::RouteRegistry::new(),
            default_route_options: HttpRouteOptions::default(),
            registration_error: None,
        }
    }

    pub fn route(&mut self, path: &str, method_router: MethodRouter<AppContext>) -> &mut Self {
        self.route_with_options(path, method_router, HttpRouteOptions::default())
    }

    pub fn route_with_options(
        &mut self,
        path: &str,
        method_router: MethodRouter<AppContext>,
        options: HttpRouteOptions,
    ) -> &mut Self {
        self.push_route_registration(
            None,
            path.to_string(),
            None,
            method_router,
            options.with_defaults(&self.default_route_options),
            true,
        );
        self
    }

    /// Register a named route for URL generation.
    pub fn route_named<I>(
        &mut self,
        name: I,
        path: &str,
        method_router: MethodRouter<AppContext>,
    ) -> &mut Self
    where
        I: Into<RouteId>,
    {
        self.route_named_with_options_and_method(
            name,
            path,
            method_router,
            HttpRouteOptions::default(),
            None,
        )
    }

    /// Register a named route with options.
    pub fn route_named_with_options<I>(
        &mut self,
        name: I,
        path: &str,
        method_router: MethodRouter<AppContext>,
        options: HttpRouteOptions,
    ) -> &mut Self
    where
        I: Into<RouteId>,
    {
        self.route_named_with_options_and_method(name, path, method_router, options, None)
    }

    pub fn get<I, H, T>(
        &mut self,
        name: I,
        path: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        I: Into<RouteId>,
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_method_route(name, path, axum::routing::get(handler), "get", configure)
    }

    pub fn post<I, H, T>(
        &mut self,
        name: I,
        path: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        I: Into<RouteId>,
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_method_route(name, path, axum::routing::post(handler), "post", configure)
    }

    pub fn put<I, H, T>(
        &mut self,
        name: I,
        path: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        I: Into<RouteId>,
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_method_route(name, path, axum::routing::put(handler), "put", configure)
    }

    pub fn patch<I, H, T>(
        &mut self,
        name: I,
        path: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        I: Into<RouteId>,
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_method_route(
            name,
            path,
            axum::routing::patch(handler),
            "patch",
            configure,
        )
    }

    pub fn delete<I, H, T>(
        &mut self,
        name: I,
        path: &str,
        handler: H,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        I: Into<RouteId>,
        H: Handler<T, AppContext>,
        T: 'static,
    {
        self.register_method_route(
            name,
            path,
            axum::routing::delete(handler),
            "delete",
            configure,
        )
    }

    pub fn scope(
        &mut self,
        path: &str,
        f: impl FnOnce(&mut HttpScope<'_>) -> Result<()>,
    ) -> Result<&mut Self> {
        let state = ResolvedHttpScopeState::root(path, &self.default_route_options);
        let result = {
            let mut scope = HttpScope::new(self, state);
            run_http_registration_callback("http scope callback", &mut scope, f)
        };
        self.take_registration_error()?;
        result?;
        Ok(self)
    }

    pub fn nest(&mut self, path: &str, router: HttpRouter) -> &mut Self {
        self.registrations.push(HttpRegistration::Nest {
            path: path.to_string(),
            router,
        });
        self
    }

    fn register_method_route<I>(
        &mut self,
        name: I,
        path: &str,
        method_router: MethodRouter<AppContext>,
        method: &str,
        configure: impl FnOnce(&mut HttpRouteBuilder),
    ) -> &mut Self
    where
        I: Into<RouteId>,
    {
        let mut route = HttpRouteBuilder::root(method);
        if let Err(error) =
            run_http_registration_callback("http route configure callback", &mut route, |route| {
                configure(route);
                Ok(())
            })
        {
            self.record_registration_error(error);
            return self;
        }

        self.route_named_resolved(
            name.into(),
            path,
            Some(method),
            method_router,
            route.finish(),
        );
        self
    }

    pub fn merge(&mut self, router: HttpRouter) -> &mut Self {
        self.registrations.push(HttpRegistration::Merge { router });
        self
    }

    /// Create a route group under a shared path prefix.
    ///
    /// Routes registered inside the closure are nested under `prefix`.
    ///
    /// ```ignore
    /// r.group("/admin", |r| {
    ///     r.route("/dashboard", get(dashboard));  // /admin/dashboard
    ///     r.route("/settings", get(settings));     // /admin/settings
    ///     Ok(())
    /// })?;
    /// ```
    pub fn group(
        &mut self,
        prefix: &str,
        f: impl FnOnce(&mut HttpRegistrar) -> Result<()>,
    ) -> Result<&mut Self> {
        let mut sub = HttpRegistrar::new();
        sub.default_route_options = self.default_route_options.clone();
        let result = run_http_registration_callback("http group callback", &mut sub, f);
        sub.take_registration_error()?;
        result?;
        self.merge_group(prefix, sub)
    }

    /// Create a route group under a shared path prefix with inherited defaults.
    ///
    /// Guard, middleware, rate-limit, and OpenAPI defaults from `options`
    /// apply to every route registered inside the closure.
    pub fn group_with_options(
        &mut self,
        prefix: &str,
        options: HttpRouteOptions,
        f: impl FnOnce(&mut HttpRegistrar) -> Result<()>,
    ) -> Result<&mut Self> {
        let mut sub = HttpRegistrar::new();
        sub.default_route_options = options.with_defaults(&self.default_route_options);
        let result = run_http_registration_callback("http group callback", &mut sub, f);
        sub.take_registration_error()?;
        result?;
        self.merge_group(prefix, sub)
    }

    fn record_registration_error(&mut self, error: Error) {
        if self.registration_error.is_none() {
            self.registration_error = Some(error);
        }
    }

    fn take_registration_error(&mut self) -> Result<()> {
        match self.registration_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn merge_group(&mut self, prefix: &str, sub: HttpRegistrar) -> Result<&mut Self> {
        for registration in sub.registrations {
            match registration {
                HttpRegistration::Route(route) => {
                    let RouteRegistration {
                        name,
                        path,
                        method,
                        method_router,
                        options,
                        inherit_parent_defaults_on_merge,
                    } = *route;
                    let path = join_route_path(prefix, &path);
                    let options = if inherit_parent_defaults_on_merge {
                        options.with_defaults(&self.default_route_options)
                    } else {
                        options
                    };

                    self.push_route_registration(name, path, method, method_router, options, false);
                }
                HttpRegistration::Nest { path, router } => {
                    self.registrations.push(HttpRegistration::Nest {
                        path: join_route_path(prefix, &path),
                        router,
                    });
                }
                HttpRegistration::Merge { router } => {
                    // Merged routers cannot be trivially prefixed, so nest them.
                    self.registrations.push(HttpRegistration::Nest {
                        path: prefix.to_string(),
                        router,
                    });
                }
            }
        }
        // Merge named routes from sub-registrar with prefix applied
        for (name, pattern) in sub.named_routes.iter() {
            self.named_routes
                .register(name.clone(), join_route_path(prefix, pattern));
        }
        Ok(self)
    }

    /// Create an API version group.
    ///
    /// Routes registered inside the closure are nested under `/api/v{version}`.
    ///
    /// ```ignore
    /// r.api_version(1, |r| {
    ///     r.route("/users", get(list_users));   // /api/v1/users
    ///     r.route("/orders", get(list_orders));  // /api/v1/orders
    ///     Ok(())
    /// })?;
    /// ```
    pub fn api_version(
        &mut self,
        version: u32,
        f: impl FnOnce(&mut HttpRegistrar) -> Result<()>,
    ) -> Result<&mut Self> {
        self.group(&format!("/api/v{version}"), f)
    }

    pub fn resource(&mut self, name: &str, path: &str, routes: HttpResourceRoutes) -> &mut Self {
        self.resource_with_options(name, path, routes, HttpRouteOptions::default())
    }

    pub fn resource_with_options(
        &mut self,
        name: &str,
        path: &str,
        routes: HttpResourceRoutes,
        options: HttpRouteOptions,
    ) -> &mut Self {
        if let Some(route) = routes.index {
            self.route_named_with_options(
                RouteId::owned(format!("{name}.index")),
                path,
                route,
                options.clone().document_method("get"),
            );
        }
        if let Some(route) = routes.store {
            self.route_named_with_options(
                RouteId::owned(format!("{name}.store")),
                path,
                route,
                options.clone().document_method("post"),
            );
        }

        let member_path = format!("{path}/:{}", routes.id_param);
        if let Some(route) = routes.show {
            self.route_named_with_options(
                RouteId::owned(format!("{name}.show")),
                &member_path,
                route,
                options.clone().document_method("get"),
            );
        }
        if let Some(route) = routes.update {
            self.route_named_with_options(
                RouteId::owned(format!("{name}.update")),
                &member_path,
                route,
                options.clone().document_method("put"),
            );
        }
        if let Some(route) = routes.destroy {
            self.route_named_with_options(
                RouteId::owned(format!("{name}.destroy")),
                &member_path,
                route,
                options.document_method("delete"),
            );
        }

        self
    }

    /// Collect documented routes for OpenAPI spec generation.
    pub(crate) fn collect_documented_routes(&self) -> Vec<crate::openapi::spec::DocumentedRoute> {
        let mut docs = Vec::new();
        for registration in &self.registrations {
            if let HttpRegistration::Route(route) = registration {
                if let Some(ref doc) = route.options.doc {
                    docs.push(crate::openapi::spec::DocumentedRoute {
                        method: route
                            .method
                            .clone()
                            .or_else(|| doc.method.clone())
                            .unwrap_or_else(|| "get".into()),
                        path: route.path.clone(),
                        doc: doc.clone(),
                    });
                }
            }
        }
        docs
    }

    pub fn into_router(self, app: AppContext) -> Router {
        self.into_router_with_middlewares(app, Vec::new())
    }

    pub fn into_router_with_middlewares(
        self,
        app: AppContext,
        middlewares: Vec<middleware::MiddlewareConfig>,
    ) -> Router {
        let mut router = Router::<AppContext>::new();

        for registration in self.registrations {
            match registration {
                HttpRegistration::Route(route) => {
                    let RouteRegistration {
                        path,
                        method_router,
                        mut options,
                        ..
                    } = *route;
                    let audit_area = options.resolved_audit_area().map(ToOwned::to_owned);
                    let mut route_middlewares = Vec::new();
                    // Expand middleware group if specified
                    if let Some(ref group_name) = options.middleware_group_name {
                        if let Ok(groups) = app.resolve::<middleware::MiddlewareGroups>() {
                            if let Some(group_mws) = groups.get(group_name) {
                                route_middlewares.extend(group_mws.clone());
                            }
                        }
                    }
                    route_middlewares.extend(options.middlewares.clone());
                    if !options.requires_auth() {
                        if let Some(rate_limit) = options.post_auth_rate_limit.take() {
                            if matches!(
                                rate_limit.rate_limit_by(),
                                middleware::RateLimitBy::ActorOrIp
                            ) {
                                route_middlewares.push(rate_limit.build());
                            } else {
                                tracing::warn!(
                                    path = %path,
                                    "foundry: actor-based rate limit skipped on public route"
                                );
                            }
                        }
                    }
                    let method_router = if options.requires_auth() {
                        let post_auth_rl = options.post_auth_rate_limit.as_ref().map(|rl| {
                            middleware::RateLimitState {
                                max: rl.max(),
                                window_secs: rl.window_secs(),
                                key_prefix: rl.key_prefix_str().to_string(),
                                by: rl.rate_limit_by(),
                                store: middleware::create_rate_limit_store(&app),
                            }
                        });
                        method_router.route_layer(axum_middleware::from_fn_with_state(
                            HttpAuthState {
                                app: app.clone(),
                                options,
                                post_auth_rl,
                            },
                            http_auth_middleware,
                        ))
                    } else {
                        method_router
                    };

                    if route_middlewares.is_empty() && audit_area.is_none() {
                        router = router.route(&path, method_router);
                    } else {
                        let mut mini = Router::<AppContext>::new().route(&path, method_router);
                        if !route_middlewares.is_empty() {
                            mini = middleware::apply_ordered_middlewares(
                                mini,
                                route_middlewares,
                                &app,
                            );
                        }
                        if let Some(audit_area) = audit_area {
                            mini = mini.layer(axum_middleware::from_fn_with_state(
                                RouteRequestContextState { audit_area },
                                route_request_context_middleware,
                            ));
                        }
                        router = router.merge(mini);
                    }
                }
                HttpRegistration::Nest {
                    path,
                    router: nested,
                } => {
                    router = router.nest(&path, nested);
                }
                HttpRegistration::Merge { router: merged } => {
                    router = router.merge(merged);
                }
            }
        }

        router = router.layer(axum_middleware::from_fn_with_state(
            app.clone(),
            crate::logging::request_origin_middleware,
        ));

        // Apply user-registered middleware (CORS, security headers, rate limit, etc.)
        router = middleware::apply_ordered_middlewares(router, middlewares, &app);

        router
            .layer(axum_middleware::from_fn_with_state(
                app.clone(),
                crate::logging::request_context_middleware,
            ))
            .with_state(app)
    }

    fn route_named_resolved(
        &mut self,
        name: RouteId,
        path: &str,
        method: Option<&str>,
        method_router: MethodRouter<AppContext>,
        options: HttpRouteOptions,
    ) -> &mut Self {
        self.named_routes.register(name.clone(), path);
        self.push_route_registration(
            Some(name),
            path.to_string(),
            method.map(ToOwned::to_owned),
            method_router,
            options,
            false,
        );
        self
    }

    fn route_named_with_options_and_method<I>(
        &mut self,
        name: I,
        path: &str,
        method_router: MethodRouter<AppContext>,
        options: HttpRouteOptions,
        method: Option<&str>,
    ) -> &mut Self
    where
        I: Into<RouteId>,
    {
        let name = name.into();
        self.named_routes.register(name.clone(), path);
        self.push_route_registration(
            Some(name),
            path.to_string(),
            method.map(ToOwned::to_owned),
            method_router,
            options.with_defaults(&self.default_route_options),
            true,
        );
        self
    }

    fn push_route_registration(
        &mut self,
        name: Option<RouteId>,
        path: String,
        method: Option<String>,
        method_router: MethodRouter<AppContext>,
        options: HttpRouteOptions,
        inherit_parent_defaults_on_merge: bool,
    ) {
        self.registrations
            .push(HttpRegistration::Route(Box::new(RouteRegistration {
                name,
                path,
                method,
                method_router,
                options,
                inherit_parent_defaults_on_merge,
            })));
    }

    pub fn collect_route_manifest(&self) -> Result<Vec<RouteManifestEntry>> {
        let mut manifest = Vec::new();
        let mut route_ids = HashSet::new();

        for registration in &self.registrations {
            let HttpRegistration::Route(route) = registration else {
                continue;
            };
            let Some(id) = &route.name else {
                continue;
            };

            if !route_ids.insert(id.clone()) {
                return Err(Error::message(format!(
                    "route manifest contains duplicate route id `{}`",
                    id.as_str()
                )));
            }

            let doc = route.options.doc.as_ref();
            let method = route
                .method
                .clone()
                .or_else(|| doc.and_then(|doc| doc.method.clone()));
            let mut responses = doc
                .map(|doc| {
                    doc.responses
                        .iter()
                        .map(|(status, schema)| RouteManifestResponse {
                            status: *status,
                            schema: schema.name,
                            schema_json: (schema.schema_fn)(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            responses.sort_by_key(|response| response.status);

            manifest.push(RouteManifestEntry {
                id: id.clone(),
                path: route.path.clone(),
                method,
                params: route_path_params(&route.path),
                client_export: route.options.client_export,
                guard: route.options.guard_id().cloned(),
                permissions: route.options.permissions_set().into_iter().collect(),
                summary: doc.and_then(|doc| doc.summary.clone()),
                request: doc.and_then(|doc| doc.request.as_ref().map(|schema| schema.name)),
                responses,
            });
        }

        manifest.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Ok(manifest)
    }
}

trait DocumentMethod {
    fn document_method(self, method: &str) -> Self;
}

impl DocumentMethod for HttpRouteOptions {
    fn document_method(mut self, method: &str) -> Self {
        mutate_doc(&mut self, |doc| doc.method(method));
        self
    }
}

pub(crate) fn route_path_params(path: &str) -> Vec<String> {
    let mut params = Vec::new();

    for segment in path.split('/') {
        let Some(param) = routes::route_segment_param(segment) else {
            continue;
        };
        if !param.is_empty() && !params.iter().any(|existing| existing == param) {
            params.push(param.to_string());
        }
    }

    params
}

#[derive(Clone)]
struct RouteRequestContextState {
    audit_area: String,
}

#[derive(Clone)]
struct HttpAuthState {
    app: AppContext,
    options: HttpRouteOptions,
    post_auth_rl: Option<middleware::RateLimitState>,
}

async fn route_request_context_middleware(
    State(state): State<RouteRequestContextState>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let current = crate::logging::CurrentRequest::from_parts(&parts)
        .with_audit_area(Some(state.audit_area.clone()));
    parts.extensions.insert(current.clone());
    let request = Request::from_parts(parts, body);

    let mut response =
        crate::logging::scope_current_request(current.clone(), next.run(request)).await;
    response.extensions_mut().insert(current);
    response
}

async fn http_auth_middleware(
    State(state): State<HttpAuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let auth = match state.app.auth() {
        Ok(auth) => auth,
        Err(error) => {
            record_auth_outcome(&state.app, AuthOutcome::Error);
            return internal_error_response(error);
        }
    };
    let authorizer = match state.app.authorizer() {
        Ok(authorizer) => authorizer,
        Err(error) => {
            record_auth_outcome(&state.app, AuthOutcome::Error);
            return internal_error_response(error);
        }
    };
    let actor = match auth
        .authenticate_headers(request.headers(), state.options.guard_id())
        .await
    {
        Ok(actor) => actor,
        Err(error) => {
            record_auth_outcome(&state.app, auth_outcome_from_error(&error));
            return error.into_response();
        }
    };

    if actor_has_mfa_pending(&actor) && !state.options.allow_mfa_pending_token {
        record_auth_outcome(&state.app, AuthOutcome::Forbidden);
        return AuthError::forbidden("Multi-factor authentication verification is required.")
            .into_response();
    }

    let permissions = state.options.permissions_set();
    if let Err(error) = authorizer.authorize_permissions(&actor, &permissions).await {
        record_auth_outcome(&state.app, auth_outcome_from_error(&error));
        return error.into_response();
    }

    if let Some(ref authorize) = state.options.authorize {
        let ctx = HttpAuthorizeContext::new(state.app.clone(), actor.clone());
        if let Err(error) = authorize(ctx).await {
            record_auth_outcome(&state.app, auth_outcome_from_route_error(&error));
            return error.into_response();
        }
    }

    // Post-auth rate limiting (for by_actor / by_actor_or_ip)
    if let Some(ref rl_state) = state.post_auth_rl {
        let client_ip = middleware::extract_client_ip(&request);
        if let Some(rejection) =
            middleware::enforce_rate_limit_for_actor(rl_state, &actor, client_ip).await
        {
            return rejection;
        }
    }

    crate::logging::scope_current_actor(actor.clone(), async move {
        record_auth_outcome(&state.app, AuthOutcome::Success);
        request.extensions_mut().insert(state.app.clone());
        request.extensions_mut().insert(actor.clone());
        let mut response = next.run(request).await;
        response.extensions_mut().insert(actor);
        response
    })
    .await
}

fn internal_error_response(error: Error) -> Response {
    error.into_response()
}

fn auth_outcome_from_error(error: &AuthError) -> AuthOutcome {
    match error {
        AuthError::Unauthorized(_) => AuthOutcome::Unauthorized,
        AuthError::Forbidden(_) => AuthOutcome::Forbidden,
        AuthError::Internal(_) => AuthOutcome::Error,
    }
}

fn auth_outcome_from_route_error(error: &Error) -> AuthOutcome {
    match error {
        Error::Http { status, .. } => match *status {
            401 => AuthOutcome::Unauthorized,
            403 | 404 => AuthOutcome::Forbidden,
            _ => AuthOutcome::Error,
        },
        Error::NotFound(_) => AuthOutcome::Forbidden,
        Error::Message(_) | Error::Other(_) | Error::Validation(_) => AuthOutcome::Error,
    }
}

fn record_auth_outcome(app: &AppContext, outcome: AuthOutcome) {
    if let Ok(diagnostics) = app.diagnostics() {
        diagnostics.record_auth_outcome(outcome);
    }
}

// ---------------------------------------------------------------------------
// Maintenance mode CLI commands (down / up)
// ---------------------------------------------------------------------------

pub(crate) fn maintenance_cli_registrar() -> crate::cli::CommandRegistrar {
    use clap::{Arg, Command};

    use crate::cli::CommandRegistrar;
    use crate::support::runtime::RuntimeBackend;
    use crate::support::CommandId;

    const DOWN_COMMAND: CommandId = CommandId::new("down");
    const UP_COMMAND: CommandId = CommandId::new("up");
    const ROUTES_LIST_COMMAND: CommandId = CommandId::new("routes:list");

    let registrar: CommandRegistrar = Arc::new(|registry| {
        registry.command(
            DOWN_COMMAND,
            Command::new(DOWN_COMMAND.as_str().to_string())
                .about("Put the application into maintenance mode")
                .arg(
                    Arg::new("secret")
                        .long("secret")
                        .value_name("SECRET")
                        .help("Bypass secret for maintenance mode"),
                ),
            |invocation| async move {
                let app = invocation.app();
                let backend = app.resolve::<RuntimeBackend>()?;
                let secret = invocation
                    .matches()
                    .get_one::<String>("secret")
                    .cloned()
                    .unwrap_or_default();

                // Clear any existing key and set fresh
                let _ = backend.del_key("maintenance:active").await;
                backend
                    .set_nx_value("maintenance:active", &secret, 31_536_000)
                    .await?;

                println!("Application is now in maintenance mode.");
                if !secret.is_empty() {
                    println!("Bypass secret: {secret}");
                }
                Ok(())
            },
        )?;

        registry.command(
            UP_COMMAND,
            Command::new(UP_COMMAND.as_str().to_string())
                .about("Bring the application out of maintenance mode"),
            |invocation| async move {
                let app = invocation.app();
                let backend = app.resolve::<RuntimeBackend>()?;
                backend.del_key("maintenance:active").await?;
                println!("Application is now live.");
                Ok(())
            },
        )?;

        registry.command(
            ROUTES_LIST_COMMAND,
            Command::new(ROUTES_LIST_COMMAND.as_str().to_string())
                .about("List all registered named routes"),
            |invocation| async move {
                let app = invocation.app();
                match app.resolve::<routes::RouteRegistry>() {
                    Ok(registry) => {
                        let mut routes: Vec<_> = registry.iter().collect();
                        routes.sort_by_key(|(name, _)| name.as_str());
                        if routes.is_empty() {
                            println!("No named routes registered.");
                        } else {
                            println!("{:<30} PATH", "NAME");
                            println!("{}", "-".repeat(60));
                            for (name, pattern) in routes {
                                println!("{:<30} {}", name, pattern);
                            }
                        }
                    }
                    Err(error) => {
                        println!("Route registry not available: {error}");
                    }
                }
                Ok(())
            },
        )?;

        Ok(())
    });
    registrar
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::future::ready;
    use std::sync::Arc;

    use axum::routing::{delete, get, post, put};

    use super::{
        HttpRegistrar, HttpRegistration, HttpResourceRoutes, HttpRouteOptions, RouteRegistrar,
    };
    use crate::auth::Actor;
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::http::middleware::{RateLimit, RateLimitWindow};
    use crate::support::{GuardId, PermissionId, RouteId};
    use crate::validation::RuleRegistry;

    async fn ok() -> &'static str {
        "ok"
    }

    fn authorize_context() -> super::HttpAuthorizeContext {
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        super::HttpAuthorizeContext::new(app, Actor::new("user-1", GuardId::new("api")))
    }

    fn route_by_path<'a>(registrar: &'a HttpRegistrar, path: &str) -> &'a super::RouteRegistration {
        registrar
            .registrations
            .iter()
            .find_map(|registration| match registration {
                HttpRegistration::Route(route) if route.path == path => Some(route.as_ref()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing route at `{path}`"))
    }

    #[tokio::test]
    async fn http_authorize_future_panic_becomes_error() {
        let authorize = super::wrap_http_authorize_callback(|_ctx| async {
            let should_panic = true;
            if should_panic {
                panic!("route auth boom");
            }
            Ok(())
        });

        let error = authorize(authorize_context()).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("http authorizer panicked: route auth boom"));
    }

    #[tokio::test]
    async fn http_authorize_factory_panic_becomes_error() {
        let authorize = super::wrap_http_authorize_callback(|_ctx| {
            if std::hint::black_box(true) {
                panic!("route auth factory boom");
            }
            ready(Ok(()))
        });

        let error = authorize(authorize_context()).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("http authorizer panicked: route auth factory boom"));
    }

    #[test]
    fn route_registrar_panic_becomes_error() {
        let registrars: Vec<RouteRegistrar> = vec![Arc::new(|_| {
            panic!("route registrar explode");
        })];

        let error = match super::build_registrar(&registrars) {
            Ok(_) => panic!("expected route registrar panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "route registrar panicked: route registrar explode"
        );
    }

    #[test]
    fn group_callback_panic_becomes_error() {
        let mut registrar = HttpRegistrar::new();

        let error = match registrar.group("/api", |_| -> crate::Result<()> {
            panic!("group explode");
        }) {
            Ok(_) => panic!("expected group callback panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "http group callback panicked: group explode"
        );
    }

    #[test]
    fn scope_callback_panic_becomes_error() {
        let mut registrar = HttpRegistrar::new();

        let error = match registrar.scope("/api", |_| -> crate::Result<()> {
            panic!("scope explode");
        }) {
            Ok(_) => panic!("expected scope callback panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "http scope callback panicked: scope explode"
        );
    }

    #[test]
    fn route_configure_callback_panic_becomes_error_without_registering_route() {
        let mut registrar = HttpRegistrar::new();

        let error = match registrar.scope("/api", |routes| {
            routes.get("/users", "users.index", ok, |_| {
                panic!("configure explode");
            });
            Ok(())
        }) {
            Ok(_) => panic!("expected route configure callback panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "http route configure callback panicked: configure explode"
        );
        assert!(registrar.registrations.is_empty());
        assert!(!registrar.named_routes.has(RouteId::new("users.index")));
    }

    #[test]
    fn group_with_options_inherits_guard_and_doc_defaults() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .group_with_options(
                "/api",
                HttpRouteOptions::new()
                    .guard(GuardId::new("api"))
                    .tag("users"),
                |routes| {
                    routes.route_with_options(
                        "/users",
                        get(ok),
                        HttpRouteOptions::new().summary("List users"),
                    );
                    Ok(())
                },
            )
            .unwrap();

        let HttpRegistration::Route(route) = &registrar.registrations[0] else {
            panic!("expected route registration");
        };

        assert_eq!(route.path, "/api/users");
        assert_eq!(route.options.guard_id(), Some(&GuardId::new("api")));

        let doc = route
            .options
            .doc
            .as_ref()
            .expect("route docs should be present");
        assert_eq!(doc.tags, vec!["users".to_string()]);
        assert_eq!(doc.summary.as_deref(), Some("List users"));
    }

    #[test]
    fn group_joins_slashy_prefixes_without_duplicate_separators() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .group("/api/", |routes| {
                routes.route_named(RouteId::new("users.index"), "/users", get(ok));
                Ok(())
            })
            .unwrap();

        assert!(registrar.named_routes.has(RouteId::new("users.index")));
        assert_eq!(
            registrar
                .named_routes
                .url(RouteId::new("users.index"), &[])
                .unwrap()
                .as_str(),
            "/api/users"
        );
        route_by_path(&registrar, "/api/users");
    }

    #[test]
    fn scope_inside_group_with_options_inherits_outer_defaults_and_can_reset_access() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .group_with_options(
                "/api",
                HttpRouteOptions::new()
                    .guard(GuardId::new("api"))
                    .tag("outer"),
                |routes| {
                    routes.scope("/admin", |admin| {
                        admin.name_prefix("admin");
                        admin.get("/health", "health", ok, |_| {});
                        admin.get("/login", "login", ok, |route| {
                            route.public();
                        });
                        Ok(())
                    })?;
                    Ok(())
                },
            )
            .unwrap();

        assert!(registrar.named_routes.has(RouteId::new("admin.health")));
        assert!(registrar.named_routes.has(RouteId::new("admin.login")));

        let health = route_by_path(&registrar, "/api/admin/health");
        let health_doc = health.options.doc.as_ref().expect("docs should exist");
        assert_eq!(health.options.guard_id(), Some(&GuardId::new("api")));
        assert_eq!(health_doc.tags, vec!["outer".to_string()]);

        let login = route_by_path(&registrar, "/api/admin/login");
        let login_doc = login.options.doc.as_ref().expect("docs should exist");
        assert_eq!(login.options.guard_id(), None);
        assert!(login.options.permissions_set().is_empty());
        assert!(!login.options.requires_auth());
        assert_eq!(login_doc.tags, vec!["outer".to_string()]);
    }

    #[test]
    fn resource_registers_common_named_routes() {
        let mut registrar = HttpRegistrar::new();
        registrar.resource_with_options(
            "users",
            "/users",
            HttpResourceRoutes::new()
                .index(get(ok))
                .store(post(ok))
                .show(get(ok))
                .update(put(ok))
                .destroy(delete(ok)),
            HttpRouteOptions::new().guard(GuardId::new("api")),
        );

        assert!(registrar.named_routes.has(RouteId::new("users.index")));
        assert!(registrar.named_routes.has(RouteId::new("users.store")));
        assert!(registrar.named_routes.has(RouteId::new("users.show")));
        assert!(registrar.named_routes.has(RouteId::new("users.update")));
        assert!(registrar.named_routes.has(RouteId::new("users.destroy")));

        let registered_paths = registrar
            .registrations
            .iter()
            .filter_map(|registration| match registration {
                HttpRegistration::Route(route) => Some(route.path.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(registered_paths.contains(&"/users"));
        assert!(registered_paths.contains(&"/users/:id"));
    }

    #[test]
    fn route_path_params_extract_axum_wildcards_and_legacy_segments() {
        assert_eq!(
            super::route_path_params("/users/{id}/files/{*path}/legacy/:token"),
            vec!["id".to_string(), "path".to_string(), "token".to_string()]
        );
        assert_eq!(
            super::route_path_params("/users/{id}/audit/{id}"),
            vec!["id".to_string()]
        );
        assert!(super::route_path_params("/health").is_empty());
    }

    #[test]
    fn route_registry_url_supports_axum_params_and_percent_encodes_values() {
        let mut registry = super::routes::RouteRegistry::new();
        registry.register(
            RouteId::new("files.show"),
            "/files/{*path}/users/{id}/legacy/:token",
        );

        let url = registry
            .url(
                RouteId::new("files.show"),
                &[
                    ("path", "quarter 1/report.pdf"),
                    ("id", "42/99"),
                    ("token", "a b"),
                ],
            )
            .unwrap();

        assert_eq!(
            url,
            "/files/quarter%201%2Freport.pdf/users/42%2F99/legacy/a%20b"
        );
    }

    #[test]
    fn route_registry_url_rejects_missing_params() {
        let mut registry = super::routes::RouteRegistry::new();
        registry.register(RouteId::new("users.show"), "/users/{id}");

        let error = registry
            .url(RouteId::new("users.show"), &[])
            .expect_err("missing params should fail");

        assert!(
            error
                .to_string()
                .contains("route 'users.show' is missing required parameter `id`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn scope_joins_nested_paths_and_relative_route_names() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin.name_prefix("admin");
                admin.scope("/profile", |profile| {
                    profile.name_prefix("profile");
                    profile.put("", "update", ok, |route| {
                        route.summary("Update admin profile");
                    });
                    Ok(())
                })?;
                Ok(())
            })
            .unwrap();

        assert!(registrar
            .named_routes
            .has(RouteId::new("admin.profile.update")));

        let route = route_by_path(&registrar, "/admin/profile");
        let doc = route.options.doc.as_ref().expect("route docs should exist");
        assert_eq!(doc.method.as_deref(), Some("put"));
        assert_eq!(doc.summary.as_deref(), Some("Update admin profile"));
    }

    #[test]
    fn collect_route_manifest_includes_named_route_metadata() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .api_version(1, |routes| {
                routes.scope("/admin", |admin| {
                    admin
                        .name_prefix("admin")
                        .guard(GuardId::new("admin"))
                        .permission(PermissionId::new("users.read"));

                    admin.scope("/users", |users| {
                        users.name_prefix("users");
                        users.get("", "index", ok, |route| {
                            route.summary("List admin users");
                            route.response::<String>(200);
                        });
                        users.get("/{id}", "show", ok, |route| {
                            route.summary("Show admin user");
                            route.request::<String>();
                            route.response::<String>(200);
                        });
                        Ok(())
                    })?;

                    Ok(())
                })?;

                Ok(())
            })
            .unwrap();

        let manifest = registrar.collect_route_manifest().unwrap();
        let index = manifest
            .iter()
            .find(|route| route.id == RouteId::new("admin.users.index"))
            .expect("index route manifest entry");
        assert_eq!(index.path, "/api/v1/admin/users");
        assert_eq!(index.method.as_deref(), Some("get"));
        assert!(index.params.is_empty());
        assert!(index.client_export);
        assert_eq!(index.guard, Some(GuardId::new("admin")));
        assert_eq!(index.permissions, vec![PermissionId::new("users.read")]);
        assert_eq!(index.summary.as_deref(), Some("List admin users"));

        let show = manifest
            .iter()
            .find(|route| route.id == RouteId::new("admin.users.show"))
            .expect("show route manifest entry");
        assert_eq!(show.path, "/api/v1/admin/users/{id}");
        assert_eq!(show.params, vec!["id".to_string()]);
        assert_eq!(show.request, Some("String"));
        assert_eq!(show.responses.len(), 1);
        assert_eq!(show.responses[0].status, 200);
        assert_eq!(show.responses[0].schema, "String");
    }

    #[test]
    fn collect_route_manifest_includes_client_export_opt_out() {
        let mut registrar = HttpRegistrar::new();
        registrar.route_named_with_options(
            RouteId::new("login"),
            "/login",
            post(ok),
            HttpRouteOptions::new()
                .request::<String>()
                .response::<String>(200)
                .without_client_export(),
        );

        let manifest = registrar.collect_route_manifest().unwrap();
        let login = manifest
            .iter()
            .find(|route| route.id == RouteId::new("login"))
            .expect("login route manifest entry");

        assert!(!login.client_export);
        assert_eq!(login.request, Some("String"));
    }

    #[test]
    fn collect_route_manifest_rejects_duplicate_route_ids() {
        let mut registrar = HttpRegistrar::new();
        registrar.route_named(RouteId::new("health"), "/health", get(ok));
        registrar.route_named(RouteId::new("health"), "/healthz", get(ok));

        let error = registrar
            .collect_route_manifest()
            .expect_err("duplicate route ids should fail");

        assert!(
            error
                .to_string()
                .contains("route manifest contains duplicate route id `health`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn scope_inherits_defaults_across_nested_scopes() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin
                    .name_prefix("admin")
                    .guard(GuardId::new("admin"))
                    .audit_area("admin")
                    .permission(PermissionId::new("users.view"))
                    .middleware_group("api")
                    .rate_limit(RateLimit::new(60).per_minute().by_actor())
                    .tag("admin:users")
                    .summary("Admin users")
                    .description("Inherited from scope")
                    .deprecated();

                admin.scope("/users", |users| {
                    users.name_prefix("users");
                    users.get("/:id", "show", ok, |_| {});
                    Ok(())
                })?;

                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/users/:id");
        let doc = route.options.doc.as_ref().expect("route docs should exist");
        let rate_limit = route
            .options
            .post_auth_rate_limit
            .as_ref()
            .expect("scope rate limit should be inherited");

        assert_eq!(route.options.guard_id(), Some(&GuardId::new("admin")));
        assert_eq!(route.options.resolved_audit_area(), Some("admin"));
        assert_eq!(
            route.options.permissions_set(),
            BTreeSet::from([PermissionId::new("users.view")])
        );
        assert_eq!(route.options.middleware_group_name.as_deref(), Some("api"));
        assert_eq!(rate_limit.max(), 60);
        assert!(matches!(rate_limit.window(), RateLimitWindow::Minute));
        assert_eq!(doc.method.as_deref(), Some("get"));
        assert_eq!(doc.tags, vec!["admin:users".to_string()]);
        assert_eq!(doc.summary.as_deref(), Some("Admin users"));
        assert_eq!(doc.description.as_deref(), Some("Inherited from scope"));
        assert!(doc.deprecated);
    }

    #[test]
    fn route_public_clears_inherited_scope_access() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin
                    .name_prefix("admin")
                    .guard(GuardId::new("admin"))
                    .audit_area("admin")
                    .permission(PermissionId::new("admin.access"))
                    .authorize(|_ctx| async { Ok(()) });

                admin.get("/login", "login", ok, |route| {
                    route.public();
                });
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/login");
        assert_eq!(route.options.guard_id(), None);
        assert_eq!(route.options.resolved_audit_area(), Some("admin"));
        assert!(route.options.permissions_set().is_empty());
        assert!(route.options.authorize.is_none());
        assert!(!route.options.requires_auth());
    }

    #[test]
    fn child_scope_public_clears_inherited_parent_access() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin
                    .name_prefix("admin")
                    .guard(GuardId::new("admin"))
                    .audit_area("admin")
                    .permission(PermissionId::new("admin.access"));

                admin.scope("/auth", |auth| {
                    auth.name_prefix("auth").public();
                    auth.post("/login", "login", ok, |_| {});
                    Ok(())
                })?;

                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/auth/login");
        assert_eq!(route.options.guard_id(), None);
        assert_eq!(route.options.resolved_audit_area(), Some("admin"));
        assert!(route.options.permissions_set().is_empty());
        assert!(!route.options.requires_auth());
    }

    #[test]
    fn route_can_disable_inherited_audit_area() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin.name_prefix("admin").audit_area("admin");
                admin.get("/health", "health", ok, |route| {
                    route.audit_disabled();
                });
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/health");
        assert!(matches!(
            route.options.audit_area,
            super::AuditAreaSetting::Disabled
        ));
        assert_eq!(route.options.resolved_audit_area(), None);
    }

    #[test]
    fn child_scope_can_override_inherited_audit_area() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin.name_prefix("admin").audit_area("admin");
                admin.scope("/support", |support| {
                    support.name_prefix("support").audit_area("support");
                    support.get("/tickets", "tickets", ok, |_| {});
                    Ok(())
                })?;
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/support/tickets");
        assert_eq!(route.options.resolved_audit_area(), Some("support"));
    }

    #[test]
    fn scope_authorize_is_inherited_by_routes() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin
                    .name_prefix("admin")
                    .guard(GuardId::new("admin"))
                    .authorize(|_ctx| async { Ok(()) });

                admin.get("/dashboard", "dashboard", ok, |_| {});
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/dashboard");
        assert_eq!(route.options.guard_id(), Some(&GuardId::new("admin")));
        assert!(route.options.authorize.is_some());
        assert!(route.options.requires_auth());
    }

    #[test]
    fn route_builder_authorize_marks_route_as_authenticated() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/reports", |reports| {
                reports.name_prefix("reports");
                reports.get("/audit", "audit", ok, |route| {
                    route.authorize(|_ctx| async { Ok(()) });
                });
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/reports/audit");
        assert_eq!(route.options.guard_id(), None);
        assert!(route.options.authorize.is_some());
        assert!(route.options.requires_auth());
    }

    #[test]
    fn route_overrides_guard_and_adds_permissions() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin
                    .name_prefix("admin")
                    .guard(GuardId::new("admin"))
                    .permission(PermissionId::new("users.view"))
                    .tag("admin:users");

                admin.get("/users/:id", "show", ok, |route| {
                    route.guard(GuardId::new("support"));
                    route.permission(PermissionId::new("users.edit"));
                });
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/users/:id");
        assert_eq!(route.options.guard_id(), Some(&GuardId::new("support")));
        assert_eq!(
            route.options.permissions_set(),
            BTreeSet::from([
                PermissionId::new("users.edit"),
                PermissionId::new("users.view"),
            ])
        );
    }

    #[test]
    fn route_permissions_replace_inherited_permissions_and_tags_replace_inherited_tags() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/admin", |admin| {
                admin
                    .name_prefix("admin")
                    .guard(GuardId::new("admin"))
                    .permission(PermissionId::new("users.view"))
                    .tag("admin:users");

                admin.get("/users/:id/audit", "audit", ok, |route| {
                    route.permissions([PermissionId::new("users.manage")]);
                    route.tag("custom:users");
                    route.tag("custom:audit");
                });
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/admin/users/:id/audit");
        let doc = route.options.doc.as_ref().expect("route docs should exist");

        assert_eq!(
            route.options.permissions_set(),
            BTreeSet::from([PermissionId::new("users.manage")])
        );
        assert_eq!(
            doc.tags,
            vec!["custom:users".to_string(), "custom:audit".to_string()]
        );
    }

    #[test]
    fn verb_helpers_populate_method_and_preserve_request_response_docs() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .scope("/users", |users| {
                users.name_prefix("users");
                users.patch("/:id", "update", ok, |route| {
                    route.summary("Patch user");
                    route.request::<String>();
                    route.response::<String>(200);
                });
                Ok(())
            })
            .unwrap();

        let route = route_by_path(&registrar, "/users/:id");
        let doc = route.options.doc.as_ref().expect("route docs should exist");

        assert_eq!(doc.method.as_deref(), Some("patch"));
        assert_eq!(doc.summary.as_deref(), Some("Patch user"));
        assert_eq!(
            doc.request.as_ref().map(|schema| schema.name),
            Some("String")
        );
        assert_eq!(doc.responses.len(), 1);
        assert_eq!(doc.responses[0].0, 200);
        assert_eq!(doc.responses[0].1.name, "String");
    }

    #[test]
    fn scope_dsl_registers_starter_style_routes_and_openapi_docs() {
        let mut registrar = HttpRegistrar::new();
        registrar
            .api_version(1, |routes| {
                routes.scope("/admin", |admin| {
                    admin.name_prefix("admin");

                    admin.scope("/auth", |auth| {
                        auth.name_prefix("auth").tag("admin:auth");

                        auth.post("/login", "login", ok, |route| {
                            route.public();
                            route.summary("Admin login");
                            route.request::<String>();
                            route.response::<String>(200);
                        });

                        auth.get("/me", "me", ok, |route| {
                            route.guard(GuardId::new("admin"));
                            route.summary("Get authenticated admin profile");
                            route.response::<String>(200);
                        });

                        Ok(())
                    })?;

                    admin.scope("/profile", |profile| {
                        profile
                            .name_prefix("profile")
                            .tag("admin:profile")
                            .guard(GuardId::new("admin"));

                        profile.put("", "update", ok, |route| {
                            route.summary("Update admin profile");
                            route.request::<String>();
                            route.response::<String>(200);
                        });

                        Ok(())
                    })?;

                    Ok(())
                })?;

                Ok(())
            })
            .unwrap();

        assert!(registrar.named_routes.has(RouteId::new("admin.auth.login")));
        assert!(registrar.named_routes.has(RouteId::new("admin.auth.me")));
        assert!(registrar
            .named_routes
            .has(RouteId::new("admin.profile.update")));

        let docs = registrar.collect_documented_routes();
        let spec = crate::openapi::spec::generate_openapi_spec("Foundry", "1.0.0", &docs);

        assert_eq!(
            spec["paths"]["/api/v1/admin/auth/login"]["post"]["summary"],
            "Admin login"
        );
        assert_eq!(
            spec["paths"]["/api/v1/admin/auth/login"]["post"]["tags"][0],
            "admin:auth"
        );
        assert_eq!(
            spec["paths"]["/api/v1/admin/profile"]["put"]["summary"],
            "Update admin profile"
        );
        assert_eq!(
            spec["paths"]["/api/v1/admin/profile"]["put"]["tags"][0],
            "admin:profile"
        );
    }
}
