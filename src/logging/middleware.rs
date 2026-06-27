use axum::extract::{MatchedPath, Request, State};
use axum::http::header::HeaderName;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tracing::Instrument;

use super::context::CurrentRequest;
use super::request_id::{generate_request_id, RequestId, REQUEST_ID_HEADER};
use super::{
    catch_future_panic, normalized_observability_base_path, panic_payload_message,
    scope_current_request, scope_current_trace, HttpRequestRecord, TraceContext,
};
use crate::foundation::AppContext;

pub(crate) async fn request_context_middleware(
    State(app): State<AppContext>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(generate_request_id);

    request
        .extensions_mut()
        .insert(RequestId::new(request_id.clone()));

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let route_path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string())
        .unwrap_or_else(|| path.clone());
    let user_agent = request
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let span = tracing::info_span!(
        "foundry.http.request",
        method = %method,
        path = %path,
        request_id = %request_id,
        trace_id = %request_id
    );

    let locale = resolve_request_locale(&request, &app);
    let start = std::time::Instant::now();
    let trace_context = TraceContext::http(request_id.clone());
    let execution_context = super::ExecutionContext::Http {
        method: method.to_string(),
        path: path.clone(),
        request_id: Some(request_id.clone()),
    };
    let database_config = app.config().database().ok();
    let observability_config = app.config().observability().ok();
    let response = scope_current_trace(
        trace_context,
        crate::database::scope_http_sql_query_trace(
            database_config,
            observability_config,
            method.to_string(),
            path.clone(),
            Some(request_id.clone()),
            super::scope_current_execution(
                execution_context,
                crate::database::scope_model_extensions(
                    crate::translations::CURRENT_LOCALE
                        .scope(locale, next.run(request).instrument(span)),
                ),
            ),
        ),
    );
    let mut response = match catch_future_panic(response).await {
        Ok(response) => response,
        Err(panic) => {
            let message = panic_payload_message(panic);
            tracing::error!(
                method = %method,
                path = %path,
                request_id = %request_id,
                trace_id = %request_id,
                panic = %message,
                "HTTP request panicked"
            );
            crate::foundation::Error::message(format!("http handler panicked: {message}"))
                .into_response()
        }
    };
    response = crate::http::middleware::normalize_edge_rejection_response(response);
    let duration_ms = start.elapsed().as_millis() as u64;

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(REQUEST_ID_HEADER), value);
    }
    let current_request = response
        .extensions()
        .get::<CurrentRequest>()
        .cloned()
        .unwrap_or(CurrentRequest {
            request_id: Some(request_id.clone()),
            ip: None,
            user_agent,
            audit_area: None,
        });
    let error_extension = response
        .extensions()
        .get::<super::reporter::HandlerErrorResponseExtension>()
        .cloned();
    let actor = response.extensions().get::<crate::auth::Actor>().cloned();
    super::report_handler_error_response(
        &app,
        method.as_str(),
        &path,
        &current_request,
        actor,
        error_extension,
    )
    .await;
    let status = response.status();
    if let Ok(diagnostics) = app.diagnostics() {
        if let Some(rejection) = crate::http::middleware::edge_rejection_from_response(&response) {
            diagnostics.record_http_edge_rejection(rejection);
        }
        if should_sample_http_request(&app, &path) {
            diagnostics.record_http_request(HttpRequestRecord {
                method: method.to_string(),
                path: route_path,
                status,
                duration_ms,
                request_id: current_request.request_id.clone(),
                trace_id: Some(request_id.clone()),
            });
        } else {
            diagnostics.record_http_response_with_duration(status, duration_ms);
        }
    }

    tracing::info!(
        method = %method,
        path = %path,
        status = status.as_u16(),
        duration_ms = duration_ms,
        request_id = %request_id,
        trace_id = %request_id,
        "Request completed"
    );

    response
}

fn should_sample_http_request(app: &AppContext, path: &str) -> bool {
    app.config()
        .observability()
        .map(|config| !path_is_under_observability_base(path, &config.base_path))
        .unwrap_or(true)
}

fn path_is_under_observability_base(path: &str, base_path: &str) -> bool {
    let base_path = normalized_observability_base_path(base_path);
    if base_path == "/" {
        return false;
    }

    path == base_path
        || path
            .strip_prefix(&base_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(crate) async fn request_origin_middleware(
    State(_app): State<AppContext>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let current = CurrentRequest::from_parts(&parts);
    let request = Request::from_parts(parts, body);

    let mut response = scope_current_request(current.clone(), next.run(request)).await;
    let current = response
        .extensions()
        .get::<CurrentRequest>()
        .cloned()
        .unwrap_or(current);
    response.extensions_mut().insert(current);
    response
}

fn resolve_request_locale(request: &Request, app: &AppContext) -> String {
    if let Some(locale) = request.extensions().get::<crate::i18n::Locale>() {
        return locale.0.clone();
    }
    match app.i18n() {
        Ok(manager) => request
            .headers()
            .get("accept-language")
            .and_then(|v| v.to_str().ok())
            .map(|s| manager.resolve_locale(s))
            .unwrap_or_else(|| manager.default_locale().to_string()),
        Err(_) => "en".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::middleware;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::{path_is_under_observability_base, request_context_middleware};
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::logging::{
        current_execution, ErrorReporter, ErrorReporterRegistry, ExecutionContext,
        HandlerErrorReport, JobDeadLetteredReport, PanicReport, RuntimeDiagnostics,
        REQUEST_ID_HEADER,
    };
    use crate::validation::RuleRegistry;

    #[derive(Default)]
    struct StubReporter {
        handler_reports: Mutex<Vec<HandlerErrorReport>>,
    }

    #[async_trait]
    impl ErrorReporter for StubReporter {
        async fn report_handler_error(&self, report: HandlerErrorReport) {
            self.handler_reports.lock().unwrap().push(report);
        }

        async fn report_panic(&self, _report: PanicReport) {}

        async fn report_job_dead_lettered(&self, _report: JobDeadLetteredReport) {}
    }

    fn test_app_with_reporter(
        reporter: Arc<StubReporter>,
    ) -> (AppContext, Arc<RuntimeDiagnostics>) {
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        let diagnostics = Arc::new(RuntimeDiagnostics::default());
        let reporters: Vec<Arc<dyn ErrorReporter>> = vec![reporter];
        app.container().singleton_arc(diagnostics.clone()).unwrap();
        app.container()
            .singleton_arc(Arc::new(ErrorReporterRegistry::new(reporters)))
            .unwrap();
        (app, diagnostics)
    }

    #[tokio::test]
    async fn panicking_http_handler_returns_structured_500_and_records_error_path() {
        let reporter = Arc::new(StubReporter::default());
        let (app, diagnostics) = test_app_with_reporter(reporter.clone());
        let seen_context = Arc::new(Mutex::new(None));
        let handler_context = seen_context.clone();
        let router = axum::Router::new()
            .route(
                "/panic",
                get(move || {
                    let handler_context = handler_context.clone();
                    async move {
                        *handler_context.lock().unwrap() = current_execution();
                        panic!("http explode");
                        #[allow(unreachable_code)]
                        "unreachable"
                    }
                }),
            )
            .layer(middleware::from_fn_with_state(
                app.clone(),
                request_context_middleware,
            ));

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/panic")
                    .header(REQUEST_ID_HEADER, "req-http-panic")
                    .header(axum::http::header::USER_AGENT, "FoundryHTTP/1.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.headers().get(REQUEST_ID_HEADER).unwrap(),
            "req-http-panic"
        );

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload["message"], "Internal server error");
        assert_eq!(payload["status"], 500);

        assert_eq!(
            *seen_context.lock().unwrap(),
            Some(ExecutionContext::Http {
                method: "GET".to_string(),
                path: "/panic".to_string(),
                request_id: Some("req-http-panic".to_string()),
            })
        );

        let reports = reporter.handler_reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].method, "GET");
        assert_eq!(reports[0].path, "/panic");
        assert_eq!(reports[0].status, 500);
        assert_eq!(reports[0].error, "http handler panicked: http explode");
        assert_eq!(reports[0].request_id.as_deref(), Some("req-http-panic"));
        assert_eq!(
            reports[0]
                .origin
                .as_ref()
                .and_then(|origin| origin.user_agent.as_deref()),
            Some("FoundryHTTP/1.0")
        );

        let snapshot = diagnostics.snapshot().http;
        assert_eq!(snapshot.requests_total, 1);
        assert_eq!(snapshot.server_error_total, 1);
        assert_eq!(snapshot.success_total, 0);
        assert_eq!(snapshot.duration_ms.count, 1);

        let http = diagnostics.http_observability_snapshot();
        assert_eq!(http.stats.error_request_count, 1);
        assert_eq!(http.top_error_routes[0].path, "/panic");
        assert_eq!(http.top_error_routes[0].server_error_total, 1);
        assert_eq!(
            http.recent_error_requests[0].request_id.as_deref(),
            Some("req-http-panic")
        );
        assert_eq!(
            http.recent_error_requests[0].trace_id.as_deref(),
            Some("req-http-panic")
        );
    }

    #[tokio::test]
    async fn request_context_middleware_preserves_normal_responses() {
        let reporter = Arc::new(StubReporter::default());
        let (app, diagnostics) = test_app_with_reporter(reporter.clone());
        let router = axum::Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                app,
                request_context_middleware,
            ));

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header(REQUEST_ID_HEADER, "req-http-ok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(REQUEST_ID_HEADER).unwrap(),
            "req-http-ok"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.as_ref(), b"ok");
        assert!(reporter.handler_reports.lock().unwrap().is_empty());

        let snapshot = diagnostics.snapshot().http;
        assert_eq!(snapshot.requests_total, 1);
        assert_eq!(snapshot.success_total, 1);
        assert_eq!(snapshot.server_error_total, 0);
    }

    #[tokio::test]
    async fn request_context_middleware_normalizes_and_counts_edge_rejections() {
        let reporter = Arc::new(StubReporter::default());
        let (app, diagnostics) = test_app_with_reporter(reporter);
        let router = axum::Router::new()
            .route(
                "/large",
                get(|| async { StatusCode::PAYLOAD_TOO_LARGE.into_response() }),
            )
            .layer(middleware::from_fn_with_state(
                app,
                request_context_middleware,
            ));

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/large")
                    .header(REQUEST_ID_HEADER, "req-http-large")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "application/json"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload["message"], "Payload too large");
        assert_eq!(payload["status"], 413);

        let snapshot = diagnostics.snapshot().http;
        assert_eq!(snapshot.requests_total, 1);
        assert_eq!(snapshot.client_error_total, 1);
        assert_eq!(snapshot.edge_rejections.payload_too_large_total, 1);
    }

    #[tokio::test]
    async fn http_observability_groups_by_matched_route_path_when_available() {
        let reporter = Arc::new(StubReporter::default());
        let (app, diagnostics) = test_app_with_reporter(reporter);
        let router = axum::Router::new()
            .route("/users/{id}", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                app,
                request_context_middleware,
            ));

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/users/42")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let http = diagnostics.http_observability_snapshot();
        assert_eq!(http.top_slowest_routes[0].path, "/users/{id}");
        assert_eq!(http.top_slowest_routes[0].requests_total, 1);
    }

    #[tokio::test]
    async fn http_observability_samples_skip_foundry_observability_routes() {
        let reporter = Arc::new(StubReporter::default());
        let (app, diagnostics) = test_app_with_reporter(reporter);
        let router = axum::Router::new()
            .route("/_foundry/runtime", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                app,
                request_context_middleware,
            ));

        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/_foundry/runtime")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let counters = diagnostics.snapshot().http;
        assert_eq!(counters.requests_total, 1);
        assert_eq!(counters.success_total, 1);
        assert_eq!(counters.duration_ms.count, 1);

        let http = diagnostics.http_observability_snapshot();
        assert_eq!(http.stats.retained_request_count, 0);
        assert!(http.top_slowest_routes.is_empty());
    }

    #[test]
    fn observability_path_filter_matches_only_configured_base_path() {
        assert!(path_is_under_observability_base(
            "/_foundry/http/stats",
            "/_foundry"
        ));
        assert!(path_is_under_observability_base("/_ops/ws/stats", "/_ops/"));
        assert!(!path_is_under_observability_base(
            "/_foundry-admin",
            "/_foundry"
        ));
        assert!(!path_is_under_observability_base("/users", "/_foundry"));
        assert!(!path_is_under_observability_base("/users", "/"));
    }
}
