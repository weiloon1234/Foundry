pub mod email_verification;
pub mod lockout;
pub mod mfa;
pub mod password_reset;
pub mod session;
pub mod token;
pub(crate) mod token_store;

pub(crate) use token::builtin_cli_registrar;

use std::any::Any;
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AuthConfig;
use crate::database::{ColumnRef, ComparisonOp, DbType, DbValue, Expr, Model, QueryExecutor};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_async_panic, panic_payload_message};
use crate::support::sync::lock_unpoisoned;
use crate::support::{GuardId, ModelId, PermissionId, PolicyId, RoleId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AccessScope {
    #[default]
    Public,
    Guarded(GuardedAccess),
}

impl AccessScope {
    pub fn requires_auth(&self) -> bool {
        matches!(self, Self::Guarded(_))
    }

    pub fn guard(&self) -> Option<&GuardId> {
        match self {
            Self::Public => None,
            Self::Guarded(access) => access.guard.as_ref(),
        }
    }

    pub fn permissions(&self) -> BTreeSet<PermissionId> {
        match self {
            Self::Public => BTreeSet::new(),
            Self::Guarded(access) => access.permissions.clone(),
        }
    }

    pub fn with_guard<I>(mut self, guard: I) -> Self
    where
        I: Into<GuardId>,
    {
        self.guarded_mut().guard = Some(guard.into());
        self
    }

    pub fn with_permission<I>(mut self, permission: I) -> Self
    where
        I: Into<PermissionId>,
    {
        self.guarded_mut().permissions.insert(permission.into());
        self
    }

    pub fn with_permissions<I, P>(mut self, permissions: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PermissionId>,
    {
        let access = self.guarded_mut();
        access.permissions = permissions.into_iter().map(Into::into).collect();
        self
    }

    fn guarded_mut(&mut self) -> &mut GuardedAccess {
        if !matches!(self, Self::Guarded(_)) {
            *self = Self::Guarded(GuardedAccess::default());
        }

        match self {
            Self::Public => unreachable!("access scope should be guarded"),
            Self::Guarded(access) => access,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardedAccess {
    pub guard: Option<GuardId>,
    pub permissions: BTreeSet<PermissionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Actor {
    pub id: String,
    pub guard: GuardId,
    #[serde(default)]
    pub roles: BTreeSet<RoleId>,
    #[serde(default)]
    pub permissions: BTreeSet<PermissionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claims: Option<Value>,
}

impl Actor {
    pub fn new<I, G>(id: I, guard: G) -> Self
    where
        I: Into<String>,
        G: Into<GuardId>,
    {
        Self {
            id: id.into(),
            guard: guard.into(),
            roles: BTreeSet::new(),
            permissions: BTreeSet::new(),
            claims: None,
        }
    }

    pub fn with_guard<I>(mut self, guard: I) -> Self
    where
        I: Into<GuardId>,
    {
        self.guard = guard.into();
        self
    }

    pub fn with_roles<I, R>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = R>,
        R: Into<RoleId>,
    {
        self.roles = roles.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_permissions<I, P>(mut self, permissions: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PermissionId>,
    {
        self.permissions = permissions.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_claims(mut self, claims: Value) -> Self {
        self.claims = Some(claims);
        self
    }

    pub fn has_role<I>(&self, role: I) -> bool
    where
        I: Into<RoleId>,
    {
        self.roles.contains(&role.into())
    }

    pub fn has_permission<I>(&self, permission: I) -> bool
    where
        I: Into<PermissionId>,
    {
        self.permissions.contains(&permission.into())
    }

    /// Resolve this actor to its backing model.
    ///
    /// Returns the model instance if found, `None` if the actor's ID
    /// does not match any record, or an error if the guard doesn't match.
    ///
    /// ```ignore
    /// let user = actor.resolve::<User>(&app).await?;
    /// ```
    pub async fn resolve<M>(&self, app: &AppContext) -> Result<Option<M>>
    where
        M: Authenticatable,
    {
        if self.guard != M::guard() {
            return Err(Error::message(format!(
                "actor guard `{}` does not match authenticatable guard `{}`",
                self.guard,
                M::guard()
            )));
        }
        let db = app.database()?;
        M::resolve_from_actor(self, db.as_ref()).await
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthErrorCode {
    InvalidBearerToken,
    InvalidRefreshToken,
    MissingSessionCookie,
    InvalidSession,
    MissingAuthorizationHeader,
    InvalidAuthorizationHeader,
    InvalidAuthorizationScheme,
    MissingBearerToken,
    MissingAuthCredentials,
    MissingRequiredPermission,
    AuthenticatedActorNotFound,
    AuthenticatedModelNotFound,
    MaxConnectionsPerUserExceeded,
}

impl AuthErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidBearerToken => "invalid_bearer_token",
            Self::InvalidRefreshToken => "invalid_refresh_token",
            Self::MissingSessionCookie => "missing_session_cookie",
            Self::InvalidSession => "invalid_session",
            Self::MissingAuthorizationHeader => "missing_authorization_header",
            Self::InvalidAuthorizationHeader => "invalid_authorization_header",
            Self::InvalidAuthorizationScheme => "invalid_authorization_scheme",
            Self::MissingBearerToken => "missing_bearer_token",
            Self::MissingAuthCredentials => "missing_auth_credentials",
            Self::MissingRequiredPermission => "missing_required_permission",
            Self::AuthenticatedActorNotFound => "authenticated_actor_not_found",
            Self::AuthenticatedModelNotFound => "authenticated_model_not_found",
            Self::MaxConnectionsPerUserExceeded => "max_connections_per_user_exceeded",
        }
    }

    pub const fn translation_key(self) -> &'static str {
        match self {
            Self::InvalidBearerToken => "auth.invalid_bearer_token",
            Self::InvalidRefreshToken => "auth.invalid_refresh_token",
            Self::MissingSessionCookie => "auth.missing_session_cookie",
            Self::InvalidSession => "auth.invalid_session",
            Self::MissingAuthorizationHeader => "auth.missing_authorization_header",
            Self::InvalidAuthorizationHeader => "auth.invalid_authorization_header",
            Self::InvalidAuthorizationScheme => "auth.invalid_authorization_scheme",
            Self::MissingBearerToken => "auth.missing_bearer_token",
            Self::MissingAuthCredentials => "auth.missing_auth_credentials",
            Self::MissingRequiredPermission => "auth.missing_required_permission",
            Self::AuthenticatedActorNotFound => "auth.authenticated_actor_not_found",
            Self::AuthenticatedModelNotFound => "auth.authenticated_model_not_found",
            Self::MaxConnectionsPerUserExceeded => "auth.max_connections_per_user_exceeded",
        }
    }

    pub const fn default_message(self) -> &'static str {
        match self {
            Self::InvalidBearerToken => "The bearer token is invalid.",
            Self::InvalidRefreshToken => "The refresh token is invalid or expired.",
            Self::MissingSessionCookie => "The session cookie is missing.",
            Self::InvalidSession => "The session is invalid or has expired.",
            Self::MissingAuthorizationHeader => "The Authorization header is missing.",
            Self::InvalidAuthorizationHeader => "The Authorization header is invalid.",
            Self::InvalidAuthorizationScheme => "The Authorization scheme is invalid.",
            Self::MissingBearerToken => "The bearer token is missing.",
            Self::MissingAuthCredentials => "Authentication credentials are required.",
            Self::MissingRequiredPermission => "You do not have permission to perform this action.",
            Self::AuthenticatedActorNotFound => "The authenticated actor could not be resolved.",
            Self::AuthenticatedModelNotFound => "The authenticated model could not be resolved.",
            Self::MaxConnectionsPerUserExceeded => {
                "You have reached the maximum number of allowed connections."
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthErrorMessage {
    message: String,
    code: Option<AuthErrorCode>,
}

impl AuthErrorMessage {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }

    pub fn from_code(code: AuthErrorCode) -> Self {
        Self {
            message: code.default_message().to_string(),
            code: Some(code),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn code(&self) -> Option<AuthErrorCode> {
        self.code
    }
}

impl std::fmt::Display for AuthErrorMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    Unauthorized(AuthErrorMessage),
    #[error("{0}")]
    Forbidden(AuthErrorMessage),
    #[error("{0}")]
    Internal(String),
}

impl AuthError {
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(AuthErrorMessage::new(message))
    }

    pub fn unauthorized_code(code: AuthErrorCode) -> Self {
        Self::Unauthorized(AuthErrorMessage::from_code(code))
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(AuthErrorMessage::new(message))
    }

    pub fn forbidden_code(code: AuthErrorCode) -> Self {
        Self::Forbidden(AuthErrorMessage::from_code(code))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> Option<AuthErrorCode> {
        match self {
            Self::Unauthorized(message) | Self::Forbidden(message) => message.code(),
            Self::Internal(_) => None,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Unauthorized(message) | Self::Forbidden(message) => message.message(),
            Self::Internal(message) => message,
        }
    }

    fn public_message(&self) -> &str {
        match self {
            Self::Internal(_) => Error::internal_server_error_message(),
            Self::Unauthorized(_) | Self::Forbidden(_) => self.message(),
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "message": self.public_message(),
            "status": self.status_code().as_u16(),
        });

        if let Some(code) = self.code() {
            payload["error_code"] = serde_json::Value::String(code.as_str().to_string());
            payload["message_key"] = serde_json::Value::String(code.translation_key().to_string());
        }

        payload
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let error_text = self.message().to_string();
        let mut response = (status, Json(self.payload())).into_response();
        if status.is_server_error() {
            crate::logging::mark_handler_error_response(
                &mut response,
                status.as_u16(),
                error_text,
                Vec::new(),
            );
        }
        response
    }
}

#[async_trait]
pub trait BearerAuthenticator: Send + Sync + 'static {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>>;
}

#[async_trait]
pub trait Policy: Send + Sync + 'static {
    async fn evaluate(&self, actor: &Actor, app: &AppContext) -> Result<bool>;
}

/// A model that can be resolved from an authenticated [`Actor`].
///
/// Implement this on your User, Admin, or Merchant model to enable
/// `actor.resolve::<User>(&app).await` — similar to Laravel's
/// `$request->user()` returning the backing Eloquent model.
///
/// Each `Authenticatable` model is associated with exactly one guard.
/// Multiple models may back different guards (e.g., `"api"` → `User`,
/// `"admin"` → `AdminUser`), but two models cannot share the same guard.
#[async_trait]
pub trait Authenticatable: Model + Sized + Send + Sync + 'static {
    /// The guard this model backs.
    fn guard() -> GuardId;

    /// Resolve the model instance from the actor's ID.
    ///
    /// Override this to add eager loading, active-status checks, or
    /// any custom resolution logic.
    async fn resolve_from_actor<E>(actor: &Actor, executor: &E) -> Result<Option<Self>>
    where
        E: QueryExecutor,
    {
        resolve_authenticatable_from_primary_key::<Self, E>(actor, executor).await
    }
}

async fn resolve_authenticatable_from_primary_key<M, E>(
    actor: &Actor,
    executor: &E,
) -> Result<Option<M>>
where
    M: Authenticatable,
    E: QueryExecutor,
{
    let table = M::table_meta();
    let primary_key = table.primary_key_column_info().ok_or_else(|| {
        Error::message(format!(
            "model `{}` is missing primary key column metadata",
            std::any::type_name::<M>()
        ))
    })?;

    let Some(actor_id) = actor_id_to_primary_key_value::<M>(&actor.id, primary_key.db_type)? else {
        return Ok(None);
    };

    let column = ColumnRef::new(table.name(), table.primary_key_name()).typed(primary_key.db_type);
    M::model_query()
        .where_(crate::database::Condition::compare(
            Expr::column(column),
            ComparisonOp::Eq,
            Expr::value(actor_id),
        ))
        .first(executor)
        .await
}

fn actor_id_to_primary_key_value<M>(actor_id: &str, db_type: DbType) -> Result<Option<DbValue>>
where
    M: Model,
{
    let value = match db_type {
        DbType::Uuid => match ModelId::<M>::parse_str(actor_id) {
            Ok(id) => Some(DbValue::Uuid(id.into_uuid())),
            Err(_) => None,
        },
        DbType::Text => Some(DbValue::Text(actor_id.to_string())),
        DbType::Int16 => actor_id.parse::<i16>().ok().map(DbValue::Int16),
        DbType::Int32 => actor_id.parse::<i32>().ok().map(DbValue::Int32),
        DbType::Int64 => actor_id.parse::<i64>().ok().map(DbValue::Int64),
        unsupported => {
            return Err(Error::message(format!(
            "default Authenticatable resolution does not support primary key type `{:?}` for `{}`",
            unsupported,
            std::any::type_name::<M>()
        )))
        }
    };

    Ok(value)
}

pub(crate) type GuardRegistryHandle = Arc<Mutex<GuardRegistryBuilder>>;
pub(crate) type PolicyRegistryHandle = Arc<Mutex<PolicyRegistryBuilder>>;
pub(crate) type AuthenticatableRegistryHandle = Arc<Mutex<AuthenticatableRegistryBuilder>>;

/// Internal enum distinguishing bearer (token) from session (cookie) guard drivers.
#[derive(Clone)]
pub(crate) enum GuardAuthenticator {
    Bearer(Arc<dyn BearerAuthenticator>),
    Session(Arc<session::SessionManager>),
}

#[derive(Default)]
pub(crate) struct GuardRegistryBuilder {
    guards: HashMap<GuardId, GuardAuthenticator>,
}

impl GuardRegistryBuilder {
    pub(crate) fn shared() -> GuardRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.guards.keys().any(|k| k.as_ref() == id)
    }

    pub(crate) fn register_arc<I>(
        &mut self,
        id: I,
        guard: Arc<dyn BearerAuthenticator>,
    ) -> Result<()>
    where
        I: Into<GuardId>,
    {
        let id = id.into();
        if self.guards.contains_key(&id) {
            return Err(Error::message(format!(
                "auth guard `{id}` already registered"
            )));
        }
        self.guards.insert(id, GuardAuthenticator::Bearer(guard));
        Ok(())
    }

    pub(crate) fn register_session<I>(
        &mut self,
        id: I,
        manager: Arc<session::SessionManager>,
    ) -> Result<()>
    where
        I: Into<GuardId>,
    {
        let id = id.into();
        if self.guards.contains_key(&id) {
            return Err(Error::message(format!(
                "auth guard `{id}` already registered"
            )));
        }
        self.guards.insert(id, GuardAuthenticator::Session(manager));
        Ok(())
    }

    pub(crate) fn freeze_shared(
        handle: GuardRegistryHandle,
    ) -> HashMap<GuardId, GuardAuthenticator> {
        let mut builder = lock_unpoisoned(&handle, "guard registry");
        std::mem::take(&mut builder.guards)
    }
}

#[derive(Default)]
pub(crate) struct PolicyRegistryBuilder {
    policies: HashMap<PolicyId, Arc<dyn Policy>>,
}

impl PolicyRegistryBuilder {
    pub(crate) fn shared() -> PolicyRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register_arc<I>(&mut self, id: I, policy: Arc<dyn Policy>) -> Result<()>
    where
        I: Into<PolicyId>,
    {
        let id = id.into();
        if self.policies.contains_key(&id) {
            return Err(Error::message(format!(
                "auth policy `{id}` already registered"
            )));
        }
        self.policies.insert(id, policy);
        Ok(())
    }

    pub(crate) fn freeze_shared(
        handle: PolicyRegistryHandle,
    ) -> HashMap<PolicyId, Arc<dyn Policy>> {
        let mut builder = lock_unpoisoned(&handle, "policy registry");
        std::mem::take(&mut builder.policies)
    }
}

type ErasedResolver = Arc<
    dyn for<'a> Fn(
            &'a Actor,
            &'a AppContext,
        ) -> Pin<
            Box<dyn Future<Output = Result<Option<Box<dyn Any + Send + Sync>>>> + Send + 'a>,
        > + Send
        + Sync,
>;

#[derive(Default)]
pub(crate) struct AuthenticatableRegistryBuilder {
    resolvers: HashMap<GuardId, ErasedResolver>,
}

impl AuthenticatableRegistryBuilder {
    pub(crate) fn shared() -> AuthenticatableRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register<M>(&mut self) -> Result<()>
    where
        M: Authenticatable,
    {
        let guard = M::guard();
        if self.resolvers.contains_key(&guard) {
            return Err(Error::message(format!(
                "authenticatable guard `{guard}` already registered"
            )));
        }

        let resolver: ErasedResolver = Arc::new(|actor: &Actor, app: &AppContext| {
            let db = app.database();
            let actor_clone = actor.clone();
            Box::pin(async move {
                let db = db?;
                let model = M::resolve_from_actor(&actor_clone, db.as_ref()).await?;
                Ok(model.map(|m| Box::new(m) as Box<dyn Any + Send + Sync>))
            })
        });

        self.resolvers.insert(guard, resolver);
        Ok(())
    }

    pub(crate) fn freeze_shared(handle: AuthenticatableRegistryHandle) -> AuthenticatableRegistry {
        let mut builder = lock_unpoisoned(&handle, "authenticatable registry");
        AuthenticatableRegistry {
            resolvers: std::mem::take(&mut builder.resolvers),
        }
    }
}

pub struct AuthenticatableRegistry {
    resolvers: HashMap<GuardId, ErasedResolver>,
}

impl AuthenticatableRegistry {
    /// Resolve an actor to its backing model using the type-erased registry.
    ///
    /// This is useful for dynamic dispatch when the concrete type isn't known
    /// at compile time (e.g., queued export jobs). For typed resolution,
    /// prefer [`Actor::resolve`].
    pub async fn resolve_dynamic(
        &self,
        actor: &Actor,
        app: &AppContext,
    ) -> Result<Option<Box<dyn Any + Send + Sync>>> {
        let Some(resolver) = self.resolvers.get(&actor.guard) else {
            return Err(Error::message(format!(
                "no authenticatable model registered for guard `{}`",
                actor.guard
            )));
        };
        resolver(actor, app).await
    }

    /// Check whether a guard has a registered authenticatable model.
    pub fn contains_guard(&self, guard: &GuardId) -> bool {
        self.resolvers.contains_key(guard)
    }
}

#[derive(Clone)]
pub struct AuthManager {
    config: AuthConfig,
    guards: Arc<HashMap<GuardId, GuardAuthenticator>>,
}

impl AuthManager {
    pub(crate) fn new(config: AuthConfig, guards: HashMap<GuardId, GuardAuthenticator>) -> Self {
        Self {
            config,
            guards: Arc::new(guards),
        }
    }

    pub fn default_guard(&self) -> &GuardId {
        &self.config.default_guard
    }

    /// Authenticate a request using the appropriate strategy for the guard.
    ///
    /// For bearer guards: reads the `Authorization` header.
    /// For session guards: reads the session cookie from the `Cookie` header.
    pub async fn authenticate_headers(
        &self,
        headers: &HeaderMap,
        guard: Option<&GuardId>,
    ) -> std::result::Result<Actor, AuthError> {
        let guard_id = guard
            .cloned()
            .unwrap_or_else(|| self.default_guard().clone());

        let Some(authenticator) = self.guards.get(&guard_id).cloned() else {
            return Err(AuthError::internal(format!(
                "auth guard `{guard_id}` is not registered"
            )));
        };

        let actor = match authenticator {
            GuardAuthenticator::Bearer(bearer) => {
                let token = self.extract_token(headers)?;
                run_bearer_authenticator(&guard_id, bearer, &token)
                    .await?
                    .ok_or_else(|| {
                        AuthError::unauthorized_code(AuthErrorCode::InvalidBearerToken)
                    })?
            }
            GuardAuthenticator::Session(session_manager) => {
                let session_id = session_manager.extract_session_id(headers).ok_or_else(|| {
                    AuthError::unauthorized_code(AuthErrorCode::MissingSessionCookie)
                })?;
                session_manager
                    .validate(&session_id)
                    .await
                    .map_err(|error| AuthError::internal(error.to_string()))?
                    .ok_or_else(|| AuthError::unauthorized_code(AuthErrorCode::InvalidSession))?
            }
        };

        Ok(actor.with_guard(guard_id))
    }

    pub async fn authenticate_token(
        &self,
        token: &str,
        guard: Option<&GuardId>,
    ) -> std::result::Result<Actor, AuthError> {
        let guard_id = guard
            .cloned()
            .unwrap_or_else(|| self.default_guard().clone());
        let Some(authenticator) = self.guards.get(&guard_id).cloned() else {
            return Err(AuthError::internal(format!(
                "auth guard `{guard_id}` is not registered"
            )));
        };

        match authenticator {
            GuardAuthenticator::Bearer(bearer) => {
                let Some(actor) = run_bearer_authenticator(&guard_id, bearer, token).await? else {
                    return Err(AuthError::unauthorized_code(
                        AuthErrorCode::InvalidBearerToken,
                    ));
                };
                Ok(actor.with_guard(guard_id))
            }
            GuardAuthenticator::Session(_) => Err(AuthError::internal(format!(
                "guard `{guard_id}` uses session authentication; use a bearer token guard or authenticate via cookies instead"
            ))),
        }
    }

    pub fn extract_token(&self, headers: &HeaderMap) -> std::result::Result<String, AuthError> {
        let Some(value) = headers.get(header::AUTHORIZATION) else {
            return Err(AuthError::unauthorized_code(
                AuthErrorCode::MissingAuthorizationHeader,
            ));
        };
        let value = value
            .to_str()
            .map_err(|_| AuthError::unauthorized_code(AuthErrorCode::InvalidAuthorizationHeader))?;
        let prefix = self.config.bearer_prefix.trim();
        let expected = format!("{prefix} ");
        if !value
            .get(..expected.len())
            .map(|actual| actual.eq_ignore_ascii_case(&expected))
            .unwrap_or(false)
        {
            return Err(AuthError::unauthorized_code(
                AuthErrorCode::InvalidAuthorizationScheme,
            ));
        }
        let token = value[expected.len()..].trim();
        if token.is_empty() {
            return Err(AuthError::unauthorized_code(
                AuthErrorCode::MissingBearerToken,
            ));
        }
        Ok(token.to_string())
    }
}

#[derive(Clone)]
pub struct Authorizer {
    app: AppContext,
    policies: Arc<HashMap<PolicyId, Arc<dyn Policy>>>,
}

impl Authorizer {
    pub(crate) fn new(app: AppContext, policies: HashMap<PolicyId, Arc<dyn Policy>>) -> Self {
        Self {
            app,
            policies: Arc::new(policies),
        }
    }

    pub fn allows_permission(&self, actor: &Actor, permission: &PermissionId) -> bool {
        actor.permissions.contains(permission)
    }

    pub fn allows_permissions(&self, actor: &Actor, permissions: &BTreeSet<PermissionId>) -> bool {
        permissions
            .iter()
            .all(|permission| self.allows_permission(actor, permission))
    }

    pub async fn authorize_permissions(
        &self,
        actor: &Actor,
        permissions: &BTreeSet<PermissionId>,
    ) -> std::result::Result<(), AuthError> {
        if self.allows_permissions(actor, permissions) {
            Ok(())
        } else {
            Err(AuthError::forbidden_code(
                AuthErrorCode::MissingRequiredPermission,
            ))
        }
    }

    pub async fn allows_policy<I>(&self, actor: &Actor, policy: I) -> Result<bool>
    where
        I: Into<PolicyId>,
    {
        let policy = policy.into();
        let Some(policy_handler) = self.policies.get(&policy).cloned() else {
            return Err(Error::message(format!(
                "auth policy `{policy}` is not registered"
            )));
        };

        run_policy_evaluator(&policy, policy_handler, actor, &self.app).await
    }
}

async fn run_bearer_authenticator(
    guard: &GuardId,
    bearer: Arc<dyn BearerAuthenticator>,
    token: &str,
) -> std::result::Result<Option<Actor>, AuthError> {
    match catch_async_panic(|| bearer.authenticate(token)).await {
        Ok(result) => result.map_err(|error| AuthError::internal(error.to_string())),
        Err(panic) => Err(auth_guard_panic_error(guard, panic)),
    }
}

async fn run_policy_evaluator(
    policy: &PolicyId,
    policy_handler: Arc<dyn Policy>,
    actor: &Actor,
    app: &AppContext,
) -> Result<bool> {
    match catch_async_panic(|| policy_handler.evaluate(actor, app)).await {
        Ok(result) => result,
        Err(panic) => Err(auth_policy_panic_error(policy, panic)),
    }
}

fn auth_guard_panic_error(guard: &GuardId, panic: Box<dyn Any + Send>) -> AuthError {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.auth",
        guard = %guard,
        panic = %message,
        "auth guard panicked"
    );
    AuthError::internal(format!("auth guard `{guard}` panicked: {message}"))
}

fn auth_policy_panic_error(policy: &PolicyId, panic: Box<dyn Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.auth",
        policy = %policy,
        panic = %message,
        "auth policy panicked"
    );
    Error::message(format!("auth policy `{policy}` panicked: {message}"))
}

#[derive(Clone, Default)]
pub struct StaticBearerAuthenticator {
    // Keyed by SHA-256 of the token: hashing the presented token before the
    // lookup keeps plaintext comparison timing out of reach of callers.
    actors: Arc<HashMap<String, Actor>>,
}

impl StaticBearerAuthenticator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn token(mut self, token: impl Into<String>, actor: Actor) -> Self {
        let hash = crate::support::sha256_hex_str(&token.into());
        Arc::make_mut(&mut self.actors).insert(hash, actor);
        self
    }
}

#[async_trait]
impl BearerAuthenticator for StaticBearerAuthenticator {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>> {
        let hash = crate::support::sha256_hex_str(token);
        Ok(self.actors.get(&hash).cloned())
    }
}

#[derive(Debug, Clone)]
pub struct CurrentActor(pub Actor);

impl Deref for CurrentActor {
    type Target = Actor;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> FromRequestParts<S> for CurrentActor
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Actor>()
            .cloned()
            .map(Self)
            .ok_or_else(|| {
                AuthError::unauthorized_code(AuthErrorCode::AuthenticatedActorNotFound)
                    .into_response()
            })
    }
}

#[derive(Debug, Clone, Default)]
pub struct OptionalActor(pub Option<Actor>);

impl OptionalActor {
    pub fn as_ref(&self) -> Option<&Actor> {
        self.0.as_ref()
    }

    pub fn into_inner(self) -> Option<Actor> {
        self.0
    }
}

impl<S> FromRequestParts<S> for OptionalActor
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        Ok(Self(parts.extensions.get::<Actor>().cloned()))
    }
}

/// Extractor that authenticates and resolves the backing model in one step.
///
/// Combines [`CurrentActor`] extraction with [`Actor::resolve`], returning
/// the fully hydrated model. Returns 401 if unauthenticated, or if the
/// model cannot be found.
///
/// ```ignore
/// async fn profile(
///     AuthenticatedModel(user): AuthenticatedModel<User>,
/// ) -> Result<Json<User>> {
///     Ok(Json(user))
/// }
/// ```
pub struct AuthenticatedModel<M: Authenticatable>(pub M);

impl<M: Authenticatable> Deref for AuthenticatedModel<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<M, S> FromRequestParts<S> for AuthenticatedModel<M>
where
    M: Authenticatable,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let CurrentActor(actor) = CurrentActor::from_request_parts(parts, state).await?;

        let app = parts
            .extensions
            .get::<AppContext>()
            .cloned()
            .ok_or_else(|| {
                AuthError::internal("app context not available in request extensions")
                    .into_response()
            })?;

        let model = actor
            .resolve::<M>(&app)
            .await
            .map_err(|e| AuthError::internal(e.to_string()).into_response())?
            .ok_or_else(|| {
                AuthError::unauthorized_code(AuthErrorCode::AuthenticatedModelNotFound)
                    .into_response()
            })?;

        Ok(Self(model))
    }
}

/// Short alias for [`AuthenticatedModel`].
///
/// ```ignore
/// async fn profile(Auth(user): Auth<User>) -> Result<Json<User>> {
///     Ok(Json(user))
/// }
/// ```
pub type Auth<M> = AuthenticatedModel<M>;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::body::to_bytes;
    use axum::http::{header, HeaderMap, StatusCode};
    use axum::response::IntoResponse;

    use super::{
        Actor, AuthConfig, AuthManager, AuthenticatableRegistryBuilder, Authorizer,
        GuardRegistryBuilder, PermissionId, PolicyRegistryBuilder, StaticBearerAuthenticator,
    };
    use crate::config::ConfigRepository;
    use crate::database::{
        ColumnInfo, DbRecord, DbType, DbValue, QueryExecutionOptions, QueryExecutor, TableMeta,
    };
    use crate::foundation::{AppContext, Container};
    use crate::support::{GuardId, ModelId, PolicyId, RoleId};
    use crate::validation::RuleRegistry;

    struct AllowEverythingPolicy;

    #[async_trait]
    impl super::Policy for AllowEverythingPolicy {
        async fn evaluate(&self, _actor: &super::Actor, _app: &AppContext) -> crate::Result<bool> {
            Ok(true)
        }
    }

    struct PanickingAuthenticator;

    #[async_trait]
    impl super::BearerAuthenticator for PanickingAuthenticator {
        async fn authenticate(&self, _token: &str) -> crate::Result<Option<Actor>> {
            panic!("guard boom")
        }
    }

    struct PanickingPolicy;

    #[async_trait]
    impl super::Policy for PanickingPolicy {
        async fn evaluate(&self, _actor: &Actor, _app: &AppContext) -> crate::Result<bool> {
            panic!("policy boom")
        }
    }

    fn app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    #[test]
    fn rejects_duplicate_guard_registration() {
        let guards = GuardRegistryBuilder::shared();
        guards
            .lock()
            .unwrap()
            .register_arc(
                GuardId::new("api"),
                Arc::new(StaticBearerAuthenticator::new()),
            )
            .unwrap();

        let error = guards
            .lock()
            .unwrap()
            .register_arc(
                GuardId::new("api"),
                Arc::new(StaticBearerAuthenticator::new()),
            )
            .unwrap_err();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn rejects_duplicate_policy_registration() {
        let policies = PolicyRegistryBuilder::shared();
        policies
            .lock()
            .unwrap()
            .register_arc(PolicyId::new("admin"), Arc::new(AllowEverythingPolicy))
            .unwrap();

        let error = policies
            .lock()
            .unwrap()
            .register_arc(PolicyId::new("admin"), Arc::new(AllowEverythingPolicy))
            .unwrap_err();
        assert!(error.to_string().contains("already registered"));
    }

    #[tokio::test]
    async fn uses_default_guard_and_parses_bearer_header() {
        let actor = Actor::new("user-1", GuardId::new("placeholder"));
        let manager = AuthManager::new(
            AuthConfig::default(),
            HashMap::from([(
                GuardId::new("api"),
                super::GuardAuthenticator::Bearer(Arc::new(
                    StaticBearerAuthenticator::new().token("token-1", actor.clone()),
                )),
            )]),
        );

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer token-1".parse().unwrap());
        let resolved = manager.authenticate_headers(&headers, None).await.unwrap();

        assert_eq!(resolved.id, actor.id);
        assert_eq!(resolved.guard, GuardId::new("api"));
    }

    #[tokio::test]
    async fn bearer_authenticator_panic_becomes_internal_auth_error() {
        let manager = AuthManager::new(
            AuthConfig::default(),
            HashMap::from([(
                GuardId::new("api"),
                super::GuardAuthenticator::Bearer(Arc::new(PanickingAuthenticator)),
            )]),
        );

        let error = manager
            .authenticate_token("token-1", Some(&GuardId::new("api")))
            .await
            .unwrap_err();

        assert!(matches!(error, super::AuthError::Internal(_)));
        assert!(error
            .to_string()
            .contains("auth guard `api` panicked: guard boom"));
    }

    #[tokio::test]
    async fn internal_auth_error_response_uses_generic_public_message() {
        let response = super::AuthError::internal(
            "database URL postgres://user:secret@example.test/app leaked",
        )
        .into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["message"], "Internal server error");
        assert_eq!(payload["status"], 500);
        assert!(!payload["message"].as_str().unwrap().contains("secret"));
        assert!(!payload["message"].as_str().unwrap().contains("postgres://"));
    }

    #[tokio::test]
    async fn policy_panic_becomes_authorization_error() {
        let app = app();
        let authorizer = Authorizer::new(
            app,
            HashMap::from([(PolicyId::new("admin"), Arc::new(PanickingPolicy) as Arc<_>)]),
        );
        let actor = Actor::new("user-1", GuardId::new("api"));

        let error = authorizer
            .allows_policy(&actor, PolicyId::new("admin"))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("auth policy `admin` panicked: policy boom"));
    }

    #[tokio::test]
    async fn permission_checks_follow_actor_permissions() {
        let app = app();
        let authorizer = Authorizer::new(app, HashMap::new());
        let actor = Actor::new("user-1", GuardId::new("api"))
            .with_roles([RoleId::new("viewer")])
            .with_permissions([PermissionId::new("reports:view")]);

        let allowed = BTreeSet::from([PermissionId::new("reports:view")]);
        let denied = BTreeSet::from([PermissionId::new("admin")]);

        assert!(authorizer
            .authorize_permissions(&actor, &allowed)
            .await
            .is_ok());
        assert!(actor.has_role(RoleId::new("viewer")));
        assert!(authorizer
            .authorize_permissions(&actor, &denied)
            .await
            .is_err());
    }

    // --- Authenticatable registry tests ---

    // Minimal model stub for testing registration only (no real DB needed)
    #[derive(Clone, Debug)]
    struct FakeUser;

    impl crate::database::Model for FakeUser {
        type Lifecycle = crate::database::NoModelLifecycle;
        fn table_meta() -> &'static crate::database::TableMeta<Self> {
            unimplemented!("test stub")
        }
    }

    #[async_trait]
    impl super::Authenticatable for FakeUser {
        fn guard() -> GuardId {
            GuardId::new("api")
        }

        async fn resolve_from_actor<E>(_actor: &Actor, _executor: &E) -> crate::Result<Option<Self>>
        where
            E: crate::database::QueryExecutor,
        {
            Ok(Some(FakeUser))
        }
    }

    #[derive(Clone, Debug)]
    struct FakeAdmin;

    impl crate::database::Model for FakeAdmin {
        type Lifecycle = crate::database::NoModelLifecycle;
        fn table_meta() -> &'static crate::database::TableMeta<Self> {
            unimplemented!("test stub")
        }
    }

    #[async_trait]
    impl super::Authenticatable for FakeAdmin {
        fn guard() -> GuardId {
            GuardId::new("admin")
        }

        async fn resolve_from_actor<E>(_actor: &Actor, _executor: &E) -> crate::Result<Option<Self>>
        where
            E: crate::database::QueryExecutor,
        {
            Ok(Some(FakeAdmin))
        }
    }

    #[derive(Clone, Debug)]
    struct AutoResolvedUser {
        id: ModelId<Self>,
        email: String,
    }

    impl crate::database::Model for AutoResolvedUser {
        type Lifecycle = crate::database::NoModelLifecycle;

        fn table_meta() -> &'static TableMeta<Self> {
            static COLUMNS: [ColumnInfo; 2] = [
                ColumnInfo::new("id", DbType::Uuid),
                ColumnInfo::new("email", DbType::Text),
            ];
            static TABLE: std::sync::OnceLock<TableMeta<AutoResolvedUser>> =
                std::sync::OnceLock::new();
            TABLE.get_or_init(|| {
                TableMeta::new(
                    "auto_resolved_users",
                    &COLUMNS,
                    "id",
                    crate::database::ModelPrimaryKeyStrategy::UuidV7,
                    crate::database::ModelBehavior::new(
                        crate::database::ModelFeatureSetting::Default,
                        crate::database::ModelFeatureSetting::Default,
                    ),
                    |record| {
                        Ok(AutoResolvedUser {
                            id: record.decode("id")?,
                            email: record.decode("email")?,
                        })
                    },
                )
            })
        }
    }

    #[async_trait]
    impl super::Authenticatable for AutoResolvedUser {
        fn guard() -> GuardId {
            GuardId::new("api")
        }
    }

    struct FakeExecutor {
        expected_id: uuid::Uuid,
    }

    #[async_trait]
    impl QueryExecutor for FakeExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> crate::Result<Vec<DbRecord>> {
            match bindings.first() {
                Some(DbValue::Uuid(value)) if *value == self.expected_id => {
                    let mut record = DbRecord::new();
                    record.insert("id", DbValue::Uuid(*value));
                    record.insert("email", DbValue::Text("auto@example.com".to_string()));
                    Ok(vec![record])
                }
                _ => Ok(Vec::new()),
            }
        }

        async fn raw_execute_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> crate::Result<u64> {
            Ok(0)
        }
    }

    #[test]
    fn rejects_duplicate_authenticatable_guard() {
        let registry = AuthenticatableRegistryBuilder::shared();
        registry.lock().unwrap().register::<FakeUser>().unwrap();

        // Second model trying to claim same "api" guard should fail
        let error = registry.lock().unwrap().register::<FakeUser>().unwrap_err();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn allows_different_guards_for_different_models() {
        let registry = AuthenticatableRegistryBuilder::shared();
        registry.lock().unwrap().register::<FakeUser>().unwrap();
        registry.lock().unwrap().register::<FakeAdmin>().unwrap();

        let frozen = AuthenticatableRegistryBuilder::freeze_shared(registry);
        assert!(frozen.contains_guard(&GuardId::new("api")));
        assert!(frozen.contains_guard(&GuardId::new("admin")));
    }

    #[tokio::test]
    async fn actor_resolve_rejects_guard_mismatch() {
        let app = app();
        // Actor authenticated via "admin" guard trying to resolve as User ("api" guard)
        let actor = Actor::new("user-1", GuardId::new("admin"));
        let result = actor.resolve::<FakeUser>(&app).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not match authenticatable guard"));
    }

    #[tokio::test]
    async fn authenticatable_default_resolution_uses_uuid_primary_key() {
        let id = uuid::Uuid::now_v7();
        let actor = Actor::new(id.to_string(), GuardId::new("api"));
        let executor = FakeExecutor { expected_id: id };

        let resolved =
            <AutoResolvedUser as super::Authenticatable>::resolve_from_actor(&actor, &executor)
                .await
                .unwrap()
                .expect("auto-resolved user should be found");

        assert_eq!(resolved.id.as_uuid(), &id);
        assert_eq!(resolved.email, "auto@example.com");
    }
}
