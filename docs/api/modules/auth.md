# auth

Auth: guards, policies, tokens, sessions, password reset, email verification

[Back to index](../index.md)

## foundry::auth

```rust
pub type Auth<M> = AuthenticatedModel<M>;
enum AccessScope { Public, Guarded }
  fn requires_auth(&self) -> bool
  fn guard(&self) -> Option<&GuardId>
  fn permissions(&self) -> BTreeSet<PermissionId>
  fn with_guard<I>(self, guard: I) -> Self
  fn with_permission<I>(self, permission: I) -> Self
  fn with_permissions<I, P>(self, permissions: I) -> Self
enum AuthError { Unauthorized, Forbidden, Internal }
  fn unauthorized(message: impl Into<String>) -> Self
  fn unauthorized_code(code: AuthErrorCode) -> Self
  fn forbidden(message: impl Into<String>) -> Self
  fn forbidden_code(code: AuthErrorCode) -> Self
  fn internal(message: impl Into<String>) -> Self
  fn status_code(&self) -> StatusCode
  fn code(&self) -> Option<AuthErrorCode>
  fn message(&self) -> &str
  fn payload(&self) -> Value
enum AuthErrorCode { Show 13 variants    InvalidBearerToken, InvalidRefreshToken, MissingSessionCookie, InvalidSession, MissingAuthorizationHeader, InvalidAuthorizationHeader, InvalidAuthorizationScheme, MissingBearerToken, MissingAuthCredentials, MissingRequiredPermission, AuthenticatedActorNotFound, AuthenticatedModelNotFound, MaxConnectionsPerUserExceeded }
  const fn as_str(self) -> &'static str
  const fn translation_key(self) -> &'static str
  const fn default_message(self) -> &'static str
struct Actor
  fn new<I, G>(id: I, guard: G) -> Self
  fn with_guard<I>(self, guard: I) -> Self
  fn with_roles<I, R>(self, roles: I) -> Self
  fn with_permissions<I, P>(self, permissions: I) -> Self
  fn with_claims(self, claims: Value) -> Self
  fn has_role<I>(&self, role: I) -> bool
  fn has_permission<I>(&self, permission: I) -> bool
  async fn resolve<M>(&self, app: &AppContext) -> Result<Option<M>>
struct AuthErrorMessage
  fn new(message: impl Into<String>) -> Self
  fn from_code(code: AuthErrorCode) -> Self
  fn message(&self) -> &str
  fn code(&self) -> Option<AuthErrorCode>
struct AuthManager
  fn default_guard(&self) -> &GuardId
  async fn authenticate_headers( &self, headers: &HeaderMap, guard: Option<&GuardId>, ) -> Result<Actor, AuthError>
  async fn authenticate_token( &self, token: &str, guard: Option<&GuardId>, ) -> Result<Actor, AuthError>
  fn extract_token(&self, headers: &HeaderMap) -> Result<String, AuthError>
struct AuthenticatableRegistry
  async fn resolve_dynamic( &self, actor: &Actor, app: &AppContext, ) -> Result<Option<Box<dyn Any + Send + Sync>>>
  fn contains_guard(&self, guard: &GuardId) -> bool
struct AuthenticatedModel
struct Authorizer
  fn allows_permission( &self, actor: &Actor, permission: &PermissionId, ) -> bool
  fn allows_permissions( &self, actor: &Actor, permissions: &BTreeSet<PermissionId>, ) -> bool
  async fn authorize_permissions( &self, actor: &Actor, permissions: &BTreeSet<PermissionId>, ) -> Result<(), AuthError>
  async fn allows_policy<I>(&self, actor: &Actor, policy: I) -> Result<bool>
struct CurrentActor
struct GuardedAccess
struct OptionalActor
  fn as_ref(&self) -> Option<&Actor>
  fn into_inner(self) -> Option<Actor>
struct StaticBearerAuthenticator
  fn new() -> Self
  fn token(self, token: impl Into<String>, actor: Actor) -> Self
trait Authenticatable
  fn guard() -> GuardId
  fn resolve_from_actor<'life0, 'life1, 'async_trait, E>(
trait BearerAuthenticator
  fn authenticate<'life0, 'life1, 'async_trait>(
trait Policy
  fn evaluate<'life0, 'life1, 'life2, 'async_trait>(
```

## foundry::auth::email_verification

```rust
struct EmailVerificationManager
  async fn create_token<M: Authenticatable>( &self, email: &str, ) -> Result<String>
  async fn validate_token<M: Authenticatable>( &self, email: &str, token: &str, ) -> Result<()>
  async fn prune_expired(&self) -> Result<u64>
  async fn prune_expired_limited(&self, batch_size: u64) -> Result<u64>
```

## foundry::auth::lockout

```rust
enum LockoutError { LockedOut }
  fn retry_after_seconds(&self) -> u64
struct LoginLockedOutEvent
struct LoginThrottle
  fn new(app: &AppContext) -> Result<Self>
  fn with_store( app: &AppContext, store: Arc<dyn LockoutStore>, ) -> Result<Self>
  async fn before_attempt(&self, identifier: &str) -> Result<(), LockoutError>
  async fn record_failure(&self, identifier: &str) -> Result<()>
  async fn record_success(&self, identifier: &str) -> Result<()>
struct RuntimeLockoutStore
  fn from_app(app: &AppContext) -> Result<Self>
trait LockoutStore
  fn get<'life0, 'life1, 'async_trait>(
  fn increment_failures<'life0, 'life1, 'async_trait>(
  fn set_locked_until<'life0, 'life1, 'async_trait>(
  fn clear<'life0, 'life1, 'async_trait>(
```

## foundry::auth::mfa

```rust
struct CodeRequest
struct EnrollChallenge
struct MfaDisabledEvent
struct MfaEnrolledEvent
struct MfaFailedEvent
struct MfaManager
  fn new(app: &AppContext) -> Result<Self>
  fn totp(&self) -> TotpFactor
  fn enabled(&self) -> bool
  fn requires_mfa(&self, actor: &Actor) -> bool
  fn requires_mfa_for_roles<'a, I>(&self, guard: &GuardId, roles: I) -> bool
  async fn issue_pending_token( &self, actor: &Actor, name: &str, ) -> Result<TokenPair>
  async fn issue_full_token( &self, actor: &Actor, name: &str, ) -> Result<TokenPair>
struct MfaVerifiedEvent
struct RecoveryCodesRequest
struct RecoveryCodesResponse
struct TotpFactor
  fn new(app: AppContext, config: MfaConfig) -> Self
  async fn disable(&self, actor: &Actor, response: &str) -> Result<()>
  async fn regenerate_recovery_codes( &self, actor: &Actor, current_code: &str, ) -> Result<Vec<String>>
trait MfaFactor
  fn enroll<'life0, 'life1, 'async_trait>(
  fn confirm<'life0, 'life1, 'life2, 'async_trait>(
  fn verify<'life0, 'life1, 'life2, 'async_trait>(
  fn id(&self) -> &str
```

## foundry::auth::mfa::routes

```rust
async fn confirm( __arg0: State<AppContext>, __arg1: CurrentActor, __arg2: Json<CodeRequest>, ) -> Result<StatusCode>
async fn disable( __arg0: State<AppContext>, __arg1: CurrentActor, __arg2: Json<CodeRequest>, ) -> Result<StatusCode>
async fn enroll( __arg0: State<AppContext>, __arg1: CurrentActor, ) -> Result<Json<EnrollChallenge>>
async fn recovery( __arg0: State<AppContext>, __arg1: CurrentActor, __arg2: Json<RecoveryCodesRequest>, ) -> Result<Json<RecoveryCodesResponse>>
async fn verify( __arg0: State<AppContext>, __arg1: CurrentActor, __arg2: Json<CodeRequest>, ) -> Result<Json<TokenResponse>>
```

## foundry::auth::password_reset

```rust
struct PasswordResetManager
  async fn create_token<M: Authenticatable>( &self, email: &str, ) -> Result<String>
  async fn validate_token<M: Authenticatable>( &self, email: &str, token: &str, ) -> Result<()>
  async fn prune_expired(&self) -> Result<u64>
  async fn prune_expired_limited(&self, batch_size: u64) -> Result<u64>
```

## foundry::auth::session

```rust
struct SessionManager
  fn config(&self) -> &SessionConfig
  async fn create<M: Authenticatable>(&self, actor_id: &str) -> Result<String>
  async fn create_with_remember<M: Authenticatable>( &self, actor_id: &str, remember: bool, ) -> Result<String>
  async fn validate(&self, session_id: &str) -> Result<Option<Actor>>
  async fn destroy(&self, session_id: &str) -> Result<()>
  async fn destroy_all<M: Authenticatable>( &self, actor_id: &str, ) -> Result<()>
  fn login_response( &self, session_id: String, body: impl IntoResponse, ) -> Result<Response>
  fn login_response_with_remember( &self, session_id: String, remember: bool, body: impl IntoResponse, ) -> Result<Response>
  fn logout_response(&self, body: impl IntoResponse) -> Result<Response>
```

## foundry::auth::token

```rust
pub const MFA_PENDING_ABILITY: &str;
struct RefreshTokenRequest
struct TokenAuthenticator
  fn new(manager: Arc<TokenManager>) -> Self
struct TokenManager
  async fn issue<M: Authenticatable>( &self, actor_id: &str, ) -> Result<TokenPair>
  async fn issue_named<M: Authenticatable>( &self, actor_id: &str, name: &str, ) -> Result<TokenPair>
  async fn issue_with_abilities<M: Authenticatable>( &self, actor_id: &str, name: &str, abilities: Vec<String>, ) -> Result<TokenPair>
  async fn issue_actor(&self, actor: &Actor) -> Result<TokenPair>
  async fn issue_actor_named( &self, actor: &Actor, name: &str, ) -> Result<TokenPair>
  async fn issue_actor_with_abilities( &self, actor: &Actor, name: &str, abilities: Vec<String>, ) -> Result<TokenPair>
  async fn issue_mfa_pending( &self, actor: &Actor, name: &str, ttl_minutes: u64, ) -> Result<TokenPair>
  async fn validate(&self, access_token: &str) -> Result<Option<Actor>>
  async fn touch(&self, access_token: &str) -> Result<()>
  async fn refresh(&self, refresh_token: &str) -> Result<TokenPair>
  async fn revoke(&self, access_token: &str) -> Result<()>
  async fn revoke_all<M: Authenticatable>( &self, actor_id: &str, ) -> Result<u64>
  async fn sync_abilities<M: Authenticatable>( &self, actor_id: &str, abilities: Vec<String>, ) -> Result<u64>
  async fn prune(&self, older_than_days: u64) -> Result<u64>
  async fn prune_limited( &self, older_than_days: u64, batch_size: u64, ) -> Result<u64>
struct TokenPair
struct TokenResponse
  fn new(tokens: TokenPair) -> Self
  fn into_inner(self) -> TokenPair
struct WsTokenResponse
  fn new(token: impl Into<String>) -> Self
  fn into_inner(self) -> String
trait HasToken: Authenticatable
  fn token_actor_id(&self) -> String
  fn create_token<'life0, 'life1, 'async_trait>(
  fn create_token_named<'life0, 'life1, 'life2, 'async_trait>(
  fn create_token_with_abilities<'life0, 'life1, 'life2, 'async_trait>(
  fn revoke_all_tokens<'life0, 'life1, 'async_trait>(
  fn revoke_all_tokens_with<'life0, 'life1, 'async_trait, E>(
  fn sync_token_abilities<'life0, 'life1, 'async_trait>(
  fn sync_token_abilities_with<'life0, 'life1, 'async_trait, E>(
fn actor_has_mfa_pending(actor: &Actor) -> bool
```

