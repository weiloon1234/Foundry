use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{
    HttpClient, HttpClientError, HttpClientErrorKind, HttpClientResult, HttpRequest, HttpResponse,
    HttpTransport, RetryPolicy,
};
use crate::foundation::{App, Result, ServiceProvider, ServiceRegistrar};
use crate::testing::HttpClientFake;

#[tokio::test]
async fn builder_joins_base_url_and_records_query_headers_bearer_and_json() {
    let fake = HttpClientFake::new();
    fake.respond_json(StatusCode::CREATED, &json!({ "id": 42 }))
        .unwrap();
    let client = fake
        .client_builder()
        .base_url("https://api.example.test/v1")
        .unwrap()
        .default_header("x-client", "foundry")
        .unwrap()
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();

    let response = client
        .post("users?existing=yes")
        .header("x-request", "request-value")
        .bearer_auth("secret-token")
        .query(&json!({
            "page": 2,
            "tags": ["rust", "foundry"],
            "filter": { "active": true },
            "empty": null
        }))
        .json(&json!({ "name": "Ada" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.json::<serde_json::Value>().unwrap()["id"], 42);
    fake.assert_sent_count(1).assert_sent(|request| {
        request.method() == Method::POST
            && request
                .url()
                .as_str()
                .starts_with("https://api.example.test/v1/users?")
            && request.header("x-client") == Some("foundry")
            && request.header("x-request") == Some("request-value")
            && request.header("authorization") == Some("Bearer secret-token")
            && request.header(CONTENT_TYPE.as_str()) == Some("application/json")
            && request.json_body::<serde_json::Value>().ok() == Some(json!({ "name": "Ada" }))
    });

    let request = &fake.requests()[0];
    let pairs = request.query_pairs();
    assert!(pairs.contains(&(String::from("existing"), String::from("yes"))));
    assert!(pairs.contains(&(String::from("page"), String::from("2"))));
    assert!(pairs.contains(&(String::from("tags"), String::from("rust"))));
    assert!(pairs.contains(&(String::from("tags"), String::from("foundry"))));
    assert!(pairs.contains(&(String::from("filter[active]"), String::from("true"))));
    assert!(!pairs.iter().any(|(key, _)| key == "empty"));
}

#[tokio::test]
async fn safe_default_retries_get_status_and_transport_errors_but_not_post() {
    let retry = RetryPolicy::idempotent()
        .max_attempts(3)
        .backoff(Duration::ZERO, Duration::ZERO);

    let status_fake = HttpClientFake::new();
    status_fake.sequence([
        Ok(HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE)),
        Ok(HttpResponse::new(StatusCode::OK)),
    ]);
    let status_client = status_fake
        .client_builder()
        .retry_policy(retry.clone())
        .build()
        .unwrap();
    let response = status_client
        .get("https://example.test/status")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    status_fake.assert_sent_count(2);

    let transport_fake = HttpClientFake::new();
    transport_fake.sequence([
        Err(HttpClientError::transport("connection reset")),
        Ok(HttpResponse::new(StatusCode::NO_CONTENT)),
    ]);
    let transport_client = transport_fake
        .client_builder()
        .retry_policy(retry.clone())
        .build()
        .unwrap();
    let response = transport_client
        .get("https://example.test/transport")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    transport_fake.assert_sent_count(2);

    let mutation_fake = HttpClientFake::new();
    mutation_fake.sequence([
        Ok(HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE)),
        Ok(HttpResponse::new(StatusCode::OK)),
    ]);
    let mutation_client = mutation_fake
        .client_builder()
        .retry_policy(retry)
        .build()
        .unwrap();
    let response = mutation_client
        .post("https://example.test/mutation")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    mutation_fake.assert_sent_count(1);
    assert_eq!(mutation_fake.pending_responses(), 1);

    let explicit_fake = HttpClientFake::new();
    explicit_fake.sequence([
        Ok(HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE)),
        Ok(HttpResponse::new(StatusCode::OK)),
    ]);
    let explicit_client = explicit_fake
        .client_builder()
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();
    let response = explicit_client
        .post("https://example.test/explicit-mutation")
        .retry_policy(
            RetryPolicy::idempotent()
                .retry_method(Method::POST)
                .max_attempts(2)
                .backoff(Duration::ZERO, Duration::ZERO),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    explicit_fake.assert_sent_count(2);
}

struct PendingTransport;

#[async_trait]
impl HttpTransport for PendingTransport {
    async fn send(&self, _request: HttpRequest) -> HttpClientResult<HttpResponse> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn connect_request_timeout_and_concurrency_are_configurable() {
    let client = HttpClient::builder()
        .transport(PendingTransport)
        .connect_timeout(Some(Duration::from_secs(4)))
        .request_timeout(Some(Duration::from_millis(10)))
        .max_concurrency(7)
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();

    assert_eq!(client.connect_timeout(), Some(Duration::from_secs(4)));
    assert_eq!(client.request_timeout(), Some(Duration::from_millis(10)));
    assert_eq!(client.max_concurrency(), 7);
    assert!(client.raw().is_none());

    let error = client
        .get("https://example.test/slow")
        .send()
        .await
        .unwrap_err();
    assert_eq!(error.kind(), HttpClientErrorKind::Timeout);
    assert_eq!(error.timeout_duration(), Some(Duration::from_millis(10)));

    let raw_client = HttpClient::new().unwrap();
    assert!(raw_client.raw().is_some());
}

#[derive(Clone)]
struct SlowTransport {
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

#[async_trait]
impl HttpTransport for SlowTransport {
    async fn send(&self, _request: HttpRequest) -> HttpClientResult<HttpResponse> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(HttpResponse::new(StatusCode::OK))
    }
}

#[tokio::test]
async fn concurrency_limit_bounds_active_transport_calls() {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let client = HttpClient::builder()
        .transport(SlowTransport {
            active,
            maximum: maximum.clone(),
        })
        .max_concurrency(2)
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();

    let calls = (0..6).map(|index| {
        let client = client.clone();
        async move {
            client
                .get(format!("https://example.test/{index}"))
                .send()
                .await
        }
    });
    for result in futures_util::future::join_all(calls).await {
        assert_eq!(result.unwrap().status(), StatusCode::OK);
    }

    assert_eq!(maximum.load(Ordering::SeqCst), 2);
}

#[test]
fn response_status_text_and_json_helpers_are_typed_and_body_safe() {
    let response =
        HttpResponse::from_json(StatusCode::OK, &json!({ "name": "Foundry", "ready": true }))
            .unwrap();
    assert!(response.is_success());
    assert_eq!(response.header("content-type"), Some("application/json"));
    assert!(response.text().unwrap().contains("Foundry"));
    assert_eq!(response.json::<serde_json::Value>().unwrap()["ready"], true);
    response.clone().error_for_status().unwrap();
    HttpResponse::new(StatusCode::FOUND)
        .error_for_status()
        .unwrap();

    let error = HttpResponse::new(StatusCode::UNPROCESSABLE_ENTITY)
        .with_body("private response body")
        .error_for_status()
        .unwrap_err();
    assert_eq!(error.kind(), HttpClientErrorKind::Status);
    assert_eq!(error.status(), Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert!(!error.to_string().contains("private response body"));
}

#[tokio::test]
async fn fake_sequences_record_requests_and_support_typed_assertions() {
    let fake = HttpClientFake::new();
    fake.fail(HttpClientError::transport("offline"))
        .respond(HttpResponse::new(StatusCode::ACCEPTED));
    let client = fake
        .client_builder()
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();

    let error = client
        .get("https://example.test/first")
        .send()
        .await
        .unwrap_err();
    assert_eq!(error.kind(), HttpClientErrorKind::Transport);
    let response = client
        .post("https://example.test/second")
        .body("payload")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    fake.assert_sent_count(2)
        .assert_sent(|request| {
            request.method() == Method::POST && request.body() == Some(b"payload".as_slice())
        })
        .assert_not_sent(|request| request.method() == Method::DELETE);
    fake.reset().assert_nothing_sent();

    let error = client
        .get("https://example.test/exhausted")
        .send()
        .await
        .unwrap_err();
    assert_eq!(error.kind(), HttpClientErrorKind::FakeExhausted);
}

struct CustomHttpClientProvider {
    client: HttpClient,
}

#[async_trait]
impl ServiceProvider for CustomHttpClientProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.singleton(self.client.clone())
    }
}

#[tokio::test]
async fn app_context_registers_default_client_and_preserves_provider_override() {
    let default_kernel = App::builder().build_cli_kernel().await.unwrap();
    assert!(default_kernel.app().http_client().unwrap().raw().is_some());

    let fake = HttpClientFake::new();
    let custom = fake
        .client_builder()
        .max_concurrency(3)
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();
    let custom_kernel = App::builder()
        .register_provider(CustomHttpClientProvider { client: custom })
        .build_cli_kernel()
        .await
        .unwrap();
    let resolved = custom_kernel.app().http_client().unwrap();
    assert!(resolved.raw().is_none());
    assert_eq!(resolved.max_concurrency(), 3);
}
