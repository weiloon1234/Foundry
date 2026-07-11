# Outbound HTTP Client

Foundry provides a first-class outbound HTTP client over the existing pooled
`reqwest` transport. It centralizes URL construction, headers, timeouts,
bounded concurrency, conservative retries, redacted tracing, typed responses,
and deterministic tests.

## Send a request

Every application receives a default `HttpClient` through `AppContext`:

```rust
use foundry::prelude::*;

let response = app
    .http_client()?
    .get("https://api.example.com/v1/users")
    .bearer_auth(&access_token)
    .query_pair("page", 2)
    .send()
    .await?
    .error_for_status()?;

let users: UsersResponse = response.json()?;
```

The default client has no base URL, a 10-second connect timeout, a 30-second
per-attempt request timeout, and a maximum of 64 active transport calls shared
across clones.

Request builders support arbitrary methods plus `get`, `head`, `post`, `put`,
`patch`, and `delete`; default/request headers; bearer auth; serialized query
objects; individual query pairs; JSON; raw bodies; per-request timeout; and a
per-request retry-policy override.

```rust
let response = app
    .http_client()?
    .post("https://api.example.com/v1/orders")
    .header("idempotency-key", order.id.to_string())
    .json(&CreateOrderRequest::from(&order))
    .timeout(Some(std::time::Duration::from_secs(5)))
    .send()
    .await?;

response.ensure_success()?;
```

`ensure_success` requires a 2xx response. `error_for_status` rejects only 4xx
and 5xx, so redirects remain available to callers that handle them explicitly.
`HttpResponse` also exposes status, headers, bytes, UTF-8 text, and typed JSON.

## Configure one upstream

Use a dedicated client when an upstream has a stable base URL or policy:

```rust
let client = HttpClient::builder()
    .base_url("https://payments.example.com/v2")?
    .default_header("accept", "application/json")?
    .connect_timeout(Some(std::time::Duration::from_secs(3)))
    .request_timeout(Some(std::time::Duration::from_secs(10)))
    .max_concurrency(16)
    .retry_policy(RetryPolicy::idempotent().max_attempts(4))
    .build()?;
```

Register an upstream-specific wrapper in a service provider so business code
does not repeat policy or confuse clients:

```rust
#[derive(Clone)]
struct PaymentsClient(HttpClient);

registrar.singleton(PaymentsClient(client))?;
```

A provider may also register `HttpClient` itself to replace the framework
default globally; Foundry adds its default only when that type is absent.

## Retry safety

`RetryPolicy::idempotent()` is the default. It makes at most three total
attempts for `GET`, `HEAD`, and `OPTIONS` on transport/timeouts or status
408, 429, 500, 502, 503, and 504. Exponential backoff starts at 100 ms and is
capped at 2 seconds.

Mutation methods are never retried by default. Opt in only when the upstream
operation is genuinely idempotent, normally with an idempotency key:

```rust
let mutation_retry = RetryPolicy::idempotent()
    .retry_method(HttpMethod::POST)
    .max_attempts(3);

client
    .post("orders")
    .header("idempotency-key", order.id.to_string())
    .json(&payload)
    .retry_policy(mutation_retry)
    .send()
    .await?;
```

Use `RetryPolicy::none()` when an operation must make exactly one attempt.

## Errors and observability

`HttpClientError` separates invalid URLs/headers, build and encode failures,
transport failures, timeouts, concurrency shutdown, decode errors, HTTP status
errors, and exhausted fake responses. It converts into Foundry's application
`Error`, so handlers, jobs, and providers can use `?` directly.

Tracing uses the `foundry.http_client` target. URLs have credentials removed,
fragments discarded, and every query value redacted. Header values, bodies, and
transport error text are never written to framework traces; debug output shows
only header names and body byte counts.

`client.raw()` exposes the underlying pooled `reqwest::Client` for capabilities
outside the wrapper. Raw calls bypass Foundry retries, concurrency limits,
timeouts, tracing, and fakes. It returns `None` for a custom/fake transport.

## Test outbound calls

`HttpClientFake` queues responses/errors and records typed requests in order:

```rust
let fake = HttpClientFake::new();
fake.respond_json(
    HttpStatus::OK,
    &serde_json::json!({ "status": "paid" }),
)?;

let app = TestApp::from_builder(build_app())
    .replace_service(fake.client())
    .build()
    .await?;

run_payment_sync(app.app()).await?;

fake.assert_sent_count(1).assert_sent(|request| {
    request.method() == HttpMethod::GET
        && request.url().path() == "/v2/payments/order-42"
        && request.header("authorization") == Some("Bearer test-token")
});

app.shutdown().await?;
```

Use `respond`, `respond_json`, `fail`, or `sequence` to prepare outcomes.
`requests`, `pending_responses`, `assert_sent`, `assert_not_sent`,
`assert_sent_count`, and `assert_nothing_sent` inspect behavior. An unexpected
extra request returns `HttpClientError::FakeExhausted` instead of touching the
network.

