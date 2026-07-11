use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, Url};
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::logging::{catch_async_panic, panic_payload_message};
use crate::support::redaction::is_sensitive_key;

use super::{
    HttpClientError, HttpClientResult, HttpRequest, HttpResponse, HttpTransport, ReqwestTransport,
    RetryPolicy,
};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_CONCURRENCY: usize = 64;

/// Cloneable outbound HTTP client. Clones share the connection pool,
/// concurrency limiter, transport, and policy configuration.
#[derive(Clone)]
pub struct HttpClient {
    inner: Arc<HttpClientInner>,
}

struct HttpClientInner {
    transport: Arc<dyn HttpTransport>,
    raw_client: Option<reqwest::Client>,
    base_url: Option<Url>,
    default_headers: HeaderMap,
    connect_timeout: Option<Duration>,
    request_timeout: Option<Duration>,
    max_concurrency: usize,
    semaphore: Arc<Semaphore>,
    retry_policy: RetryPolicy,
}

#[derive(Clone, Copy)]
enum RequestTimeoutOverride {
    Inherit,
    Override(Option<Duration>),
}

impl RequestTimeoutOverride {
    fn resolve(self, default: Option<Duration>) -> Option<Duration> {
        match self {
            Self::Inherit => default,
            Self::Override(timeout) => timeout,
        }
    }
}

/// Configuration builder for [`HttpClient`].
pub struct HttpClientBuilder {
    base_url: Option<Url>,
    default_headers: HeaderMap,
    connect_timeout: Option<Duration>,
    request_timeout: Option<Duration>,
    max_concurrency: usize,
    retry_policy: RetryPolicy,
    transport: Option<Arc<dyn HttpTransport>>,
}

impl HttpClient {
    pub fn new() -> HttpClientResult<Self> {
        Self::builder().build()
    }

    pub fn builder() -> HttpClientBuilder {
        HttpClientBuilder::new()
    }

    pub fn from_transport<T>(transport: T) -> HttpClientResult<Self>
    where
        T: HttpTransport,
    {
        Self::builder().transport(transport).build()
    }

    pub fn request(&self, method: Method, target: impl Into<String>) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self.clone(), method, target.into())
    }

    pub fn get(&self, target: impl Into<String>) -> HttpRequestBuilder {
        self.request(Method::GET, target)
    }

    pub fn head(&self, target: impl Into<String>) -> HttpRequestBuilder {
        self.request(Method::HEAD, target)
    }

    pub fn post(&self, target: impl Into<String>) -> HttpRequestBuilder {
        self.request(Method::POST, target)
    }

    pub fn put(&self, target: impl Into<String>) -> HttpRequestBuilder {
        self.request(Method::PUT, target)
    }

    pub fn patch(&self, target: impl Into<String>) -> HttpRequestBuilder {
        self.request(Method::PATCH, target)
    }

    pub fn delete(&self, target: impl Into<String>) -> HttpRequestBuilder {
        self.request(Method::DELETE, target)
    }

    pub async fn send(&self, request: HttpRequest) -> HttpClientResult<HttpResponse> {
        self.send_with_options(request, None, RequestTimeoutOverride::Inherit)
            .await
    }

    pub async fn send_with_retry(
        &self,
        request: HttpRequest,
        retry_policy: RetryPolicy,
    ) -> HttpClientResult<HttpResponse> {
        self.send_with_options(request, Some(retry_policy), RequestTimeoutOverride::Inherit)
            .await
    }

    pub fn raw(&self) -> Option<&reqwest::Client> {
        self.inner.raw_client.as_ref()
    }

    pub fn base_url(&self) -> Option<&Url> {
        self.inner.base_url.as_ref()
    }

    pub fn default_headers(&self) -> &HeaderMap {
        &self.inner.default_headers
    }

    pub fn connect_timeout(&self) -> Option<Duration> {
        self.inner.connect_timeout
    }

    pub fn request_timeout(&self) -> Option<Duration> {
        self.inner.request_timeout
    }

    pub fn max_concurrency(&self) -> usize {
        self.inner.max_concurrency
    }

    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.inner.retry_policy
    }

    async fn send_with_options(
        &self,
        request: HttpRequest,
        retry_policy: Option<RetryPolicy>,
        request_timeout: RequestTimeoutOverride,
    ) -> HttpClientResult<HttpResponse> {
        let policy = retry_policy.unwrap_or_else(|| self.inner.retry_policy.clone());
        let timeout = request_timeout.resolve(self.inner.request_timeout);
        let retryable_method = policy.retries_method(request.method());
        let attempts = if retryable_method {
            policy.attempts()
        } else {
            1
        };
        let method = request.method().clone();
        let redacted_url = request.redacted_url();

        for attempt in 1..=attempts {
            tracing::debug!(
                target: "foundry.http_client",
                method = %method,
                url = %redacted_url,
                attempt,
                max_attempts = attempts,
                "outbound HTTP request started"
            );
            let started = Instant::now();
            let result = self.send_once(request.clone(), timeout).await;

            match result {
                Ok(response) => {
                    let status = response.status();
                    tracing::debug!(
                        target: "foundry.http_client",
                        method = %method,
                        url = %redacted_url,
                        status = status.as_u16(),
                        elapsed_ms = started.elapsed().as_millis(),
                        attempt,
                        "outbound HTTP request completed"
                    );

                    if attempt < attempts && policy.retries_status(status) {
                        trace_retry(&method, &redacted_url, attempt, "status");
                        sleep_before_retry(&policy, attempt).await;
                        continue;
                    }
                    return Ok(response);
                }
                Err(error) => {
                    tracing::warn!(
                        target: "foundry.http_client",
                        method = %method,
                        url = %redacted_url,
                        error_kind = ?error.kind(),
                        elapsed_ms = started.elapsed().as_millis(),
                        attempt,
                        "outbound HTTP request failed"
                    );
                    if attempt < attempts && policy.retries_error(&error) {
                        trace_retry(&method, &redacted_url, attempt, "transport");
                        sleep_before_retry(&policy, attempt).await;
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        unreachable!("HTTP attempt loop always returns on its final attempt")
    }

    async fn send_once(
        &self,
        request: HttpRequest,
        timeout: Option<Duration>,
    ) -> HttpClientResult<HttpResponse> {
        let permit = self
            .inner
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| HttpClientError::ConcurrencyClosed)?;
        let transport = self.inner.transport.clone();
        let future = async move {
            match catch_async_panic(|| transport.send(request)).await {
                Ok(result) => result,
                Err(panic) => Err(HttpClientError::transport(format!(
                    "transport panicked: {}",
                    panic_payload_message(panic)
                ))),
            }
        };

        let result = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, future)
                .await
                .map_err(|_| HttpClientError::Timeout { timeout })?,
            None => future.await,
        };
        drop(permit);
        result
    }

    fn build_request(
        &self,
        method: Method,
        target: &str,
        request_headers: HeaderMap,
        query: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> HttpClientResult<HttpRequest> {
        let mut url = resolve_url(self.inner.base_url.as_ref(), target)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(&key, &value);
            }
        }

        let mut headers = self.inner.default_headers.clone();
        merge_request_headers(&mut headers, request_headers);

        let mut request = HttpRequest::new(method, url).with_headers(headers);
        if let Some(body) = body {
            request = request.with_body(body);
        }
        Ok(request)
    }
}

impl HttpClientBuilder {
    pub fn new() -> Self {
        Self {
            base_url: None,
            default_headers: HeaderMap::new(),
            connect_timeout: Some(DEFAULT_CONNECT_TIMEOUT),
            request_timeout: Some(DEFAULT_REQUEST_TIMEOUT),
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            retry_policy: RetryPolicy::default(),
            transport: None,
        }
    }

    pub fn base_url(mut self, base_url: impl AsRef<str>) -> HttpClientResult<Self> {
        let mut base_url =
            Url::parse(base_url.as_ref()).map_err(|error| HttpClientError::InvalidUrl {
                message: error.to_string(),
            })?;
        validate_http_url(&base_url)?;
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(HttpClientError::InvalidUrl {
                message: "base URL cannot contain a query string or fragment".to_string(),
            });
        }
        if !base_url.path().ends_with('/') {
            base_url
                .path_segments_mut()
                .map_err(|_| HttpClientError::InvalidUrl {
                    message: "base URL cannot be used for relative paths".to_string(),
                })?
                .push("");
        }
        self.base_url = Some(base_url);
        Ok(self)
    }

    pub fn default_header(
        mut self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> HttpClientResult<Self> {
        let name = parse_header_name(name.as_ref())?;
        let value = parse_header_value(&name, value.as_ref())?;
        self.default_headers.insert(name, value);
        Ok(self)
    }

    pub fn default_headers(mut self, mut headers: HeaderMap) -> Self {
        mark_sensitive_headers(&mut headers);
        self.default_headers = headers;
        self
    }

    pub fn connect_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn request_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    pub fn retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub fn transport<T>(mut self, transport: T) -> Self
    where
        T: HttpTransport,
    {
        self.transport = Some(Arc::new(transport));
        self
    }

    pub fn shared_transport(mut self, transport: Arc<dyn HttpTransport>) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn build(self) -> HttpClientResult<HttpClient> {
        if self.max_concurrency == 0 {
            return Err(HttpClientError::Build {
                message: "max concurrency must be greater than zero".to_string(),
            });
        }

        let (transport, raw_client): (Arc<dyn HttpTransport>, Option<reqwest::Client>) =
            match self.transport {
                Some(transport) => (transport, None),
                None => {
                    let mut builder =
                        reqwest::Client::builder().default_headers(self.default_headers.clone());
                    if let Some(timeout) = self.connect_timeout {
                        builder = builder.connect_timeout(timeout);
                    }
                    if let Some(timeout) = self.request_timeout {
                        builder = builder.timeout(timeout);
                    }
                    let client = builder.build().map_err(|error| HttpClientError::Build {
                        message: error.without_url().to_string(),
                    })?;
                    (
                        Arc::new(ReqwestTransport::new(client.clone())),
                        Some(client),
                    )
                }
            };

        Ok(HttpClient {
            inner: Arc::new(HttpClientInner {
                transport,
                raw_client,
                base_url: self.base_url,
                default_headers: self.default_headers,
                connect_timeout: self.connect_timeout,
                request_timeout: self.request_timeout,
                max_concurrency: self.max_concurrency,
                semaphore: Arc::new(Semaphore::new(self.max_concurrency)),
                retry_policy: self.retry_policy,
            }),
        })
    }
}

impl Default for HttpClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Fluent builder for one outbound HTTP request.
pub struct HttpRequestBuilder {
    client: HttpClient,
    method: Method,
    target: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    retry_policy: Option<RetryPolicy>,
    request_timeout: RequestTimeoutOverride,
    error: Option<HttpClientError>,
}

impl HttpRequestBuilder {
    fn new(client: HttpClient, method: Method, target: String) -> Self {
        Self {
            client,
            method,
            target,
            headers: HeaderMap::new(),
            query: Vec::new(),
            body: None,
            retry_policy: None,
            request_timeout: RequestTimeoutOverride::Inherit,
            error: None,
        }
    }

    pub fn header(mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        if self.error.is_none() {
            match parse_header_name(name.as_ref()).and_then(|name| {
                parse_header_value(&name, value.as_ref()).map(|value| (name, value))
            }) {
                Ok((name, value)) => {
                    self.headers.insert(name, value);
                }
                Err(error) => self.error = Some(error),
            }
        }
        self
    }

    pub fn bearer_auth(self, token: impl AsRef<str>) -> Self {
        self.header(AUTHORIZATION.as_str(), format!("Bearer {}", token.as_ref()))
    }

    pub fn query<T>(mut self, query: &T) -> Self
    where
        T: Serialize + ?Sized,
    {
        if self.error.is_none() {
            match serialize_query(query) {
                Ok(query) => self.query.extend(query),
                Err(error) => self.error = Some(error),
            }
        }
        self
    }

    pub fn query_pair(mut self, key: impl Into<String>, value: impl ToString) -> Self {
        self.query.push((key.into(), value.to_string()));
        self
    }

    pub fn json<T>(mut self, value: &T) -> Self
    where
        T: Serialize + ?Sized,
    {
        if self.error.is_none() {
            match serde_json::to_vec(value) {
                Ok(body) => {
                    self.body = Some(body);
                    self.headers
                        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                }
                Err(error) => {
                    self.error = Some(HttpClientError::Encode {
                        target: "request JSON",
                        message: error.to_string(),
                    });
                }
            }
        }
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.request_timeout = RequestTimeoutOverride::Override(timeout);
        self
    }

    pub fn build(self) -> HttpClientResult<HttpRequest> {
        if let Some(error) = self.error {
            return Err(error);
        }
        self.client.build_request(
            self.method,
            &self.target,
            self.headers,
            self.query,
            self.body,
        )
    }

    pub async fn send(self) -> HttpClientResult<HttpResponse> {
        let client = self.client.clone();
        let retry_policy = self.retry_policy.clone();
        let request_timeout = self.request_timeout;
        let request = self.build()?;
        client
            .send_with_options(request, retry_policy, request_timeout)
            .await
    }
}

fn parse_header_name(name: &str) -> HttpClientResult<HeaderName> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|error| HttpClientError::InvalidHeader {
        name: name.to_string(),
        message: error.to_string(),
    })
}

fn parse_header_value(name: &HeaderName, value: &str) -> HttpClientResult<HeaderValue> {
    let mut value =
        HeaderValue::from_str(value).map_err(|error| HttpClientError::InvalidHeader {
            name: name.to_string(),
            message: error.to_string(),
        })?;
    if is_sensitive_key(name.as_str()) {
        value.set_sensitive(true);
    }
    Ok(value)
}

fn mark_sensitive_headers(headers: &mut HeaderMap) {
    for (name, value) in headers.iter_mut() {
        if is_sensitive_key(name.as_str()) {
            value.set_sensitive(true);
        }
    }
}

fn merge_request_headers(headers: &mut HeaderMap, request_headers: HeaderMap) {
    let mut current_name = None;
    for (name, value) in request_headers {
        if let Some(name) = name {
            headers.remove(&name);
            headers.append(name.clone(), value);
            current_name = Some(name);
        } else if let Some(name) = &current_name {
            headers.append(name, value);
        }
    }
}

fn resolve_url(base_url: Option<&Url>, target: &str) -> HttpClientResult<Url> {
    if let Ok(url) = Url::parse(target) {
        validate_http_url(&url)?;
        return Ok(url);
    }

    if target.contains("://") {
        return Err(HttpClientError::InvalidUrl {
            message: "absolute URL is malformed".to_string(),
        });
    }

    let base_url = base_url.ok_or_else(|| HttpClientError::InvalidUrl {
        message: "relative URL requires a configured base URL".to_string(),
    })?;
    let url = base_url
        .join(target)
        .map_err(|error| HttpClientError::InvalidUrl {
            message: error.to_string(),
        })?;
    validate_http_url(&url)?;
    Ok(url)
}

fn validate_http_url(url: &Url) -> HttpClientResult<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(HttpClientError::InvalidUrl {
            message: "URL scheme must be http or https".to_string(),
        });
    }
    if url.host().is_none() {
        return Err(HttpClientError::InvalidUrl {
            message: "URL must include a host".to_string(),
        });
    }
    Ok(())
}

fn serialize_query<T>(query: &T) -> HttpClientResult<Vec<(String, String)>>
where
    T: Serialize + ?Sized,
{
    let value = serde_json::to_value(query).map_err(|error| HttpClientError::Encode {
        target: "query string",
        message: error.to_string(),
    })?;
    let serde_json::Value::Object(values) = value else {
        return Err(HttpClientError::Encode {
            target: "query string",
            message: "query value must serialize as an object".to_string(),
        });
    };

    let mut pairs = Vec::new();
    for (key, value) in values {
        append_query_value(&mut pairs, key, value)?;
    }
    Ok(pairs)
}

fn append_query_value(
    pairs: &mut Vec<(String, String)>,
    key: String,
    value: serde_json::Value,
) -> HttpClientResult<()> {
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::Bool(value) => pairs.push((key, value.to_string())),
        serde_json::Value::Number(value) => pairs.push((key, value.to_string())),
        serde_json::Value::String(value) => pairs.push((key, value)),
        serde_json::Value::Array(values) => {
            for value in values {
                append_query_value(pairs, key.clone(), value)?;
            }
        }
        serde_json::Value::Object(values) => {
            for (nested_key, value) in values {
                append_query_value(pairs, format!("{key}[{nested_key}]"), value)?;
            }
        }
    }
    Ok(())
}

async fn sleep_before_retry(policy: &RetryPolicy, attempt: usize) {
    let backoff = policy.backoff_for_retry(attempt);
    if !backoff.is_zero() {
        tokio::time::sleep(backoff).await;
    }
}

fn trace_retry(method: &Method, redacted_url: &str, attempt: usize, reason: &'static str) {
    tracing::debug!(
        target: "foundry.http_client",
        method = %method,
        url = %redacted_url,
        attempt,
        reason,
        "outbound HTTP request will retry"
    );
}
