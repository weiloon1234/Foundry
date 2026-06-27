use std::any::Any;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::audit::AuditManager;
use crate::auth::{
    Actor, AuthManager, AuthenticatableRegistry, AuthenticatableRegistryBuilder, Authorizer,
    GuardRegistryBuilder, PolicyRegistryBuilder,
};
use crate::cli::CommandRegistrar;
use crate::config::{ConfigRepository, RuntimeConfig};
use crate::database::{
    set_runtime_model_defaults, AfterCommitCallback, AfterCommitSink, DatabaseManager,
    DatabaseTransaction, MigrationRegistryBuilder, ModelWriteExecutor, QueryExecutionOptions,
    QueryExecutor, SeederRegistryBuilder,
};
use crate::email::{job::SendQueuedEmailJob, EmailDriverRegistryBuilder, EmailManager};
use crate::events::{EventBus, EventRegistryBuilder};
use crate::foundation::background_tasks::ManagedBackgroundTasks;
use crate::foundation::provider::RegistryHub;
use crate::foundation::{Container, Error, Result, ServiceProvider, ServiceRegistrar};
use crate::http::middleware::MiddlewareConfig;
use crate::http::RouteRegistrar;
use crate::jobs::{JobDispatcher, JobMiddlewareRegistryBuilder, JobRegistryBuilder, JobRuntime};
use crate::kernel::{
    cli::CliKernel, http::HttpKernel, scheduler::SchedulerKernel, websocket::WebSocketKernel,
    worker::WorkerKernel,
};
use crate::logging::{
    catch_async_panic, catch_future_panic, catch_sync_panic, panic_payload_message, ErrorReporter,
    ErrorReporterRegistry, ObservabilityOptions, ProbeResult, ReadinessRegistryBuilder,
    ReadinessRegistryHandle, RuntimeBackendKind, RuntimeDiagnostics, FRAMEWORK_BOOTSTRAP_PROBE,
    REDIS_PING_PROBE, RUNTIME_BACKEND_PROBE,
};
use crate::plugin::{Plugin, PluginRegistry};
use crate::redis::RedisManager;
use crate::scheduler::ScheduleRegistrar;
use crate::storage::{StorageDriverRegistryBuilder, StorageManager};
use crate::support::runtime::RuntimeBackend;
use crate::support::sync::{lock_unpoisoned, mutex_into_inner_unpoisoned};
use crate::support::{
    Clock, CryptManager, GuardId, HashManager, RouteId, Timezone, ValidationRuleId,
};
use crate::validation::{RuleRegistry, ValidationRule};
use crate::websocket::{WebSocketPublisher, WebSocketRouteRegistrar};

#[derive(Clone)]
pub struct AppContext {
    container: Container,
    config: ConfigRepository,
    timezone: Timezone,
    rules: RuleRegistry,
}

pub struct AppTransaction {
    app: AppContext,
    transaction: DatabaseTransaction,
    after_commit: Mutex<Vec<AfterCommitCallback>>,
    actor: Option<Actor>,
}

async fn finish_kernel_run(app: AppContext, result: Result<()>) -> Result<()> {
    let kernel_failed = result.is_err();
    let mut cleanup_error = None;

    if let Err(error) = app.shutdown_background_tasks().await {
        tracing::warn!(
            error = %error,
            "background task shutdown failed"
        );
        if !kernel_failed && cleanup_error.is_none() {
            cleanup_error = Some(error);
        }
    }

    if let Err(error) = app.shutdown_plugins().await {
        tracing::warn!(
            error = %error,
            "plugin shutdown failed"
        );
        if !kernel_failed && cleanup_error.is_none() {
            cleanup_error = Some(error);
        }
    }

    match result {
        Ok(()) => cleanup_error.map_or(Ok(()), Err),
        Err(error) => Err(error),
    }
}

impl AppContext {
    pub fn new(
        container: Container,
        config: ConfigRepository,
        rules: RuleRegistry,
    ) -> Result<Self> {
        let timezone = config.app()?.timezone;
        Ok(Self {
            container,
            config,
            timezone,
            rules,
        })
    }

    pub fn container(&self) -> &Container {
        &self.container
    }

    pub fn config(&self) -> &ConfigRepository {
        &self.config
    }

    pub fn timezone(&self) -> Result<Timezone> {
        Ok(self.timezone.clone())
    }

    pub fn clock(&self) -> Clock {
        Clock::new(self.timezone.clone())
    }

    pub fn rules(&self) -> &RuleRegistry {
        &self.rules
    }

    pub fn resolve<T>(&self) -> Result<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.container.resolve::<T>()
    }

    pub fn events(&self) -> Result<Arc<EventBus>> {
        self.resolve::<EventBus>()
    }

    pub fn auth(&self) -> Result<Arc<AuthManager>> {
        self.resolve::<AuthManager>()
    }

    pub fn authorizer(&self) -> Result<Arc<Authorizer>> {
        self.resolve::<Authorizer>()
    }

    pub fn jobs(&self) -> Result<Arc<JobDispatcher>> {
        self.resolve::<JobDispatcher>()
    }

    pub(crate) fn audit(&self) -> Result<Arc<AuditManager>> {
        self.resolve::<AuditManager>()
    }

    pub fn websocket(&self) -> Result<Arc<WebSocketPublisher>> {
        self.resolve::<WebSocketPublisher>()
    }

    pub fn websocket_channels(&self) -> Result<Arc<crate::websocket::WebSocketChannelRegistry>> {
        self.resolve::<crate::websocket::WebSocketChannelRegistry>()
    }

    pub fn database(&self) -> Result<Arc<DatabaseManager>> {
        self.resolve::<DatabaseManager>()
    }

    pub fn redis(&self) -> Result<Arc<RedisManager>> {
        self.resolve::<RedisManager>()
    }

    pub fn storage(&self) -> Result<Arc<StorageManager>> {
        self.resolve::<StorageManager>()
    }

    pub fn email(&self) -> Result<Arc<EmailManager>> {
        self.resolve::<EmailManager>()
    }

    pub fn hash(&self) -> Result<Arc<HashManager>> {
        self.resolve::<HashManager>()
    }

    pub fn crypt(&self) -> Result<Arc<CryptManager>> {
        self.resolve::<CryptManager>()
    }

    pub async fn begin_transaction(&self) -> Result<AppTransaction> {
        let database = self.database()?;
        let transaction = database.begin().await?;
        Ok(AppTransaction {
            app: self.clone(),
            transaction,
            after_commit: Mutex::new(Vec::new()),
            actor: None,
        })
    }

    /// Run work inside a model extension cache scope.
    ///
    /// HTTP requests get this automatically. This helper is useful for CLI jobs
    /// or tests that want attachment/translation eager loading and lazy batch
    /// safety outside the HTTP middleware stack.
    pub async fn with_model_batching<F, T>(&self, future: F) -> T
    where
        F: Future<Output = T>,
    {
        crate::database::scope_model_extensions(future).await
    }

    pub fn diagnostics(&self) -> Result<Arc<RuntimeDiagnostics>> {
        self.resolve::<RuntimeDiagnostics>()
    }

    pub fn i18n(&self) -> Result<Arc<crate::i18n::I18nManager>> {
        self.resolve::<crate::i18n::I18nManager>()
    }

    pub fn plugins(&self) -> Result<Arc<PluginRegistry>> {
        self.resolve::<PluginRegistry>()
    }

    pub fn datatables(&self) -> Result<Arc<crate::datatable::DatatableRegistry>> {
        self.resolve::<crate::datatable::DatatableRegistry>()
    }

    pub fn authenticatables(&self) -> Result<Arc<AuthenticatableRegistry>> {
        self.resolve::<AuthenticatableRegistry>()
    }

    pub fn tokens(&self) -> Result<Arc<crate::auth::token::TokenManager>> {
        self.resolve::<crate::auth::token::TokenManager>()
    }

    pub fn sessions(&self) -> Result<Arc<crate::auth::session::SessionManager>> {
        self.resolve::<crate::auth::session::SessionManager>()
    }

    pub fn password_resets(
        &self,
    ) -> Result<Arc<crate::auth::password_reset::PasswordResetManager>> {
        self.resolve::<crate::auth::password_reset::PasswordResetManager>()
    }

    pub fn email_verification(
        &self,
    ) -> Result<Arc<crate::auth::email_verification::EmailVerificationManager>> {
        self.resolve::<crate::auth::email_verification::EmailVerificationManager>()
    }

    pub fn cache(&self) -> Result<Arc<crate::cache::CacheManager>> {
        self.resolve::<crate::cache::CacheManager>()
    }

    pub fn lock(&self) -> Result<Arc<crate::support::lock::DistributedLock>> {
        self.resolve::<crate::support::lock::DistributedLock>()
    }

    pub async fn notify(
        &self,
        notifiable: &dyn crate::notifications::Notifiable,
        notification: &dyn crate::notifications::Notification,
    ) -> Result<()> {
        crate::notifications::notify(self, notifiable, notification).await
    }

    /// Dispatch a notification asynchronously via the job queue.
    pub async fn notify_queued(
        &self,
        notifiable: &dyn crate::notifications::Notifiable,
        notification: &dyn crate::notifications::Notification,
    ) -> Result<()> {
        crate::notifications::notify_queued(self, notifiable, notification).await
    }

    /// Generate a URL from a named route.
    ///
    /// ```ignore
    /// let url = app.route_url(Route::UsersShow, &[("id", "123")])?;
    /// ```
    pub fn route_url<I>(&self, name: I, params: &[(&str, &str)]) -> Result<String>
    where
        I: Into<RouteId>,
    {
        let registry = self.resolve::<crate::http::routes::RouteRegistry>()?;
        registry.url(name, params)
    }

    /// Generate a signed URL from a named route.
    pub fn signed_route_url<I>(
        &self,
        name: I,
        params: &[(&str, &str)],
        expires_at: crate::support::DateTime,
    ) -> Result<String>
    where
        I: Into<RouteId>,
    {
        let registry = self.resolve::<crate::http::routes::RouteRegistry>()?;
        let signing_key = self.config().app()?.signing_key_bytes()?;
        registry.signed_url(name, params, &signing_key, expires_at)
    }

    /// Verify a signed URL.
    pub fn verify_signed_url(&self, url: &str) -> Result<()> {
        let signing_key = self.config().app()?.signing_key_bytes()?;
        crate::http::routes::RouteRegistry::verify_signature(url, &signing_key)
    }

    /// Shut down all registered plugins in reverse dependency order.
    /// Called automatically during graceful shutdown.
    pub async fn shutdown_plugins(&self) -> Result<()> {
        let list = match self.resolve::<PluginShutdownList>() {
            Ok(list) => list,
            Err(_) => return Ok(()), // no plugins registered
        };
        for plugin in &list.0 {
            if let Err(e) = crate::plugin::shutdown_plugin(plugin, self).await {
                tracing::warn!(
                    error = %e,
                    "plugin shutdown failed"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn managed_background_tasks(&self) -> Result<Arc<ManagedBackgroundTasks>> {
        self.resolve::<ManagedBackgroundTasks>()
    }

    pub(crate) fn spawn_managed_background_task<F, Fut>(
        &self,
        name: impl Into<String>,
        build_task: F,
    ) -> Result<Option<tokio::task::JoinHandle<()>>>
    where
        F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Result<Fut>,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let tasks = match self.managed_background_tasks() {
            Ok(tasks) => tasks,
            Err(_) => return Ok(None),
        };
        let name = name.into();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
        let task = match catch_sync_panic(|| build_task(shutdown_rx)) {
            Ok(task) => task?,
            Err(panic) => return Err(managed_background_task_panic_error(&name, "factory", panic)),
        };
        let task_name = name.clone();
        let handle = tokio::spawn(async move {
            if let Err(panic) = catch_future_panic(task).await {
                let _ = managed_background_task_panic_error(&task_name, "future", panic);
            }
            let _ = completed_tx.send(());
        });
        tasks.register(name, shutdown_tx, completed_rx, handle.abort_handle());
        Ok(Some(handle))
    }

    pub(crate) async fn shutdown_background_tasks(&self) -> Result<()> {
        let tasks = match self.managed_background_tasks() {
            Ok(tasks) => tasks,
            Err(_) => return Ok(()),
        };
        let timeout = self
            .config()
            .app()
            .map(|config| Duration::from_millis(config.background_shutdown_timeout_ms))
            .unwrap_or_else(|_| Duration::from_millis(30_000));
        tasks.shutdown(timeout).await;
        Ok(())
    }

    pub(crate) fn job_runtime(&self) -> Result<Arc<JobRuntime>> {
        self.resolve::<JobRuntime>()
    }
}

impl AppTransaction {
    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn transaction(&self) -> &DatabaseTransaction {
        &self.transaction
    }

    /// Set a PostgreSQL session configuration value for the current transaction.
    ///
    /// This delegates to [`DatabaseTransaction::set_local_config`] for apps that
    /// use the higher-level app transaction wrapper.
    pub async fn set_local_config(&self, name: &str, value: &str) -> Result<()> {
        self.transaction.set_local_config(name, value).await
    }

    /// Set the actor for audit trail support in lifecycle hooks.
    ///
    /// When an actor is set, it will be available via `ModelHookContext::actor()`
    /// in all lifecycle hooks (creating, created, updating, updated, deleting, deleted)
    /// triggered through this transaction.
    pub fn set_actor(&mut self, actor: Actor) {
        self.actor = Some(actor);
    }

    pub fn actor(&self) -> Option<&Actor> {
        self.actor.as_ref()
    }

    /// Buffer a job dispatch that will only execute after a successful `commit()`.
    ///
    /// If the transaction is rolled back (or dropped), the job is never dispatched.
    pub fn dispatch_after_commit<J: crate::jobs::Job>(&self, job: J) {
        self.defer_after_commit(Box::new(move |app| {
            Box::pin(async move { app.jobs()?.dispatch(job).await })
        }));
    }

    /// Buffer a queued notification that will only be dispatched after a successful `commit()`.
    ///
    /// Selected channel payloads are pre-rendered immediately (at call time) so
    /// the notification/notifiable do not need to outlive the transaction.
    pub fn notify_after_commit(
        &self,
        notifiable: &dyn crate::notifications::Notifiable,
        notification: &dyn crate::notifications::Notification,
    ) {
        match crate::notifications::try_build_notification_job(notifiable, notification) {
            Ok(job) => self.dispatch_after_commit(job),
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "after-commit notification payload rendering failed"
                );
            }
        }
    }

    /// Register an arbitrary async callback to run after a successful `commit()`.
    pub fn after_commit<F, Fut>(&self, callback: F)
    where
        F: FnOnce(AppContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.defer_after_commit(Box::new(move |app| Box::pin(callback(app))));
    }

    /// Commit the database transaction, then flush all pending after-commit callbacks.
    ///
    /// If the commit itself fails, no callbacks are executed.
    /// If an after-commit callback fails, the error is logged but remaining callbacks
    /// continue to execute (the database commit is not rolled back).
    pub async fn commit(self) -> Result<()> {
        self.transaction.commit().await?;

        let callbacks = mutex_into_inner_unpoisoned(self.after_commit, "after-commit callbacks");

        run_after_commit_callbacks(&self.app, callbacks).await;

        Ok(())
    }

    /// Roll back the database transaction. All pending after-commit callbacks are dropped.
    pub async fn rollback(self) -> Result<()> {
        // `self.after_commit` is dropped — callbacks never execute.
        self.transaction.rollback().await
    }
}

async fn run_after_commit_callbacks(app: &AppContext, callbacks: Vec<AfterCommitCallback>) {
    for callback in callbacks {
        if let Err(error) = run_after_commit_callback(app, callback).await {
            tracing::error!(error = %error, "after-commit dispatch failed");
        }
    }
}

async fn run_after_commit_callback(app: &AppContext, callback: AfterCommitCallback) -> Result<()> {
    let future = match catch_sync_panic(|| callback(app.clone())) {
        Ok(future) => future,
        Err(panic) => return Err(after_commit_panic_error(panic, "factory")),
    };

    match catch_future_panic(future).await {
        Ok(result) => result,
        Err(panic) => Err(after_commit_panic_error(panic, "future")),
    }
}

fn after_commit_panic_error(panic: Box<dyn Any + Send>, phase: &'static str) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        phase = phase,
        panic = %message,
        "after-commit callback panicked"
    );
    Error::message(format!("after-commit callback panicked: {message}"))
}

fn managed_background_task_panic_error(
    name: &str,
    phase: &'static str,
    panic: Box<dyn Any + Send>,
) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry::foundation::background_tasks",
        task = name,
        phase = phase,
        panic = %message,
        "managed background task panicked"
    );
    Error::message(format!(
        "managed background task `{name}` {phase} panicked: {message}"
    ))
}

async fn register_service_provider(
    provider: &dyn ServiceProvider,
    registrar: &mut ServiceRegistrar,
) -> Result<()> {
    match catch_async_panic(|| provider.register(registrar)).await {
        Ok(result) => result,
        Err(panic) => Err(service_provider_panic_error("register", panic)),
    }
}

async fn boot_service_provider(provider: &dyn ServiceProvider, app: &AppContext) -> Result<()> {
    match catch_async_panic(|| provider.boot(app)).await {
        Ok(result) => result,
        Err(panic) => Err(service_provider_panic_error("boot", panic)),
    }
}

fn registrar_action_panic_error(panic: Box<dyn Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        panic = %message,
        "plugin registrar action panicked"
    );
    Error::message(format!("plugin registrar action panicked: {message}"))
}

fn service_provider_panic_error(phase: &'static str, panic: Box<dyn Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        phase = phase,
        panic = %message,
        "service provider panicked"
    );
    Error::message(format!("service provider {phase} panicked: {message}"))
}

fn app_runner_panic_error(phase: &'static str, panic: Box<dyn Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        phase = phase,
        panic = %message,
        "app runner panicked"
    );
    Error::message(format!("app runner {phase} panicked: {message}"))
}

fn app_runner_active_runtime_error() -> Error {
    tracing::error!(
        phase = "runtime",
        "app runner cannot start inside an active Tokio runtime"
    );
    Error::message(
        "app runner runtime unavailable: sync app runners cannot be started from within an active Tokio runtime; use the async run_*_async variant instead",
    )
}

#[async_trait]
impl QueryExecutor for AppContext {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[crate::database::DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<crate::database::DbRecord>> {
        self.database()?
            .raw_query_with(sql, bindings, options)
            .await
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[crate::database::DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        self.database()?
            .raw_execute_with(sql, bindings, options)
            .await
    }
}

#[async_trait]
impl QueryExecutor for AppTransaction {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[crate::database::DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<crate::database::DbRecord>> {
        self.transaction
            .raw_query_with(sql, bindings, options)
            .await
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[crate::database::DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        self.transaction
            .raw_execute_with(sql, bindings, options)
            .await
    }
}

impl ModelWriteExecutor for AppContext {
    fn app_context(&self) -> &AppContext {
        self
    }
}

impl AfterCommitSink for AppContext {}

impl AfterCommitSink for AppTransaction {
    fn supports_after_commit(&self) -> bool {
        true
    }

    fn defer_after_commit(&self, callback: AfterCommitCallback) {
        lock_unpoisoned(&self.after_commit, "after-commit callbacks").push(callback);
    }
}

impl ModelWriteExecutor for AppTransaction {
    fn app_context(&self) -> &AppContext {
        &self.app
    }

    fn active_transaction(&self) -> Option<&DatabaseTransaction> {
        Some(&self.transaction)
    }

    fn actor(&self) -> Option<&Actor> {
        self.actor.as_ref()
    }
}

/// Plugin instances stored in reverse dependency order for graceful shutdown.
struct PluginShutdownList(Vec<Arc<dyn Plugin>>);

pub struct App;

impl App {
    pub fn builder() -> AppBuilder {
        AppBuilder::new()
    }
}

pub struct AppBuilder {
    load_env: bool,
    config_dir: Option<PathBuf>,
    plugins: Vec<Arc<dyn Plugin>>,
    providers: Vec<Arc<dyn ServiceProvider>>,
    routes: Vec<RouteRegistrar>,
    commands: Vec<CommandRegistrar>,
    schedules: Vec<ScheduleRegistrar>,
    websocket_routes: Vec<WebSocketRouteRegistrar>,
    validation_rules: Vec<(ValidationRuleId, Arc<dyn ValidationRule>)>,
    middlewares: Vec<MiddlewareConfig>,
    middleware_groups: std::collections::HashMap<String, Vec<MiddlewareConfig>>,
    error_reporters: Vec<Arc<dyn ErrorReporter>>,
    observability: Option<ObservabilityOptions>,
    spa_dir: Option<PathBuf>,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AppBuilder {
    pub fn new() -> Self {
        Self {
            load_env: false,
            config_dir: None,
            plugins: Vec::new(),
            providers: Vec::new(),
            routes: Vec::new(),
            commands: Vec::new(),
            schedules: Vec::new(),
            websocket_routes: Vec::new(),
            validation_rules: Vec::new(),
            middlewares: Vec::new(),
            middleware_groups: std::collections::HashMap::new(),
            error_reporters: Vec::new(),
            observability: None,
            spa_dir: None,
        }
    }

    /// Serve a SPA frontend from the given directory. All requests not matched
    /// by API routes will fall back to `{dir}/index.html` for client-side routing.
    ///
    /// ```ignore
    /// App::builder()
    ///     .register_routes(api::routes)
    ///     .serve_spa("dist/")
    ///     .run_http()?;
    /// ```
    pub fn serve_spa(mut self, dir: impl Into<PathBuf>) -> Self {
        self.spa_dir = Some(dir.into());
        self
    }

    pub fn load_env(mut self) -> Self {
        self.load_env = true;
        self
    }

    pub fn load_config_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_dir = Some(path.into());
        self
    }

    pub fn register_plugin<P>(mut self, plugin: P) -> Self
    where
        P: Plugin,
    {
        self.plugins.push(Arc::new(plugin));
        self
    }

    pub fn register_plugins<I, P>(mut self, plugins: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Plugin,
    {
        self.plugins.extend(
            plugins
                .into_iter()
                .map(|plugin| Arc::new(plugin) as Arc<dyn Plugin>),
        );
        self
    }

    pub fn register_provider<P>(mut self, provider: P) -> Self
    where
        P: ServiceProvider,
    {
        self.providers.push(Arc::new(provider));
        self
    }

    pub fn register_routes<F>(mut self, registrar: F) -> Self
    where
        F: Fn(&mut crate::http::HttpRegistrar) -> Result<()> + Send + Sync + 'static,
    {
        self.routes.push(Arc::new(registrar));
        self
    }

    pub fn register_commands<F>(mut self, registrar: F) -> Self
    where
        F: Fn(&mut crate::cli::CommandRegistry) -> Result<()> + Send + Sync + 'static,
    {
        self.commands.push(Arc::new(registrar));
        self
    }

    pub fn register_schedule<F>(mut self, registrar: F) -> Self
    where
        F: Fn(&mut crate::scheduler::ScheduleRegistry) -> Result<()> + Send + Sync + 'static,
    {
        self.schedules.push(Arc::new(registrar));
        self
    }

    pub fn register_websocket_routes<F>(mut self, registrar: F) -> Self
    where
        F: Fn(&mut crate::websocket::WebSocketRegistrar) -> Result<()> + Send + Sync + 'static,
    {
        self.websocket_routes.push(Arc::new(registrar));
        self
    }

    pub fn register_validation_rule<I, R>(mut self, id: I, rule: R) -> Self
    where
        I: Into<ValidationRuleId>,
        R: ValidationRule,
    {
        self.validation_rules.push((id.into(), Arc::new(rule)));
        self
    }

    pub fn register_middleware(mut self, config: MiddlewareConfig) -> Self {
        self.middlewares.push(config);
        self
    }

    pub fn register_error_reporter<R>(mut self) -> Self
    where
        R: ErrorReporter + Default,
    {
        self.error_reporters.push(Arc::new(R::default()));
        self
    }

    pub fn register_error_reporter_instance(mut self, reporter: Arc<dyn ErrorReporter>) -> Self {
        self.error_reporters.push(reporter);
        self
    }

    /// Register a named middleware group for reuse on routes.
    ///
    /// ```ignore
    /// App::builder()
    ///     .middleware_group("api", vec![
    ///         RateLimit::new(100).per_minute().build(),
    ///         Compression::new().build(),
    ///     ])
    /// ```
    pub fn middleware_group(
        mut self,
        name: impl Into<String>,
        middlewares: Vec<MiddlewareConfig>,
    ) -> Self {
        self.middleware_groups.insert(name.into(), middlewares);
        self
    }

    pub fn enable_observability(mut self) -> Self {
        self.observability = Some(ObservabilityOptions::default());
        self
    }

    pub fn enable_observability_with(mut self, options: ObservabilityOptions) -> Self {
        self.observability = Some(options);
        self
    }

    pub fn run_http(self) -> Result<()> {
        self.block_on(|builder| async move { builder.run_http_async().await })
    }

    pub async fn run_http_async(self) -> Result<()> {
        let kernel = self.build_http_kernel().await?;
        let app = kernel.app().clone();
        let result = kernel.serve().await;
        finish_kernel_run(app, result).await
    }

    pub fn run_cli(self) -> Result<()> {
        self.block_on(|builder| async move { builder.run_cli_async().await })
    }

    pub async fn run_cli_async(self) -> Result<()> {
        let kernel = self.build_cli_kernel().await?;
        let app = kernel.app().clone();
        let result = kernel.run().await;
        finish_kernel_run(app, result).await
    }

    pub fn run_scheduler(self) -> Result<()> {
        self.block_on(|builder| async move { builder.run_scheduler_async().await })
    }

    pub async fn run_scheduler_async(self) -> Result<()> {
        let kernel = self.build_scheduler_kernel().await?;
        let app = kernel.app().clone();
        let result = kernel.run().await;
        finish_kernel_run(app, result).await
    }

    pub fn run_worker(self) -> Result<()> {
        self.block_on(|builder| async move { builder.run_worker_async().await })
    }

    pub async fn run_worker_async(self) -> Result<()> {
        let kernel = self.build_worker_kernel().await?;
        let app = kernel.app().clone();
        let result = kernel.run().await;
        finish_kernel_run(app, result).await
    }

    pub fn run_websocket(self) -> Result<()> {
        self.block_on(|builder| async move { builder.run_websocket_async().await })
    }

    pub async fn run_websocket_async(self) -> Result<()> {
        let kernel = self.build_websocket_kernel().await?;
        let app = kernel.app().clone();
        let result = kernel.serve().await;
        finish_kernel_run(app, result).await
    }

    pub async fn build_http_kernel(self) -> Result<HttpKernel> {
        let boot = self.bootstrap(BootProfile::http()).await?;
        Ok(HttpKernel::new(
            boot.app,
            boot.routes,
            boot.middlewares,
            boot.observability,
            boot.spa_dir,
        ))
    }

    pub async fn build_cli_kernel(self) -> Result<CliKernel> {
        let boot = self.bootstrap(BootProfile::cli()).await?;
        Ok(CliKernel::new(boot.app, boot.commands))
    }

    pub async fn build_scheduler_kernel(self) -> Result<SchedulerKernel> {
        let boot = self.bootstrap(BootProfile::scheduler()).await?;
        let registry = crate::scheduler::build_registry(&boot.schedules)?;
        SchedulerKernel::new(boot.app, registry)
    }

    pub async fn build_worker_kernel(self) -> Result<WorkerKernel> {
        let boot = self.bootstrap(BootProfile::worker()).await?;
        WorkerKernel::new(boot.app)
    }

    pub async fn build_websocket_kernel(self) -> Result<WebSocketKernel> {
        let boot = self.bootstrap(BootProfile::websocket()).await?;
        Ok(WebSocketKernel::new(boot.app))
    }

    async fn bootstrap(self, profile: BootProfile) -> Result<BootArtifacts> {
        let AppBuilder {
            load_env,
            config_dir,
            plugins,
            providers,
            routes,
            commands,
            schedules,
            websocket_routes,
            validation_rules,
            middlewares,
            middleware_groups,
            error_reporters,
            observability,
            spa_dir,
        } = self;

        if load_env {
            dotenvy::dotenv().ok();
        }

        let prepared_plugins = crate::plugin::prepare_plugins(&plugins)?;
        let config = load_boot_config(config_dir, prepared_plugins.config_defaults.clone())?;
        set_runtime_model_defaults(config.database()?.models.clone());
        crate::logging::init(&config)?;

        let container = Container::new();
        let rules = build_rule_registry(&prepared_plugins.validation_rules, validation_rules)?;
        let registries = RegistryHub::new();
        let mut registrar = ServiceRegistrar::new(
            container.clone(),
            config.clone(),
            rules.clone(),
            registries.clone(),
        );
        for provider in &prepared_plugins.providers {
            register_service_provider(provider.as_ref(), &mut registrar).await?;
        }
        // Apply plugin direct registrations (guards, jobs, events, etc.)
        for action in prepared_plugins.registrar_actions {
            match catch_sync_panic(|| action(&registrar)) {
                Ok(result) => result?,
                Err(panic) => return Err(registrar_action_panic_error(panic)),
            }
        }
        for provider in &providers {
            register_service_provider(provider.as_ref(), &mut registrar).await?;
        }

        // Register framework-internal jobs
        registrar.register_job::<SendQueuedEmailJob>()?;
        registrar.register_job::<crate::datatable::export_job::DatatableExportJob>()?;
        registrar.register_job::<crate::notifications::SendNotificationJob>()?;

        let app = AppContext::new(container, config, rules)?;
        app.container()
            .singleton_arc(Arc::new(ManagedBackgroundTasks::default()))?;
        let error_reporter_registry = Arc::new(ErrorReporterRegistry::new(error_reporters));
        crate::logging::set_global_panic_reporters(error_reporter_registry.clone());
        registrar.register_job_middleware(crate::logging::ErrorReporterJobMiddleware)?;
        let observability_config = app.config().observability()?;
        let database = Arc::new(
            DatabaseManager::from_config_with_observability(
                &app.config().database()?,
                Some(&observability_config),
            )
            .await?,
        );

        let auth_config = app.config().auth()?;
        let backend = RuntimeBackend::from_config(app.config())?;
        let backend_kind = backend.kind();
        let jobs_config = app.config().jobs()?;
        let redis = Arc::new(RedisManager::from_config(app.config())?);
        app.container().singleton_arc(Arc::new(backend.clone()))?;
        // Create distributed lock from the same backend
        let distributed_lock = Arc::new(crate::support::lock::DistributedLock::new(
            app.resolve::<RuntimeBackend>()?,
        ));
        app.container().singleton_arc(distributed_lock.clone())?;

        // Auto-register guard authenticators from config before freezing
        let token_manager = Arc::new(crate::auth::token::TokenManager::new(
            database.clone(),
            auth_config.tokens.clone(),
        ));
        let session_manager = Arc::new(crate::auth::session::SessionManager::new(
            redis.clone(),
            auth_config.sessions.clone(),
        ));
        let password_reset_expiry_minutes = auth_config.password_resets.expiry_minutes;
        let email_verification_expiry_minutes = auth_config.email_verification.expiry_minutes;
        {
            let mut guards = lock_unpoisoned(&registries.guard, "guard registry");
            for (guard_name, driver_config) in &auth_config.guards {
                if guards.contains(guard_name) {
                    continue; // consumer-registered guard takes precedence
                }
                match driver_config.driver {
                    crate::config::GuardDriver::Token => {
                        guards.register_arc(
                            GuardId::owned(guard_name.clone()),
                            Arc::new(crate::auth::token::TokenAuthenticator::new(
                                token_manager.clone(),
                            )),
                        )?;
                    }
                    crate::config::GuardDriver::Session => {
                        guards.register_session(
                            GuardId::owned(guard_name.clone()),
                            session_manager.clone(),
                        )?;
                    }
                    crate::config::GuardDriver::Custom => {}
                }
            }
        }

        let auth_manager = Arc::new(AuthManager::new(
            auth_config,
            GuardRegistryBuilder::freeze_shared(registries.guard.clone()),
        ));
        let authorizer = Arc::new(Authorizer::new(
            app.clone(),
            PolicyRegistryBuilder::freeze_shared(registries.policy.clone()),
        ));
        let authenticatable_registry = Arc::new(AuthenticatableRegistryBuilder::freeze_shared(
            registries.authenticatable.clone(),
        ));
        register_builtin_readiness_checks(&registries.readiness, backend_kind)?;
        let diagnostics = Arc::new(RuntimeDiagnostics::new_with_config(
            backend_kind,
            ReadinessRegistryBuilder::freeze_shared(registries.readiness.clone()),
            crate::logging::RuntimeDiagnosticsConfig {
                capture_enabled: observability_config.capture_enabled,
                http_sample_retention: observability_config.http_sample_retention,
                websocket_channel_retention: observability_config.websocket_channel_retention,
            },
        ));
        let ws_config = app.config().websocket()?;
        let websocket_publisher = Arc::new(WebSocketPublisher::new(
            backend.clone(),
            diagnostics.clone(),
            ws_config.history_ttl_seconds,
            ws_config.history_buffer_size,
        ));
        let event_bus = Arc::new(EventBus::new(
            app.clone(),
            EventRegistryBuilder::freeze_shared(registries.event.clone()),
        ));
        let job_runtime = Arc::new(JobRuntime::new(
            backend,
            jobs_config.clone(),
            JobRegistryBuilder::freeze_shared(registries.job.clone(), &jobs_config),
        ));
        let job_dispatcher = Arc::new(JobDispatcher::new(job_runtime.clone(), diagnostics.clone()));
        let job_middleware_registry = Arc::new(JobMiddlewareRegistryBuilder::freeze_shared(
            registrar.job_middleware_registry(),
        ));
        let migration_registry = Arc::new(MigrationRegistryBuilder::freeze_shared(
            registries.migration.clone(),
        )?);
        let seeder_registry = Arc::new(SeederRegistryBuilder::freeze_shared(
            registries.seeder.clone(),
        )?);
        let datatable_registry = Arc::new(
            crate::datatable::registry::DatatableRegistryBuilder::freeze_shared(
                registrar.datatable_registry(),
            ),
        );

        // Auto-register built-in notification channels (consumer-registered ones take precedence)
        let ncr_handle = registrar.notification_channel_registry();
        {
            let mut ncr = lock_unpoisoned(&ncr_handle, "notification channel registry");
            if !ncr.contains(&crate::notifications::NOTIFY_EMAIL) {
                ncr.register(
                    crate::notifications::NOTIFY_EMAIL,
                    Arc::new(crate::notifications::EmailNotificationChannel),
                )?;
            }
            if !ncr.contains(&crate::notifications::NOTIFY_DATABASE) {
                ncr.register(
                    crate::notifications::NOTIFY_DATABASE,
                    Arc::new(crate::notifications::DatabaseNotificationChannel),
                )?;
            }
            if !ncr.contains(&crate::notifications::NOTIFY_BROADCAST) {
                ncr.register(
                    crate::notifications::NOTIFY_BROADCAST,
                    Arc::new(crate::notifications::BroadcastNotificationChannel),
                )?;
            }
        }
        let notification_channel_registry = Arc::new(
            crate::notifications::NotificationChannelRegistryBuilder::freeze_shared(ncr_handle),
        );

        // Cache manager (needs redis before it's moved into container)
        let cache_config = app.config().cache()?;
        let cache_store: Arc<dyn crate::cache::CacheStore> = match cache_config.driver.clone() {
            crate::config::CacheDriver::Memory => Arc::new(crate::cache::MemoryCacheStore::new(
                cache_config.max_entries,
            )),
            crate::config::CacheDriver::Redis => Arc::new(crate::cache::RedisCacheStore::new(
                redis.clone(),
                cache_config.prefix.clone(),
            )),
        };
        let cache_manager = Arc::new(crate::cache::CacheManager::with_config(
            cache_store,
            cache_config,
            Some(distributed_lock.clone()),
        ));
        let audit_manager = Arc::new(AuditManager::new());

        let password_reset_manager =
            Arc::new(crate::auth::password_reset::PasswordResetManager::new(
                database.clone(),
                password_reset_expiry_minutes,
            ));

        let email_verification_manager = Arc::new(
            crate::auth::email_verification::EmailVerificationManager::new(
                database.clone(),
                email_verification_expiry_minutes,
            ),
        );

        app.container()
            .singleton_arc(prepared_plugins.registry.clone())?;
        app.container().singleton_arc(database)?;
        app.container().singleton_arc(redis)?;
        app.container().singleton_arc(auth_manager)?;
        app.container().singleton_arc(authorizer)?;
        app.container().singleton_arc(authenticatable_registry)?;
        app.container().singleton_arc(token_manager)?;
        app.container().singleton_arc(session_manager)?;
        app.container().singleton_arc(password_reset_manager)?;
        app.container().singleton_arc(email_verification_manager)?;
        app.container().singleton_arc(cache_manager)?;
        app.container().singleton_arc(audit_manager)?;
        app.container().singleton_arc(error_reporter_registry)?;

        app.container().singleton_arc(diagnostics.clone())?;
        app.container().singleton_arc(websocket_publisher)?;
        app.container().singleton_arc(event_bus)?;
        app.container().singleton_arc(job_runtime)?;
        app.container().singleton_arc(job_dispatcher)?;
        app.container().singleton_arc(job_middleware_registry)?;
        app.container().singleton_arc(migration_registry)?;
        app.container().singleton_arc(seeder_registry)?;
        app.container().singleton_arc(datatable_registry)?;
        app.container()
            .singleton_arc(notification_channel_registry)?;

        // Register middleware groups for route-level resolution
        let groups = Arc::new(crate::http::middleware::MiddlewareGroups(middleware_groups));
        app.container().singleton_arc(groups)?;

        // Register i18n if configured
        if let Ok(i18n_config) = app.config().i18n() {
            if !i18n_config.resource_path.is_empty() {
                let i18n_manager = crate::i18n::I18nManager::load(&i18n_config)?;
                app.container().singleton_arc(Arc::new(i18n_manager))?;
            }
        }

        let collect_route_metadata = profile.routes || profile.commands;
        let mut boot_routes = Vec::new();
        if collect_route_metadata {
            boot_routes.extend(prepared_plugins.routes);
            boot_routes.extend(routes);
            let route_registry = Arc::new(crate::http::collect_named_routes(&boot_routes)?);
            app.container().singleton_arc(route_registry)?;
        }

        // Freeze registries that providers populated during register() before boot()
        // so boot hooks can resolve the same runtime services as handlers and jobs.
        let custom_storage_drivers =
            StorageDriverRegistryBuilder::freeze_shared(registries.storage_driver.clone());
        let storage =
            Arc::new(StorageManager::from_config(app.config(), custom_storage_drivers).await?);
        app.container().singleton_arc(storage)?;

        let custom_email_drivers =
            EmailDriverRegistryBuilder::freeze_shared(registries.email_driver.clone());
        let email = Arc::new(EmailManager::from_config(
            app.config(),
            custom_email_drivers,
            app.clone(),
        )?);
        app.container().singleton_arc(email)?;

        let hashing_config = app.config().hashing()?;
        let hash = Arc::new(HashManager::from_config(&hashing_config)?);
        app.container().singleton_arc(hash)?;

        let crypt_config = app.config().crypt()?;
        if !crypt_config.key.is_empty() {
            let crypt = Arc::new(CryptManager::from_config(&crypt_config)?);
            app.container().singleton_arc(crypt)?;
        }

        if profile.websocket_routes {
            let mut boot_websocket_routes = prepared_plugins.websocket_routes.clone();
            boot_websocket_routes.extend(websocket_routes.clone());

            let ws_registrar = crate::websocket::build_registrar(&boot_websocket_routes)?;
            let ws_registry =
                crate::websocket::WebSocketChannelRegistry::from_registrar(ws_registrar);
            for descriptor in ws_registry.descriptors() {
                diagnostics.register_websocket_channel(&descriptor.id);
            }
            app.container()
                .singleton_arc(std::sync::Arc::new(ws_registry))?;
        }

        for provider in &prepared_plugins.providers {
            boot_service_provider(provider.as_ref(), &app).await?;
        }
        let mut booted_plugins = Vec::with_capacity(prepared_plugins.instances.len());
        for plugin in &prepared_plugins.instances {
            if let Err(error) = crate::plugin::boot_plugin(plugin, &app).await {
                // Shut down already-booted plugins in reverse order so
                // resources acquired by earlier boot() calls don't leak; the
                // shutdown list below is only registered on full success.
                for booted in booted_plugins.iter().rev() {
                    if let Err(shutdown_error) = crate::plugin::shutdown_plugin(booted, &app).await
                    {
                        tracing::warn!(
                            error = %shutdown_error,
                            "plugin shutdown failed while rolling back boot failure"
                        );
                    }
                }
                return Err(error);
            }
            booted_plugins.push(plugin.clone());
        }
        // Store plugin instances in reverse dependency order for shutdown
        let mut shutdown_order = prepared_plugins.instances.clone();
        shutdown_order.reverse();
        app.container()
            .singleton(PluginShutdownList(shutdown_order))?;

        for provider in &providers {
            boot_service_provider(provider.as_ref(), &app).await?;
        }

        diagnostics.mark_bootstrap_complete();

        let mut boot_commands = Vec::new();
        if profile.commands {
            boot_commands.extend([
                crate::config::publish::config_publish_cli_registrar(),
                crate::config::api_docs::docs_api_cli_registrar(),
                crate::config::env_publish::env_publish_cli_registrar(),
                crate::foundation::doctor::doctor_cli_registrar(),
                crate::http::maintenance_cli_registrar(),
                crate::database::scaffold_cli_registrar(),
            ]);
            if app.config().value("database").is_some() {
                boot_commands.push(crate::database::builtin_cli_registrar());
                boot_commands.push(crate::auth::builtin_cli_registrar());
                boot_commands.push(crate::attachments::builtin_cli_registrar());
            }
            if !prepared_plugins.registry.is_empty() {
                boot_commands.push(crate::plugin::builtin_cli_registrar());
            }
            let mut type_export_websocket_routes = prepared_plugins.websocket_routes.clone();
            type_export_websocket_routes.extend(websocket_routes.clone());
            boot_commands.push(crate::typescript::builtin_cli_registrar(
                boot_routes.clone(),
                type_export_websocket_routes,
            ));
            boot_commands.extend(prepared_plugins.commands);
            boot_commands.extend(commands);
        }

        let mut boot_schedules = Vec::new();
        if profile.schedules {
            boot_schedules.extend(prepared_plugins.schedules);
            boot_schedules.extend(schedules);
        }

        let mut boot_middlewares = Vec::new();
        if profile.routes {
            boot_middlewares.extend(prepared_plugins.middlewares);
            boot_middlewares.extend(middlewares);
        }

        Ok(BootArtifacts {
            app,
            routes: boot_routes,
            commands: boot_commands,
            schedules: boot_schedules,
            middlewares: boot_middlewares,
            observability,
            spa_dir,
        })
    }

    fn block_on<F, Fut>(self, runner: F) -> Result<()>
    where
        F: FnOnce(AppBuilder) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(app_runner_active_runtime_error());
        }

        let runtime_config = self.sync_runtime_config()?;
        let runtime = build_tokio_runtime(&runtime_config)?;
        let future = match catch_sync_panic(|| runner(self)) {
            Ok(future) => future,
            Err(panic) => return Err(app_runner_panic_error("factory", panic)),
        };

        match catch_sync_panic(|| runtime.block_on(catch_future_panic(future))) {
            Ok(Ok(result)) => result,
            Ok(Err(panic)) => Err(app_runner_panic_error("future", panic)),
            Err(panic) => {
                runtime.shutdown_background();
                Err(app_runner_panic_error("runtime", panic))
            }
        }
    }

    fn sync_runtime_config(&self) -> Result<RuntimeConfig> {
        if self.load_env {
            dotenvy::dotenv().ok();
        }

        load_boot_config(self.config_dir.clone(), Vec::new())?.runtime()
    }
}

struct BootArtifacts {
    app: AppContext,
    routes: Vec<RouteRegistrar>,
    commands: Vec<CommandRegistrar>,
    schedules: Vec<ScheduleRegistrar>,
    middlewares: Vec<MiddlewareConfig>,
    observability: Option<ObservabilityOptions>,
    spa_dir: Option<PathBuf>,
}

#[derive(Clone, Copy)]
struct BootProfile {
    routes: bool,
    commands: bool,
    schedules: bool,
    websocket_routes: bool,
}

impl BootProfile {
    fn http() -> Self {
        Self {
            routes: true,
            commands: false,
            schedules: false,
            websocket_routes: true,
        }
    }

    fn cli() -> Self {
        Self {
            routes: false,
            commands: true,
            schedules: false,
            websocket_routes: false,
        }
    }

    fn scheduler() -> Self {
        Self {
            routes: false,
            commands: false,
            schedules: true,
            websocket_routes: false,
        }
    }

    fn worker() -> Self {
        Self {
            routes: false,
            commands: false,
            schedules: false,
            websocket_routes: false,
        }
    }

    fn websocket() -> Self {
        Self {
            routes: false,
            commands: false,
            schedules: false,
            websocket_routes: true,
        }
    }
}

fn load_boot_config(
    config_dir: Option<PathBuf>,
    defaults: Vec<toml::Value>,
) -> Result<ConfigRepository> {
    match config_dir {
        Some(path) => ConfigRepository::from_dir_with_defaults(path, defaults),
        None => ConfigRepository::with_env_overlay_and_defaults(defaults),
    }
}

fn build_tokio_runtime(config: &RuntimeConfig) -> Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    if config.worker_threads > 0 {
        builder.worker_threads(config.worker_threads);
    }
    if config.max_blocking_threads > 0 {
        builder.max_blocking_threads(config.max_blocking_threads);
    }
    builder.build().map_err(Error::other)
}

fn build_rule_registry(
    plugin_rules: &[(ValidationRuleId, Arc<dyn ValidationRule>)],
    app_rules: Vec<(ValidationRuleId, Arc<dyn ValidationRule>)>,
) -> Result<RuleRegistry> {
    let rules = RuleRegistry::new();
    for (name, rule) in plugin_rules {
        rules.register_arc(name.clone(), rule.clone())?;
    }
    for (name, rule) in app_rules {
        rules.register_arc(name, rule)?;
    }
    Ok(rules)
}

fn register_builtin_readiness_checks(
    registry: &ReadinessRegistryHandle,
    backend_kind: RuntimeBackendKind,
) -> Result<()> {
    let mut registry = lock_unpoisoned(registry, "readiness registry");
    registry.register_arc(
        FRAMEWORK_BOOTSTRAP_PROBE,
        Arc::new(|app: &AppContext| {
            let app = app.clone();
            async move {
                match app.diagnostics() {
                    Ok(diagnostics) if diagnostics.bootstrap_complete() => {
                        Ok(ProbeResult::healthy(FRAMEWORK_BOOTSTRAP_PROBE))
                    }
                    Ok(_) => Ok(ProbeResult::unhealthy(
                        FRAMEWORK_BOOTSTRAP_PROBE,
                        "framework bootstrap not complete",
                    )),
                    Err(error) => Ok(ProbeResult::unhealthy(
                        FRAMEWORK_BOOTSTRAP_PROBE,
                        error.to_string(),
                    )),
                }
            }
        }),
    )?;
    registry.register_arc(
        RUNTIME_BACKEND_PROBE,
        Arc::new(|app: &AppContext| {
            let app = app.clone();
            async move {
                match app.resolve::<RuntimeBackend>() {
                    Ok(backend) => Ok(ProbeResult {
                        id: RUNTIME_BACKEND_PROBE,
                        state: crate::logging::ProbeState::Healthy,
                        message: Some(format!("{:?} backend active", backend.kind())),
                    }),
                    Err(error) => Ok(ProbeResult::unhealthy(
                        RUNTIME_BACKEND_PROBE,
                        error.to_string(),
                    )),
                }
            }
        }),
    )?;

    if matches!(backend_kind, RuntimeBackendKind::Redis) {
        registry.register_arc(
            REDIS_PING_PROBE,
            Arc::new(|app: &AppContext| {
                let app = app.clone();
                async move {
                    match app.resolve::<RuntimeBackend>() {
                        Ok(backend) => match backend.ping().await {
                            Ok(()) => Ok(ProbeResult::healthy(REDIS_PING_PROBE)),
                            Err(error) => {
                                Ok(ProbeResult::unhealthy(REDIS_PING_PROBE, error.to_string()))
                            }
                        },
                        Err(error) => {
                            Ok(ProbeResult::unhealthy(REDIS_PING_PROBE, error.to_string()))
                        }
                    }
                }
            }),
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use serde::Serialize;
    use tempfile::tempdir;

    use super::{finish_kernel_run, run_after_commit_callbacks, App};
    use crate::database::AfterCommitCallback;
    use crate::events::{Event, EventContext, EventListener};
    use crate::foundation::{AppContext, Error, Result, ServiceProvider, ServiceRegistrar};
    use crate::support::{EventId, RouteId};

    struct TestProvider {
        order: Arc<Mutex<Vec<&'static str>>>,
    }

    #[derive(Clone)]
    struct AppAwareFactoryService {
        app: AppContext,
        marker: String,
    }

    impl AppAwareFactoryService {
        fn redis_namespace(&self) -> Result<String> {
            Ok(self.app.redis()?.namespace().to_string())
        }
    }

    #[derive(Clone, Serialize)]
    struct AppAwareFactoryEvent;

    impl Event for AppAwareFactoryEvent {
        const ID: EventId = EventId::new("tests.foundation.app_aware_factory");
    }

    struct AppAwareFactoryListener {
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl EventListener<AppAwareFactoryEvent> for AppAwareFactoryListener {
        async fn handle(
            &self,
            context: &EventContext,
            _event: &AppAwareFactoryEvent,
        ) -> Result<()> {
            let service = context.app().resolve::<AppAwareFactoryService>()?;
            self.log.lock().unwrap().push(format!(
                "{}:{}",
                service.marker,
                service.redis_namespace()?
            ));
            Ok(())
        }
    }

    struct AppAwareFactoryProvider {
        log: Arc<Mutex<Vec<String>>>,
    }

    struct AppAwareFactoryPanicProvider;

    #[derive(Debug)]
    struct AppAwarePanicFactoryService;

    struct BootServiceProvider {
        resolved: Arc<Mutex<Vec<&'static str>>>,
    }

    struct RouteUrlBootProvider {
        urls: Arc<Mutex<Vec<String>>>,
    }

    struct RegisterPanicProvider;

    struct BootPanicProvider;

    async fn route_url_health() -> &'static str {
        "ok"
    }

    fn after_commit_callback<F, Fut>(callback: F) -> AfterCommitCallback
    where
        F: FnOnce(AppContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Box::new(move |app| Box::pin(callback(app)))
    }

    #[async_trait]
    impl ServiceProvider for TestProvider {
        async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
            registrar.singleton::<String>("ready".to_string())?;
            self.order.lock().unwrap().push("register");
            Ok(())
        }

        async fn boot(&self, app: &AppContext) -> Result<()> {
            let value = app.resolve::<String>()?;
            assert_eq!(value.as_str(), "ready");
            self.order.lock().unwrap().push("boot");
            Ok(())
        }
    }

    #[async_trait]
    impl ServiceProvider for AppAwareFactoryProvider {
        async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
            registrar.singleton::<String>("factory-ready".to_string())?;
            registrar.factory::<AppAwareFactoryService, _>(|container, app| {
                let marker = container.resolve::<String>()?;
                Ok(AppAwareFactoryService {
                    app: app.clone(),
                    marker: marker.as_ref().clone(),
                })
            })?;
            registrar.listen_event::<AppAwareFactoryEvent, _>(AppAwareFactoryListener {
                log: self.log.clone(),
            })?;
            Ok(())
        }
    }

    #[async_trait]
    impl ServiceProvider for AppAwareFactoryPanicProvider {
        async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
            registrar.factory::<AppAwarePanicFactoryService, _>(|_, _| {
                panic!("app-aware factory exploded")
            })?;
            Ok(())
        }
    }

    #[async_trait]
    impl ServiceProvider for BootServiceProvider {
        async fn boot(&self, app: &AppContext) -> Result<()> {
            app.storage()?;
            self.resolved.lock().unwrap().push("storage");

            app.email()?;
            self.resolved.lock().unwrap().push("email");

            app.hash()?;
            self.resolved.lock().unwrap().push("hash");

            app.crypt()?.encrypt_string("boot")?;
            self.resolved.lock().unwrap().push("crypt");

            Ok(())
        }
    }

    #[async_trait]
    impl ServiceProvider for RouteUrlBootProvider {
        async fn boot(&self, app: &AppContext) -> Result<()> {
            let url = app.route_url(RouteId::new("boot.show"), &[("id", "a b")])?;
            self.urls.lock().unwrap().push(url);
            Ok(())
        }
    }

    #[async_trait]
    impl ServiceProvider for RegisterPanicProvider {
        async fn register(&self, _registrar: &mut ServiceRegistrar) -> Result<()> {
            panic!("provider register boom")
        }
    }

    #[async_trait]
    impl ServiceProvider for BootPanicProvider {
        async fn boot(&self, _app: &AppContext) -> Result<()> {
            panic!("provider boot boom")
        }
    }

    #[tokio::test]
    async fn providers_register_before_boot() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let _kernel = App::builder()
            .register_provider(TestProvider {
                order: order.clone(),
            })
            .build_cli_kernel()
            .await
            .unwrap();

        assert_eq!(order.lock().unwrap().as_slice(), ["register", "boot"]);
    }

    #[tokio::test]
    async fn providers_boot_after_core_services_are_registered() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("crypt.toml"),
            r#"
            [crypt]
            key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
            "#,
        )
        .unwrap();

        let resolved = Arc::new(Mutex::new(Vec::new()));
        let _kernel = App::builder()
            .load_config_dir(dir.path())
            .register_provider(BootServiceProvider {
                resolved: resolved.clone(),
            })
            .build_cli_kernel()
            .await
            .unwrap();

        assert_eq!(
            resolved.lock().unwrap().as_slice(),
            ["storage", "email", "hash", "crypt"]
        );
    }

    #[tokio::test]
    async fn route_urls_are_available_during_cli_provider_boot_for_registered_routes() {
        let urls = Arc::new(Mutex::new(Vec::new()));
        let _kernel = App::builder()
            .register_routes(|routes| {
                routes.route_named(
                    RouteId::new("boot.show"),
                    "/boot/:id",
                    axum::routing::get(route_url_health),
                );
                Ok(())
            })
            .register_provider(RouteUrlBootProvider { urls: urls.clone() })
            .build_cli_kernel()
            .await
            .unwrap();

        assert_eq!(urls.lock().unwrap().as_slice(), ["/boot/a%20b"]);
    }

    #[tokio::test]
    async fn provider_register_panic_becomes_bootstrap_error() {
        let error = match App::builder()
            .register_provider(RegisterPanicProvider)
            .build_cli_kernel()
            .await
        {
            Ok(_) => panic!("expected provider register panic to fail bootstrap"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("service provider register panicked"));
        assert!(error.to_string().contains("provider register boom"));
    }

    #[tokio::test]
    async fn provider_boot_panic_becomes_bootstrap_error() {
        let error = match App::builder()
            .register_provider(BootPanicProvider)
            .build_cli_kernel()
            .await
        {
            Ok(_) => panic!("expected provider boot panic to fail bootstrap"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("service provider boot panicked"));
        assert!(error.to_string().contains("provider boot boom"));
    }

    #[test]
    fn app_block_on_runner_error_remains_unchanged() {
        let error = App::builder()
            .block_on(|_| async { Err(Error::message("runner failed")) })
            .unwrap_err();

        assert_eq!(error.to_string(), "runner failed");
    }

    #[test]
    fn app_block_on_runner_factory_panic_becomes_error() {
        let error = App::builder()
            .block_on(|_| -> std::future::Ready<Result<()>> {
                panic!("runner factory boom");
            })
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "app runner factory panicked: runner factory boom"
        );
    }

    #[test]
    fn app_block_on_runner_future_panic_becomes_error() {
        let error = App::builder()
            .block_on(|_| async {
                panic!("runner future boom");
                #[allow(unreachable_code)]
                Ok(())
            })
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "app runner future panicked: runner future boom"
        );
    }

    #[tokio::test]
    async fn app_block_on_active_runtime_becomes_error() {
        let error = App::builder().block_on(|_| async { Ok(()) }).unwrap_err();

        assert_eq!(
            error.to_string(),
            "app runner runtime unavailable: sync app runners cannot be started from within an active Tokio runtime; use the async run_*_async variant instead"
        );
    }

    #[tokio::test]
    async fn managed_background_task_helper_drains_during_app_shutdown() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = kernel
            .app()
            .spawn_managed_background_task("test.helper", move |shutdown_rx| {
                Ok(async move {
                    let _ = started_tx.send(());
                    let _ = shutdown_rx.await;
                })
            })
            .unwrap()
            .unwrap();

        started_rx.await.unwrap();
        kernel.app().shutdown_background_tasks().await.unwrap();

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn managed_background_task_factory_error_remains_unchanged() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();

        let error = match kernel.app().spawn_managed_background_task(
            "test.factory-error",
            |_shutdown_rx| -> Result<std::future::Ready<()>> {
                Err(Error::message("background factory failed"))
            },
        ) {
            Ok(_) => panic!("expected background task factory error"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "background factory failed");
    }

    #[tokio::test]
    async fn managed_background_task_factory_panic_becomes_error() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();

        let error = match kernel.app().spawn_managed_background_task(
            "test.factory-panic",
            |_shutdown_rx| -> Result<std::future::Ready<()>> {
                panic!("background factory boom");
            },
        ) {
            Ok(_) => panic!("expected background task factory panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "managed background task `test.factory-panic` factory panicked: background factory boom"
        );
    }

    #[tokio::test]
    async fn managed_background_task_future_panic_isolated_and_completes_handle() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let handle = kernel
            .app()
            .spawn_managed_background_task("test.future-panic", |_shutdown_rx| {
                Ok(async move {
                    panic!("background future boom");
                })
            })
            .unwrap()
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
        kernel.app().shutdown_background_tasks().await.unwrap();
    }

    #[tokio::test]
    async fn spawned_worker_drains_during_app_shutdown() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let handle = crate::jobs::spawn_worker(kernel.app().clone()).unwrap();

        kernel.app().shutdown_background_tasks().await.unwrap();

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn background_shutdown_aborts_tasks_after_timeout() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("app.toml"),
            r#"
            [app]
            background_shutdown_timeout_ms = 1
            "#,
        )
        .unwrap();

        let kernel = App::builder()
            .load_config_dir(dir.path())
            .build_cli_kernel()
            .await
            .unwrap();
        let tasks = kernel.app().managed_background_tasks().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _completed_tx = completed_tx;
            let _ = shutdown_rx.await;
            std::future::pending::<()>().await;
        });
        tasks.register(
            "slow-task",
            shutdown_tx,
            completed_rx,
            handle.abort_handle(),
        );

        kernel.app().shutdown_background_tasks().await.unwrap();

        let error = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.is_cancelled());
    }

    #[tokio::test]
    async fn background_task_registered_after_shutdown_is_aborted() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let tasks = kernel.app().managed_background_tasks().unwrap();

        kernel.app().shutdown_background_tasks().await.unwrap();

        let (shutdown_tx, _shutdown_rx) = tokio::sync::oneshot::channel();
        let (_completed_tx, completed_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(std::future::pending::<()>());
        tasks.register(
            "late-task",
            shutdown_tx,
            completed_rx,
            handle.abort_handle(),
        );

        let error = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.is_cancelled());
    }

    #[tokio::test]
    async fn kernel_cleanup_preserves_original_kernel_error() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let handle = crate::jobs::spawn_worker(kernel.app().clone()).unwrap();

        let error = finish_kernel_run(
            kernel.app().clone(),
            Err(Error::message("kernel failed before cleanup")),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("kernel failed before cleanup"));
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn after_commit_future_panic_isolated_and_remaining_callbacks_continue() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));

        let callbacks = vec![
            after_commit_callback({
                let log = log.clone();
                move |_| async move {
                    log.lock().unwrap().push("first");
                    Ok(())
                }
            }),
            after_commit_callback(|_| async { panic!("future boom") }),
            after_commit_callback({
                let log = log.clone();
                move |_| async move {
                    log.lock().unwrap().push("after-panic");
                    Ok(())
                }
            }),
        ];

        run_after_commit_callbacks(kernel.app(), callbacks).await;

        assert_eq!(log.lock().unwrap().as_slice(), ["first", "after-panic"]);
    }

    #[tokio::test]
    async fn after_commit_factory_panic_isolated_and_remaining_callbacks_continue() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));

        let panic_callback: AfterCommitCallback =
            Box::new(|_| -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
                panic!("factory boom")
            });
        let callbacks = vec![
            panic_callback,
            after_commit_callback({
                let log = log.clone();
                move |_| async move {
                    log.lock().unwrap().push("after-factory-panic");
                    Ok(())
                }
            }),
        ];

        run_after_commit_callbacks(kernel.app(), callbacks).await;

        assert_eq!(log.lock().unwrap().as_slice(), ["after-factory-panic"]);
    }

    #[tokio::test]
    async fn app_context_resolves_redis_manager() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let redis = kernel.app().redis().unwrap();

        assert_eq!(redis.namespace(), "foundry:development");
    }

    #[tokio::test]
    async fn bootstrap_registers_websocket_channel_registry() {
        use crate::support::ChannelId;

        let builder = crate::App::builder().register_websocket_routes(|r| {
            r.channel(ChannelId::new("chat"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        });

        let kernel = builder
            .build_websocket_kernel()
            .await
            .expect("kernel builds");
        let registry = kernel
            .app()
            .container()
            .resolve::<crate::websocket::WebSocketChannelRegistry>()
            .expect("registry registered during bootstrap");

        let descriptors = registry.descriptors();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, ChannelId::new("chat"));
    }

    #[tokio::test]
    async fn app_context_exposes_websocket_channels() {
        use crate::support::ChannelId;

        let builder = crate::App::builder().register_websocket_routes(|r| {
            r.channel(ChannelId::new("alerts"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        });

        let kernel = builder.build_websocket_kernel().await.unwrap();
        let registry = kernel.app().websocket_channels().unwrap();

        let descriptors = registry.descriptors();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, ChannelId::new("alerts"));
    }

    #[tokio::test]
    async fn cli_kernel_does_not_register_websocket_channel_registry() {
        use crate::support::ChannelId;

        let kernel = crate::App::builder()
            .register_websocket_routes(|r| {
                r.channel(ChannelId::new("alerts"), |_ctx, _payload| async { Ok(()) })?;
                Ok(())
            })
            .build_cli_kernel()
            .await
            .unwrap();

        let error = kernel.app().websocket_channels().unwrap_err();
        assert!(error
            .to_string()
            .contains("WebSocketChannelRegistry` not registered"));
    }

    #[tokio::test]
    async fn provider_factories_receive_app_context_for_listener_resolution() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let kernel = App::builder()
            .register_provider(AppAwareFactoryProvider { log: log.clone() })
            .build_cli_kernel()
            .await
            .unwrap();

        kernel
            .app()
            .events()
            .unwrap()
            .dispatch(AppAwareFactoryEvent)
            .await
            .unwrap();

        assert_eq!(
            log.lock().unwrap().as_slice(),
            ["factory-ready:foundry:development"]
        );
    }

    #[tokio::test]
    async fn provider_factory_panic_isolated_during_resolution() {
        let kernel = App::builder()
            .register_provider(AppAwareFactoryPanicProvider)
            .build_cli_kernel()
            .await
            .unwrap();

        let error = kernel
            .app()
            .resolve::<AppAwarePanicFactoryService>()
            .unwrap_err();
        let message = error.to_string();

        assert!(message.contains(&format!(
            "service factory `{}`",
            std::any::type_name::<AppAwarePanicFactoryService>()
        )));
        assert!(message.contains("panicked: app-aware factory exploded"));
    }
}
