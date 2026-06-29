use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

use crate::foundation::{App, AppBuilder, AppContext, Result};

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
        TestAppBuilder {
            inner: App::builder(),
        }
    }

    /// Access the underlying AppContext for direct service resolution.
    pub fn app(&self) -> &AppContext {
        &self.app
    }

    /// Create a test HTTP client that sends requests to the app's router directly.
    pub fn client(&self) -> TestClient {
        TestClient {
            router: self.router.clone(),
        }
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
        let key = crate::websocket::presence_key(channel);
        let member = crate::websocket::presence_member_value(actor_id, channel, joined_at);
        backend.sadd(&key, &member).await
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

/// Builder for TestApp.
pub struct TestAppBuilder {
    inner: AppBuilder,
}

impl TestAppBuilder {
    pub fn load_config_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.inner = self.inner.load_config_dir(path);
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

    /// Build the test application. Bootstraps all services without starting a server.
    pub async fn build(self) -> Result<TestApp> {
        let kernel = self.inner.build_http_kernel().await?;
        let router = kernel.build_router()?;
        Ok(TestApp {
            app: kernel.app().clone(),
            router,
        })
    }
}

/// HTTP test client that sends requests directly to the router without TCP.
#[derive(Clone)]
pub struct TestClient {
    router: Router,
}

impl TestClient {
    pub fn get(&self, path: &str) -> TestRequestBuilder {
        TestRequestBuilder::new(self.router.clone(), Method::GET, path)
    }

    pub fn post(&self, path: &str) -> TestRequestBuilder {
        TestRequestBuilder::new(self.router.clone(), Method::POST, path)
    }

    pub fn put(&self, path: &str) -> TestRequestBuilder {
        TestRequestBuilder::new(self.router.clone(), Method::PUT, path)
    }

    pub fn patch(&self, path: &str) -> TestRequestBuilder {
        TestRequestBuilder::new(self.router.clone(), Method::PATCH, path)
    }

    pub fn delete(&self, path: &str) -> TestRequestBuilder {
        TestRequestBuilder::new(self.router.clone(), Method::DELETE, path)
    }
}

/// Builder for constructing a test HTTP request.
pub struct TestRequestBuilder {
    router: Router,
    method: Method,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

impl TestRequestBuilder {
    fn new(router: Router, method: Method, path: &str) -> Self {
        Self {
            router,
            method,
            path: path.to_string(),
            headers: Vec::new(),
            body: None,
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

        let request = builder
            .body(body)
            .map_err(crate::foundation::Error::other)?;
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
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::Value;

    use super::TestResponse;

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
}
