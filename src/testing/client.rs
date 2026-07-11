use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

use crate::auth::Actor;
use crate::foundation::{App, AppBuilder, AppContext, Result};

use super::{
    ClockFake, DatabaseTestTransaction, EventFake, HttpClientFake, JobFake, MailFake,
    NotificationFake,
};

/// A test application that bootstraps the full framework without starting a server.
///
/// ```ignore
/// let app = TestApp::builder()
///     .register_provider(MyProvider)
///     .register_routes(my_routes)
///     .build().await?;
///
/// let response = app.client().get("/health").send().await?;
/// assert_eq!(response.status(), 200);
/// ```
pub struct TestApp {
    app: AppContext,
    router: Router,
}

impl TestApp {
    /// Create a builder for configuring the test application.
    pub fn builder() -> TestAppBuilder {
        Self::from_builder(App::builder())
    }

    /// Create a test app from the same [`AppBuilder`] used by a runtime bootstrap.
    ///
    /// This keeps shared providers, plugins, middleware, validation rules, and routes
    /// on the production builder instead of duplicating them in test setup.
    pub fn from_builder(builder: AppBuilder) -> TestAppBuilder {
        TestAppBuilder {
            inner: builder,
            service_overrides: Vec::new(),
        }
    }

    /// Access the underlying AppContext for direct service resolution.
    pub fn app(&self) -> &AppContext {
        &self.app
    }

    /// Create a test HTTP client that sends requests to the app's router directly.
    pub fn client(&self) -> TestClient {
        let session_cookie_name = self
            .app
            .config()
            .auth()
            .map(|config| config.sessions.cookie_name)
            .unwrap_or_else(|_| "foundry_session".to_string());
        TestClient {
            router: self.router.clone(),
            default_actor: None,
            default_headers: Vec::new(),
            session_cookie_name,
        }
    }

    /// Begin a transaction intended to isolate one database test.
    pub async fn begin_database_test(&self) -> Result<DatabaseTestTransaction> {
        DatabaseTestTransaction::begin(&self.app).await
    }

    /// Install a controllable application clock after bootstrap.
    pub fn freeze_time(&self, now: crate::support::DateTime) -> Result<ClockFake> {
        let fake = ClockFake::new(now, self.app.timezone()?);
        install_clock(self.app.container(), &fake)?;
        Ok(fake)
    }

    /// Gracefully stop managed background tasks and registered plugins.
    ///
    /// Call this at the end of tests that register plugins or start framework-managed
    /// workers so their shutdown hooks complete before the test runtime exits.
    pub async fn shutdown(self) -> Result<()> {
        let Self { app, router } = self;
        drop(router);
        app.shutdown().await
    }

    /// Seed a presence member into the Redis-backed presence set for testing.
    ///
    /// Only available in tests — reaches into the framework's internal presence
    /// storage to simulate a connected user without needing a real WebSocket
    /// handshake.
    pub async fn seed_presence(
        &self,
        channel: &crate::support::ChannelId,
        actor_id: &str,
        joined_at: i64,
    ) -> crate::foundation::Result<()> {
        let backend = crate::support::runtime::RuntimeBackend::from_config(self.app.config())?;
        let aggregate_key = crate::websocket::presence_key(channel);
        let scope_key = crate::websocket::presence_scope_key(channel, None);
        let member = crate::websocket::presence_member_value(actor_id, channel, None, joined_at);
        backend.sadd(&aggregate_key, &member).await?;
        backend.sadd(&scope_key, &member).await
    }

    /// Read the remaining TTL (seconds) on the replay history list for a channel.
    /// Returns `None` if no expiration is set or the key does not exist.
    pub async fn history_ttl(
        &self,
        channel: &crate::support::ChannelId,
    ) -> crate::foundation::Result<Option<u64>> {
        let backend = crate::support::runtime::RuntimeBackend::from_config(self.app.config())?;
        let key = format!("ws:history:{}", channel.as_str());
        backend.ttl(&key).await
    }
}

/// Builder for [`TestApp`].
pub struct TestAppBuilder {
    inner: AppBuilder,
    service_overrides: Vec<TestServiceOverride>,
}

type TestServiceOverride = Box<dyn FnOnce(&crate::foundation::Container) -> Result<()> + Send>;

impl TestAppBuilder {
    pub fn use_external_tracing_subscriber(mut self) -> Self {
        self.inner = self.inner.use_external_tracing_subscriber();
        self
    }

    pub fn load_config_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.inner = self.inner.load_config_dir(path);
        self
    }

    /// Register a plugin on the same application builder used by the test app.
    pub fn register_plugin<P>(mut self, plugin: P) -> Self
    where
        P: crate::plugin::Plugin,
    {
        self.inner = self.inner.register_plugin(plugin);
        self
    }

    /// Register multiple plugins of the same concrete type.
    pub fn register_plugins<I, P>(mut self, plugins: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: crate::plugin::Plugin,
    {
        self.inner = self.inner.register_plugins(plugins);
        self
    }

    pub fn register_provider<P>(mut self, provider: P) -> Self
    where
        P: crate::foundation::ServiceProvider,
    {
        self.inner = self.inner.register_provider(provider);
        self
    }

    pub fn register_routes<F>(mut self, registrar: F) -> Self
    where
        F: Fn(&mut crate::http::HttpRegistrar) -> Result<()> + Send + Sync + 'static,
    {
        self.inner = self.inner.register_routes(registrar);
        self
    }

    pub fn register_middleware(
        mut self,
        config: crate::http::middleware::MiddlewareConfig,
    ) -> Self {
        self.inner = self.inner.register_middleware(config);
        self
    }

    pub fn register_websocket_routes<F>(mut self, registrar: F) -> Self
    where
        F: Fn(&mut crate::websocket::WebSocketRegistrar) -> Result<()> + Send + Sync + 'static,
    {
        self.inner = self.inner.register_websocket_routes(registrar);
        self
    }

    pub fn enable_observability(mut self) -> Self {
        self.inner = self.inner.enable_observability();
        self
    }

    pub fn enable_public_observability(mut self) -> Self {
        self.inner = self.inner.enable_public_observability();
        self
    }

    pub fn enable_observability_with(
        mut self,
        options: crate::logging::ObservabilityOptions,
    ) -> Self {
        self.inner = self.inner.enable_observability_with(options);
        self
    }

    /// Replace an already registered service after bootstrap and before the
    /// test router is built. Production container registration remains strict.
    pub fn replace_service<T>(self, value: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.replace_service_arc(Arc::new(value))
    }

    /// Arc form of [`Self::replace_service`].
    pub fn replace_service_arc<T>(mut self, value: Arc<T>) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.service_overrides.push(Box::new(move |container| {
            container.replace_singleton_arc(value)
        }));
        self
    }

    /// Record typed events and suppress their listeners.
    pub fn fake_events(mut self, fake: EventFake) -> Self {
        self.service_overrides.push(Box::new(move |container| {
            let bus = container.resolve::<crate::events::EventBus>()?;
            let sink: Arc<dyn crate::events::EventDispatchSink> = Arc::new(fake);
            container.replace_singleton_arc(Arc::new(bus.with_test_sink(sink)))
        }));
        self
    }

    /// Record typed jobs and suppress queue writes.
    pub fn fake_jobs(mut self, fake: JobFake) -> Self {
        self.service_overrides.push(Box::new(move |container| {
            let dispatcher = container.resolve::<crate::jobs::JobDispatcher>()?;
            let sink: Arc<dyn crate::jobs::JobDispatchSink> = Arc::new(fake);
            container.replace_singleton_arc(Arc::new(dispatcher.with_test_sink(sink)))
        }));
        self
    }

    /// Record fully resolved outbound emails and suppress transport delivery.
    pub fn fake_mail(mut self, fake: MailFake) -> Self {
        self.service_overrides.push(Box::new(move |container| {
            let manager = container.resolve::<crate::email::EmailManager>()?;
            let driver: Arc<dyn crate::email::EmailDriver> = Arc::new(fake);
            container.replace_singleton_arc(Arc::new(manager.with_test_driver(driver)))
        }));
        self
    }

    /// Record immediate and queued notifications and suppress channel delivery.
    pub fn fake_notifications(mut self, fake: NotificationFake) -> Self {
        self.service_overrides.push(Box::new(move |container| {
            let sink: Arc<dyn crate::notifications::NotificationDispatchSink> = Arc::new(fake);
            container.singleton(crate::notifications::NotificationDispatchHook::new(sink))
        }));
        self
    }

    /// Replace the outbound HTTP client with a deterministic fake transport.
    pub fn fake_http(self, fake: HttpClientFake) -> Self {
        self.replace_service(fake.client())
    }

    /// Install a caller-owned controllable application clock.
    pub fn with_clock(mut self, fake: ClockFake) -> Self {
        self.service_overrides
            .push(Box::new(move |container| install_clock(container, &fake)));
        self
    }

    /// Build the test application. Bootstraps all services without starting a server.
    pub async fn build(self) -> Result<TestApp> {
        let Self {
            inner,
            service_overrides,
        } = self;
        let kernel = inner.build_http_kernel().await?;
        for service_override in service_overrides {
            if let Err(error) = service_override(kernel.app().container()) {
                let _ = kernel.app().shutdown().await;
                return Err(error);
            }
        }
        let router = kernel.build_router()?;
        Ok(TestApp {
            app: kernel.app().clone(),
            router,
        })
    }
}

fn install_clock(container: &crate::foundation::Container, fake: &ClockFake) -> Result<()> {
    let clock = Arc::new(fake.clock());
    if container.contains::<crate::support::Clock>() {
        container.replace_singleton_arc(clock)
    } else {
        container.singleton_arc(clock)
    }
}

/// HTTP test client that sends requests directly to the router without TCP.
#[derive(Clone)]
pub struct TestClient {
    router: Router,
    default_actor: Option<Actor>,
    default_headers: Vec<(String, String)>,
    session_cookie_name: String,
}

impl TestClient {
    /// Apply a pre-authenticated actor to every request from this client.
    pub fn acting_as(mut self, actor: Actor) -> Self {
        self.default_actor = Some(actor);
        self
    }

    /// Apply a bearer token to every request from this client.
    pub fn with_bearer_token(mut self, token: &str) -> Self {
        self.default_headers
            .push(("authorization".to_string(), format!("Bearer {token}")));
        self
    }

    /// Apply the configured auth-session cookie to every request from this client.
    pub fn with_session(mut self, session_id: &str) -> Self {
        self.default_headers.push((
            "cookie".to_string(),
            format!("{}={session_id}", self.session_cookie_name),
        ));
        self
    }

    fn request(&self, method: Method, path: &str) -> TestRequestBuilder {
        let mut request = TestRequestBuilder::new(self.router.clone(), method, path);
        request.headers = self.default_headers.clone();
        request.actor = self.default_actor.clone();
        request.session_cookie_name = self.session_cookie_name.clone();
        request
    }

    pub fn get(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::GET, path)
    }

    pub fn post(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::POST, path)
    }

    pub fn put(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::PUT, path)
    }

    pub fn patch(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::PATCH, path)
    }

    pub fn delete(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::DELETE, path)
    }
}

/// Builder for constructing a test HTTP request.
pub struct TestRequestBuilder {
    router: Router,
    method: Method,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    actor: Option<Actor>,
    session_cookie_name: String,
}

impl TestRequestBuilder {
    fn new(router: Router, method: Method, path: &str) -> Self {
        Self {
            router,
            method,
            path: path.to_string(),
            headers: Vec::new(),
            body: None,
            actor: None,
            session_cookie_name: "foundry_session".to_string(),
        }
    }

    /// Add a header to the request.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Set the Authorization header with a bearer token.
    pub fn bearer_auth(self, token: &str) -> Self {
        self.header("authorization", &format!("Bearer {token}"))
    }

    /// Set the configured auth-session cookie.
    pub fn session_auth(self, session_id: &str) -> Self {
        let cookie = format!("{}={session_id}", self.session_cookie_name);
        self.header("cookie", &cookie)
    }

    /// Bypass credential lookup with a typed actor while retaining guard,
    /// permission, policy, MFA, and post-auth middleware checks.
    pub fn acting_as(mut self, actor: Actor) -> Self {
        self.actor = Some(actor);
        self
    }

    /// Set a raw request body.
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Set a text/plain request body.
    pub fn text(self, body: impl Into<String>) -> Self {
        self.header("content-type", "text/plain")
            .body(body.into().into_bytes())
    }

    /// Set a JSON request body.
    pub fn json(mut self, value: &impl serde::Serialize) -> Result<Self> {
        self.body = Some(serde_json::to_vec(value).map_err(crate::foundation::Error::other)?);
        self.headers
            .push(("content-type".to_string(), "application/json".to_string()));
        Ok(self)
    }

    /// Send the request and return the response.
    pub async fn send(self) -> Result<TestResponse> {
        let body = match self.body {
            Some(b) => Body::from(b),
            None => Body::empty(),
        };

        let mut builder = Request::builder().method(self.method).uri(&self.path);

        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }

        let mut request = builder
            .body(body)
            .map_err(crate::foundation::Error::other)?;
        if let Some(actor) = self.actor {
            request.extensions_mut().insert(actor.clone());
            request
                .extensions_mut()
                .insert(crate::auth::TestActorOverride(actor));
        }
        let response = self
            .router
            .oneshot(request)
            .await
            .map_err(crate::foundation::Error::other)?;

        let status = response.status();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(crate::foundation::Error::other)?;

        Ok(TestResponse {
            status,
            headers,
            body: body_bytes.to_vec(),
        })
    }
}

/// A test HTTP response with convenience methods for assertions.
pub struct TestResponse {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl TestResponse {
    /// The HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get a response header value by name.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }

    /// Parse the response body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(crate::foundation::Error::other)
    }

    /// The response body as a UTF-8 string.
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).map_err(crate::foundation::Error::other)
    }

    /// The raw response body bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    #[track_caller]
    pub fn assert_status(&self, expected: StatusCode) -> &Self {
        assert_eq!(
            self.status,
            expected,
            "unexpected response status; body: {}",
            self.body_preview()
        );
        self
    }

    #[track_caller]
    pub fn assert_successful(&self) -> &Self {
        assert!(
            self.status.is_success(),
            "expected a successful response, got {}; body: {}",
            self.status,
            self.body_preview()
        );
        self
    }

    #[track_caller]
    pub fn assert_ok(&self) -> &Self {
        self.assert_status(StatusCode::OK)
    }

    #[track_caller]
    pub fn assert_created(&self) -> &Self {
        self.assert_status(StatusCode::CREATED)
    }

    #[track_caller]
    pub fn assert_no_content(&self) -> &Self {
        self.assert_status(StatusCode::NO_CONTENT)
    }

    #[track_caller]
    pub fn assert_not_found(&self) -> &Self {
        self.assert_status(StatusCode::NOT_FOUND)
    }

    #[track_caller]
    pub fn assert_unprocessable(&self) -> &Self {
        self.assert_status(StatusCode::UNPROCESSABLE_ENTITY)
    }

    #[track_caller]
    pub fn assert_header(&self, name: &str, expected: &str) -> &Self {
        assert_eq!(
            self.header(name),
            Some(expected),
            "unexpected `{name}` response header"
        );
        self
    }

    #[track_caller]
    pub fn assert_header_missing(&self, name: &str) -> &Self {
        assert!(
            self.header(name).is_none(),
            "expected `{name}` response header to be absent"
        );
        self
    }

    #[track_caller]
    pub fn assert_json(&self, expected: &serde_json::Value) -> &Self {
        let actual = self.json_value();
        assert_eq!(&actual, expected, "unexpected JSON response body");
        self
    }

    #[track_caller]
    pub fn assert_json_path(&self, path: &str, expected: &serde_json::Value) -> &Self {
        let actual = self.json_value();
        let value = json_path(&actual, path)
            .unwrap_or_else(|| panic!("JSON response does not contain path `{path}`"));
        assert_eq!(value, expected, "unexpected JSON value at path `{path}`");
        self
    }

    #[track_caller]
    pub fn assert_json_fragment(&self, expected: &serde_json::Value) -> &Self {
        let actual = self.json_value();
        assert!(
            contains_json_fragment(&actual, expected),
            "JSON response does not contain fragment {expected}; actual: {actual}"
        );
        self
    }

    #[track_caller]
    pub fn assert_json_shape(&self, paths: &[&str]) -> &Self {
        let actual = self.json_value();
        for path in paths {
            assert!(
                json_path(&actual, path).is_some(),
                "JSON response does not contain required path `{path}`"
            );
        }
        self
    }

    #[track_caller]
    pub fn assert_validation_error(&self, field: &str) -> &Self {
        self.assert_unprocessable();
        let actual = self.json_value();
        let errors = actual
            .get("errors")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("validation response does not contain an `errors` array"));
        assert!(
            errors
                .iter()
                .any(|error| error.get("field").and_then(serde_json::Value::as_str) == Some(field)),
            "validation response does not contain an error for `{field}`: {actual}"
        );
        self
    }

    #[track_caller]
    pub fn assert_redirect(&self, location: &str) -> &Self {
        assert!(
            self.status.is_redirection(),
            "expected a redirect response, got {}",
            self.status
        );
        self.assert_header("location", location)
    }

    #[track_caller]
    pub fn assert_download(&self) -> &Self {
        let disposition = self
            .header("content-disposition")
            .unwrap_or_else(|| panic!("download response is missing `content-disposition`"));
        assert!(
            disposition
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("attachment")),
            "expected attachment content disposition, got `{disposition}`"
        );
        self
    }

    #[track_caller]
    pub fn assert_download_named(&self, filename: &str) -> &Self {
        self.assert_download();
        let disposition = self
            .header("content-disposition")
            .expect("assert_download verifies the header");
        assert!(
            disposition.contains(&format!("filename=\"{filename}\""))
                || disposition.contains(&format!("filename*=UTF-8''{filename}")),
            "download filename `{filename}` is not present in `{disposition}`"
        );
        self
    }

    #[track_caller]
    fn json_value(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "response body is not valid JSON: {error}; body: {}",
                self.body_preview()
            )
        })
    }

    fn body_preview(&self) -> String {
        const LIMIT: usize = 1_000;
        let body = String::from_utf8_lossy(&self.body);
        let mut preview = body.chars().take(LIMIT).collect::<String>();
        if body.chars().count() > LIMIT {
            preview.push_str("...");
        }
        preview
    }
}

fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(value);
    }

    path.split('.').try_fold(value, |current, segment| {
        current
            .as_object()
            .and_then(|object| object.get(segment))
            .or_else(|| {
                segment
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| current.as_array()?.get(index))
            })
    })
}

fn contains_json_fragment(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    if json_value_contains(actual, expected) {
        return true;
    }

    match actual {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| contains_json_fragment(value, expected)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| contains_json_fragment(value, expected)),
        _ => false,
    }
}

fn json_value_contains(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    match (actual, expected) {
        (serde_json::Value::Object(actual), serde_json::Value::Object(expected)) => {
            expected.iter().all(|(key, value)| {
                actual
                    .get(key)
                    .is_some_and(|actual| json_value_contains(actual, value))
            })
        }
        (serde_json::Value::Array(actual), serde_json::Value::Array(expected)) => {
            expected.iter().all(|value| {
                actual
                    .iter()
                    .any(|actual| json_value_contains(actual, value))
            })
        }
        _ => actual == expected,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Json;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use super::{TestApp, TestResponse};
    use crate::auth::{Actor, CurrentActor};
    use crate::email::EmailMessage;
    use crate::events::Event;
    use crate::jobs::{Job, JobContext};
    use crate::notifications::{Notifiable, Notification, NOTIFY_DATABASE};
    use crate::support::{EventId, GuardId, JobId};
    use crate::testing::{EventFake, JobFake, MailFake, NotificationFake};

    #[derive(Debug)]
    struct ReplaceableService(&'static str);

    struct ReplaceableProvider;

    #[derive(Clone, Serialize)]
    struct TestEvent {
        order_id: u64,
    }

    impl Event for TestEvent {
        const ID: EventId = EventId::new("testing.order_created");
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct TestJob {
        order_id: u64,
    }

    #[crate::__reexports::async_trait]
    impl Job for TestJob {
        const ID: JobId = JobId::new("testing.send_order");

        async fn handle(&self, _context: JobContext) -> crate::foundation::Result<()> {
            Ok(())
        }
    }

    struct TestNotifiable;

    impl Notifiable for TestNotifiable {
        fn notifiable_type(&self) -> &str {
            "users"
        }

        fn notification_id(&self) -> String {
            "user-1".to_string()
        }
    }

    struct TestNotification;

    impl Notification for TestNotification {
        fn notification_type(&self) -> &str {
            "testing.order_ready"
        }

        fn via(&self) -> Vec<crate::support::NotificationChannelId> {
            vec![NOTIFY_DATABASE]
        }
    }

    #[crate::__reexports::async_trait]
    impl crate::foundation::ServiceProvider for ReplaceableProvider {
        async fn register(
            &self,
            registrar: &mut crate::foundation::ServiceRegistrar,
        ) -> crate::foundation::Result<()> {
            registrar.singleton(ReplaceableService("production"))
        }
    }

    fn response(body: impl Into<Vec<u8>>) -> TestResponse {
        TestResponse {
            status: StatusCode::OK,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    #[test]
    fn json_returns_parse_errors() {
        let error = response("not json").json::<Value>().unwrap_err();

        assert!(error.to_string().contains("expected ident"));
    }

    #[test]
    fn text_returns_utf8_errors() {
        let error = response([0xff, 0xfe]).text().unwrap_err();

        assert!(error.to_string().contains("invalid utf-8"));
    }

    #[test]
    fn request_builder_accepts_raw_body_bytes() {
        let builder = super::TestRequestBuilder::new(
            axum::Router::new(),
            axum::http::Method::POST,
            "/upload",
        )
        .body([0xff, 0xfe]);

        assert_eq!(builder.body.as_deref(), Some([0xff, 0xfe].as_slice()));
    }

    #[test]
    fn request_builder_text_sets_plain_text_body() {
        let builder = super::TestRequestBuilder::new(
            axum::Router::new(),
            axum::http::Method::POST,
            "/message",
        )
        .text("hello");

        assert_eq!(builder.body.as_deref(), Some(b"hello".as_slice()));
        assert_eq!(
            builder.headers,
            vec![("content-type".to_string(), "text/plain".to_string())]
        );
    }

    #[test]
    fn fluent_json_assertions_cover_paths_fragments_and_shapes() {
        let response = TestResponse {
            status: StatusCode::OK,
            headers: vec![("x-request-id".to_string(), "request-1".to_string())],
            body: serde_json::to_vec(&serde_json::json!({
                "data": {
                    "users": [
                        {"id": 1, "name": "Ada", "roles": ["admin", "editor"]},
                        {"id": 2, "name": "Grace"}
                    ]
                }
            }))
            .unwrap(),
        };

        response
            .assert_ok()
            .assert_successful()
            .assert_header("X-Request-ID", "request-1")
            .assert_header_missing("x-missing")
            .assert_json_path("data.users.0.name", &serde_json::json!("Ada"))
            .assert_json_fragment(&serde_json::json!({"id": 1, "roles": ["admin"]}))
            .assert_json_shape(&["data.users", "data.users.1.name"]);
    }

    #[test]
    fn validation_redirect_and_download_assertions_follow_http_contracts() {
        let validation = TestResponse {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            headers: Vec::new(),
            body: serde_json::to_vec(&serde_json::json!({
                "message": "Validation failed",
                "status": 422,
                "errors": [{"field": "email", "code": "required", "message": "Required"}]
            }))
            .unwrap(),
        };
        validation
            .assert_unprocessable()
            .assert_validation_error("email");

        let redirect = TestResponse {
            status: StatusCode::SEE_OTHER,
            headers: vec![("location".to_string(), "/login".to_string())],
            body: Vec::new(),
        };
        redirect.assert_redirect("/login");

        let download = TestResponse {
            status: StatusCode::OK,
            headers: vec![(
                "content-disposition".to_string(),
                "attachment; filename=\"report.csv\"; filename*=UTF-8''report.csv".to_string(),
            )],
            body: b"id,name\n1,Ada\n".to_vec(),
        };
        download
            .assert_download()
            .assert_download_named("report.csv");
    }

    #[test]
    #[should_panic(expected = "JSON response does not contain path `data.missing`")]
    fn json_path_assertion_reports_missing_path() {
        response(r#"{"data":{}}"#).assert_json_path("data.missing", &serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_app_can_replace_an_existing_service_only() {
        let app = TestApp::builder()
            .register_provider(ReplaceableProvider)
            .replace_service(ReplaceableService("fake"))
            .build()
            .await
            .unwrap();
        assert_eq!(app.app().resolve::<ReplaceableService>().unwrap().0, "fake");
        app.shutdown().await.unwrap();

        let result = TestApp::builder()
            .replace_service(ReplaceableService("missing"))
            .build()
            .await;
        assert!(result
            .err()
            .is_some_and(|error| error.to_string().contains("is not registered")));
    }

    #[tokio::test]
    async fn installed_service_fakes_record_and_suppress_side_effects() {
        let events = EventFake::new();
        let jobs = JobFake::new();
        let mail = MailFake::new();
        let notifications = NotificationFake::new();
        let app = TestApp::builder()
            .fake_events(events.clone())
            .fake_jobs(jobs.clone())
            .fake_mail(mail.clone())
            .fake_notifications(notifications.clone())
            .build()
            .await
            .unwrap();

        app.app()
            .events()
            .unwrap()
            .dispatch(TestEvent { order_id: 42 })
            .await
            .unwrap();
        app.app()
            .jobs()
            .unwrap()
            .dispatch_after(TestJob { order_id: 42 }, Duration::from_secs(30))
            .await
            .unwrap();
        app.app()
            .email()
            .unwrap()
            .send(
                EmailMessage::new("Order ready")
                    .from("sender@example.test")
                    .to("user@example.test")
                    .text_body("Ready"),
            )
            .await
            .unwrap();
        app.app()
            .notify_queued(&TestNotifiable, &TestNotification)
            .await
            .unwrap();

        events.assert_dispatched_where::<TestEvent, _>(|event, _origin| event.order_id == 42);
        jobs.assert_dispatched_where::<TestJob, _>(|job, record| {
            job.order_id == 42 && record.scheduled_at > 0
        });
        mail.assert_sent_where(|message| message.subject == "Order ready");
        notifications.assert_sent_where(|notification| {
            notification.notification_type == "testing.order_ready"
                && notification.delivery == crate::testing::NotificationDelivery::Queued
        });

        app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn acting_as_and_frozen_clock_compose_with_test_requests() {
        async fn actor(CurrentActor(actor): CurrentActor) -> Json<Value> {
            Json(serde_json::json!({
                "id": actor.id,
                "guard": actor.guard.as_str(),
            }))
        }

        let app = TestApp::builder()
            .register_routes(|routes| {
                routes.route_with_options(
                    "/actor",
                    get(actor),
                    crate::http::HttpRouteOptions::new().guard(GuardId::new("api")),
                );
                Ok(())
            })
            .build()
            .await
            .unwrap();
        let now = crate::support::DateTime::parse("2026-07-11T12:00:00Z").unwrap();
        let clock = app.freeze_time(now).unwrap();

        app.client()
            .acting_as(Actor::new("user-1", GuardId::new("api")))
            .get("/actor")
            .send()
            .await
            .unwrap()
            .assert_ok()
            .assert_json_path("id", &serde_json::json!("user-1"));
        assert_eq!(app.app().clock().now(), now);
        clock.advance_seconds(60);
        assert_eq!(app.app().clock().now(), now.add_seconds(60));

        app.client()
            .acting_as(Actor::new("admin-1", GuardId::new("admin")))
            .get("/actor")
            .send()
            .await
            .unwrap()
            .assert_status(StatusCode::UNAUTHORIZED);

        app.shutdown().await.unwrap();
    }

    #[test]
    fn token_and_session_helpers_apply_default_credentials() {
        let client = super::TestClient {
            router: axum::Router::new(),
            default_actor: None,
            default_headers: Vec::new(),
            session_cookie_name: "app_session".to_string(),
        };

        let bearer = client.clone().with_bearer_token("secret").get("/");
        assert_eq!(
            bearer.headers,
            vec![("authorization".to_string(), "Bearer secret".to_string())]
        );

        let session = client.with_session("session-1").get("/");
        assert_eq!(
            session.headers,
            vec![("cookie".to_string(), "app_session=session-1".to_string())]
        );
    }
}
