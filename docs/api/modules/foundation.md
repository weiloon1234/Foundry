# foundation

Core: App, AppBuilder, AppContext, AppTransaction, Error, ServiceProvider

[Back to index](../index.md)

## foundry::foundation

```rust
pub type Result<T> = Result<T, Error>;
enum Error { Message, Http, Validation, NotFound, Other }
  const fn internal_server_error_message() -> &'static str
  fn message(message: impl Into<String>) -> Self
  fn http(status: u16, message: impl Into<String>) -> Self
  fn http_with_code( status: u16, message: impl Into<String>, code: impl Into<String>, ) -> Self
  fn http_with_metadata( status: u16, message: impl Into<String>, error_code: Option<String>, message_key: Option<String>, ) -> Self
  fn not_found(message: impl Into<String>) -> Self
  fn other<E>(error: E) -> Self
  fn source_chain(&self) -> Vec<String>
  fn payload(&self) -> Value
struct App
  fn builder() -> AppBuilder
struct AppBuilder
  fn new() -> Self
  fn serve_spa(self, dir: impl Into<PathBuf>) -> Self
  fn load_env(self) -> Self
  fn use_external_tracing_subscriber(self) -> Self
  fn load_config_dir(self, path: impl Into<PathBuf>) -> Self
  fn register_plugin<P>(self, plugin: P) -> Self
  fn register_plugins<I, P>(self, plugins: I) -> Self
  fn register_provider<P>(self, provider: P) -> Self
  fn register_routes<F>(self, registrar: F) -> Self
  fn register_commands<F>(self, registrar: F) -> Self
  fn register_schedule<F>(self, registrar: F) -> Self
  fn register_websocket_routes<F>(self, registrar: F) -> Self
  fn register_validation_rule<I, R>(self, id: I, rule: R) -> Self
  fn register_middleware(self, config: MiddlewareConfig) -> Self
  fn register_error_reporter<R>(self) -> Self
  fn register_error_reporter_instance( self, reporter: Arc<dyn ErrorReporter>, ) -> Self
  fn middleware_group<I>( self, id: I, middlewares: Vec<MiddlewareConfig>, ) -> Self
  fn enable_observability(self) -> Self
  fn enable_public_observability(self) -> Self
  fn enable_observability_with(self, options: ObservabilityOptions) -> Self
  fn run_http(self) -> Result<()>
  async fn run_http_async(self) -> Result<()>
  fn run_cli(self) -> Result<()>
  async fn run_cli_async(self) -> Result<()>
  fn run_scheduler(self) -> Result<()>
  async fn run_scheduler_async(self) -> Result<()>
  fn run_worker(self) -> Result<()>
  async fn run_worker_async(self) -> Result<()>
  fn run_websocket(self) -> Result<()>
  async fn run_websocket_async(self) -> Result<()>
  async fn build_http_kernel(self) -> Result<HttpKernel>
  async fn build_cli_kernel(self) -> Result<CliKernel>
  async fn build_scheduler_kernel(self) -> Result<SchedulerKernel>
  async fn build_worker_kernel(self) -> Result<WorkerKernel>
  async fn build_websocket_kernel(self) -> Result<WebSocketKernel>
struct AppContext
  fn new( container: Container, config: ConfigRepository, rules: RuleRegistry, ) -> Result<Self>
  fn container(&self) -> &Container
  fn config(&self) -> &ConfigRepository
  fn timezone(&self) -> Result<Timezone>
  fn clock(&self) -> Clock
  fn rules(&self) -> &RuleRegistry
  fn resolve<T>(&self) -> Result<Arc<T>>
  fn events(&self) -> Result<Arc<EventBus>>
  fn auth(&self) -> Result<Arc<AuthManager>>
  fn authorizer(&self) -> Result<Arc<Authorizer>>
  fn jobs(&self) -> Result<Arc<JobDispatcher>>
  fn audit(&self) -> Result<Arc<AuditManager>>
  fn websocket(&self) -> Result<Arc<WebSocketPublisher>>
  fn websocket_channels(&self) -> Result<Arc<WebSocketChannelRegistry>>
  fn database(&self) -> Result<Arc<DatabaseManager>>
  fn redis(&self) -> Result<Arc<RedisManager>>
  fn storage(&self) -> Result<Arc<StorageManager>>
  fn email(&self) -> Result<Arc<EmailManager>>
  fn http_client(&self) -> Result<Arc<HttpClient>>
  fn hash(&self) -> Result<Arc<HashManager>>
  fn crypt(&self) -> Result<Arc<CryptManager>>
  async fn begin_transaction(&self) -> Result<AppTransaction>
  async fn with_model_batching<F, T>(&self, future: F) -> T
  fn diagnostics(&self) -> Result<Arc<RuntimeDiagnostics>>
  fn i18n(&self) -> Result<Arc<I18nManager>>
  fn plugins(&self) -> Result<Arc<PluginRegistry>>
  fn datatables(&self) -> Result<Arc<DatatableRegistry>>
  fn authenticatables(&self) -> Result<Arc<AuthenticatableRegistry>>
  fn tokens(&self) -> Result<Arc<TokenManager>>
  fn sessions(&self) -> Result<Arc<SessionManager>>
  fn password_resets(&self) -> Result<Arc<PasswordResetManager>>
  fn email_verification(&self) -> Result<Arc<EmailVerificationManager>>
  fn cache(&self) -> Result<Arc<CacheManager>>
  fn lock(&self) -> Result<Arc<DistributedLock>>
  async fn notify( &self, notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<()>
  async fn notify_queued( &self, notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<()>
  fn route_url<I>(&self, name: I, params: &[(&str, &str)]) -> Result<String>
  fn signed_route_url<I>( &self, name: I, params: &[(&str, &str)], expires_at: DateTime, ) -> Result<String>
  fn verify_signed_url(&self, url: &str) -> Result<()>
  async fn shutdown_plugins(&self) -> Result<()>
  async fn shutdown(&self) -> Result<()>
struct AppTransaction
  fn app(&self) -> &AppContext
  fn transaction(&self) -> &DatabaseTransaction
  async fn set_local_config(&self, name: &str, value: &str) -> Result<()>
  fn set_actor(&mut self, actor: Actor)
  fn actor(&self) -> Option<&Actor>
  fn dispatch_after_commit<J: Job>(&self, job: J)
  fn dispatch_event_after_commit<E: Event>(&self, event: E)
  fn notify_after_commit( &self, notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<()>
  fn after_commit<F, Fut>(&self, callback: F)
  async fn commit(self) -> Result<()>
  async fn rollback(self) -> Result<()>
struct Container
  fn new() -> Self
  fn singleton<T>(&self, value: T) -> Result<()>
  fn singleton_arc<T>(&self, value: Arc<T>) -> Result<()>
  fn factory<T, F>(&self, factory: F) -> Result<()>
  fn factory_arc<T, F>(&self, factory: F) -> Result<()>
  fn resolve<T>(&self) -> Result<Arc<T>>
  fn contains<T>(&self) -> bool
struct ServiceRegistrar
  fn container(&self) -> &Container
  fn config(&self) -> &ConfigRepository
  fn singleton<T>(&self, value: T) -> Result<()>
  fn singleton_arc<T>(&self, value: Arc<T>) -> Result<()>
  fn factory<T, F>(&self, factory: F) -> Result<()>
  fn resolve<T>(&self) -> Result<Arc<T>>
  fn listen_event<E, L>(&self, listener: L) -> Result<()>
  fn register_job<J>(&self) -> Result<()>
  fn register_job_middleware<M: JobMiddleware>( &self, middleware: M, ) -> Result<()>
  fn register_guard<I, G>(&self, id: I, guard: G) -> Result<()>
  fn register_actor_hydrator<I, H>(&self, guard: I, hydrator: H) -> Result<()>
  fn register_policy<I, P>(&self, id: I, policy: P) -> Result<()>
  fn register_authenticatable<M>(&self) -> Result<()>
  fn register_readiness_check<I, C>(&self, id: I, check: C) -> Result<()>
  fn register_storage_driver( &self, name: &str, factory: StorageDriverFactory, ) -> Result<()>
  fn register_email_driver( &self, name: &str, factory: EmailDriverFactory, ) -> Result<()>
  fn register_notification_channel<I, N>( &self, id: I, channel: N, ) -> Result<()>
  fn register_datatable<D>(&self) -> Result<()>
trait ServiceProvider
  fn register<'life0, 'life1, 'async_trait>(
  fn boot<'life0, 'life1, 'async_trait>(
```
