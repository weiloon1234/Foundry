use std::fs;
use std::path::Path;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderValue, Method, Request, StatusCode};
use foundry::prelude::*;
use foundry::testing::TestApp;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

fn write_config(path: &Path, body: &str) {
    fs::write(path.join("foundry.toml"), body).unwrap();
}

fn edge_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.route("/ok", get(ok));
    registrar.route("/echo", post(echo));
    registrar.route("/slow", get(slow));
    Ok(())
}

async fn ok() -> &'static str {
    "ok"
}

async fn echo(Json(payload): Json<Value>) -> impl IntoResponse {
    Json(payload)
}

async fn slow() -> &'static str {
    tokio::time::sleep(Duration::from_millis(50)).await;
    "slow"
}

#[tokio::test]
async fn global_http_edge_config_applies_json_body_timeout_rejections_and_records_observability() {
    let directory = tempdir().unwrap();
    write_config(
        directory.path(),
        r#"
            [observability]
            enabled = true
            capture_enabled = true

            [http]
            max_body_size_bytes = 64
            request_timeout_ms = 1
        "#,
    );

    let app = TestApp::builder()
        .load_config_dir(directory.path())
        .enable_public_observability()
        .register_routes(edge_routes)
        .build()
        .await
        .unwrap();

    let first = app.client().get("/ok").send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.header("x-frame-options"), Some("DENY"));
    assert!(first.header("strict-transport-security").is_none());

    let too_large = app
        .client()
        .post("/echo")
        .json(&serde_json::json!({ "payload": "x".repeat(256) }))
        .unwrap()
        .send()
        .await
        .unwrap();
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        too_large.header(header::CONTENT_TYPE.as_str()),
        Some("application/json")
    );
    let too_large_json: Value = too_large.json().unwrap();
    assert_eq!(too_large_json["message"], "Payload too large");
    assert_eq!(too_large_json["status"], 413);

    let timeout = app.client().get("/slow").send().await.unwrap();
    assert_eq!(timeout.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(
        timeout.header(header::CONTENT_TYPE.as_str()),
        Some("application/json")
    );
    let timeout_json: Value = timeout.json().unwrap();
    assert_eq!(timeout_json["message"], "Request timed out");
    assert_eq!(timeout_json["status"], 408);

    let runtime = app.client().get("/_foundry/runtime").send().await.unwrap();
    assert_eq!(runtime.status(), StatusCode::OK);
    let runtime_json: Value = runtime.json().unwrap();
    assert_eq!(
        runtime_json["http"]["edge_rejections"]["payload_too_large_total"],
        1
    );
    assert_eq!(runtime_json["http"]["edge_rejections"]["timeout_total"], 1);
}

#[tokio::test]
async fn global_rate_limit_returns_json_429_and_records_observability() {
    let directory = tempdir().unwrap();
    write_config(
        directory.path(),
        r#"
            [observability]
            enabled = true
            capture_enabled = true

            [http.rate_limit]
            enabled = true
            max_requests = 1
            window_seconds = 60
        "#,
    );

    let app = TestApp::builder()
        .load_config_dir(directory.path())
        .enable_public_observability()
        .register_routes(edge_routes)
        .build()
        .await
        .unwrap();

    let first = app.client().get("/ok").send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let limited = app.client().get("/ok").send().await.unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        limited.header(header::CONTENT_TYPE.as_str()),
        Some("application/json")
    );
    let limited_json: Value = limited.json().unwrap();
    assert_eq!(limited_json["message"], "Rate limit exceeded");
    assert_eq!(limited_json["status"], 429);

    let runtime = app.client().get("/_foundry/runtime").send().await.unwrap();
    assert_eq!(runtime.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        app.app()
            .diagnostics()
            .unwrap()
            .snapshot()
            .http
            .edge_rejections
            .rate_limited_total,
        2
    );
}

#[tokio::test]
async fn explicit_app_middleware_overrides_config_derived_duplicate_kind() {
    let directory = tempdir().unwrap();
    write_config(
        directory.path(),
        r#"
            [http.security_headers]
            enabled = true
            hsts = false
        "#,
    );

    let app = TestApp::builder()
        .load_config_dir(directory.path())
        .register_routes(edge_routes)
        .register_middleware(SecurityHeaders::new().build())
        .build()
        .await
        .unwrap();

    let response = app.client().get("/ok").send().await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.header("strict-transport-security").is_some());
}

#[tokio::test]
async fn config_derived_cors_preflight_uses_validated_allow_list() {
    let directory = tempdir().unwrap();
    write_config(
        directory.path(),
        r#"
            [http.cors]
            enabled = true
            allowed_origins = ["https://example.com"]
            allowed_methods = ["GET", "POST"]
            allowed_headers = ["authorization"]
        "#,
    );

    let kernel = App::builder()
        .load_config_dir(directory.path())
        .register_routes(edge_routes)
        .build_http_kernel()
        .await
        .unwrap();
    let router = kernel.build_router().unwrap();
    let request = Request::builder()
        .method(Method::OPTIONS)
        .uri("/ok")
        .header(header::ORIGIN, "https://example.com")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&HeaderValue::from_static("https://example.com"))
    );
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_METHODS),
        Some(&HeaderValue::from_static("GET,POST"))
    );
}
