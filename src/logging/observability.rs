use std::sync::OnceLock;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::Serialize;

use super::metrics;
use crate::auth::AccessScope;
use crate::config::ObservabilityConfig;
use crate::database::{DbValue, Expr, OrderBy, Query, Sql};
use crate::foundation::{AppContext, Error, Result};
use crate::http::{
    wrap_http_authorize_callback, HttpAuthorizeContext, HttpRegistrar, HttpRouteOptions,
};
use crate::openapi::spec::{generate_openapi_spec, DocumentedRoute};
use crate::support::{GuardId, PermissionId};

#[derive(Default)]
pub struct ObservabilityOptions {
    access: AccessScope,
    authorize: Option<crate::http::HttpAuthorizeCallback>,
}

impl Clone for ObservabilityOptions {
    fn clone(&self) -> Self {
        Self {
            access: self.access.clone(),
            authorize: self.authorize.clone(),
        }
    }
}

impl std::fmt::Debug for ObservabilityOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservabilityOptions")
            .field("access", &self.access)
            .field("has_authorize", &self.authorize.is_some())
            .finish()
    }
}

impl PartialEq for ObservabilityOptions {
    fn eq(&self, other: &Self) -> bool {
        self.access == other.access
            && match (&self.authorize, &other.authorize) {
                (None, None) => true,
                (Some(left), Some(right)) => std::sync::Arc::ptr_eq(left, right),
                _ => false,
            }
    }
}

impl Eq for ObservabilityOptions {}

impl ObservabilityOptions {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ObservabilityOptions {
    pub fn guard<I>(mut self, guard: I) -> Self
    where
        I: Into<GuardId>,
    {
        self.access = self.access.with_guard(guard);
        self
    }

    pub fn permission<I>(mut self, permission: I) -> Self
    where
        I: Into<PermissionId>,
    {
        self.access = self.access.with_permission(permission);
        self
    }

    pub fn permissions<I, P>(mut self, permissions: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PermissionId>,
    {
        self.access = self.access.with_permissions(permissions);
        self
    }

    /// Add a dynamic authorization callback for all observability routes.
    ///
    /// Called after guard and permission checks succeed. Return `Ok(())` to
    /// allow access or `Err(...)` to reject with a project-defined response.
    pub fn authorize<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(HttpAuthorizeContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.authorize = Some(wrap_http_authorize_callback(f));
        self
    }

    pub fn access(&self) -> &AccessScope {
        &self.access
    }

    pub(crate) fn http_route_options(&self) -> HttpRouteOptions {
        let mut opts = HttpRouteOptions::new();
        opts.access = self.access.clone();
        opts.authorize = self.authorize.clone();
        opts
    }
}

#[derive(Debug, Serialize)]
struct JobsStatsResponse {
    stats: Vec<JobStatusCountResponse>,
}

#[derive(Debug, Serialize)]
struct JobStatusCountResponse {
    status: String,
    count: i64,
}

#[derive(Debug, Serialize)]
struct JobsFailedResponse {
    failed_jobs: Vec<FailedJobResponse>,
}

#[derive(Debug, Serialize)]
struct FailedJobResponse {
    job_id: String,
    queue: String,
    status: String,
    attempt: Option<i64>,
    error: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    duration_ms: Option<i64>,
    created_at: Option<String>,
    request_id: Option<String>,
    trace_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct WebSocketChannelsResponse {
    channels: Vec<crate::websocket::WebSocketChannelDescriptor>,
}

#[derive(Debug, Serialize)]
struct WebSocketPresenceResponse {
    channel: String,
    count: usize,
    members: Vec<WebSocketPresenceMemberResponse>,
}

#[derive(Debug, Serialize)]
struct WebSocketPresenceMemberResponse {
    actor_id: String,
    joined_at: i64,
}

#[derive(Debug, Serialize)]
struct WebSocketHistoryResponse {
    channel: String,
    messages: Vec<WebSocketHistoryMessageResponse>,
}

#[derive(Debug, Serialize)]
struct WebSocketHistoryMessageResponse {
    channel: String,
    event: String,
    room: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_size_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
struct WebSocketStatsResponse {
    global: WebSocketGlobalStatsResponse,
    channels: Vec<WebSocketChannelStatsResponse>,
}

#[derive(Debug, Serialize)]
struct WebSocketGlobalStatsResponse {
    active_connections: u64,
    active_subscriptions: u64,
    subscriptions_total: u64,
    unsubscribes_total: u64,
    inbound_messages_total: u64,
    outbound_messages_total: u64,
    opened_total: u64,
    closed_total: u64,
}

#[derive(Debug, Serialize)]
struct WebSocketChannelStatsResponse {
    id: String,
    subscriptions_total: u64,
    unsubscribes_total: u64,
    active_subscriptions: u64,
    inbound_messages_total: u64,
    outbound_messages_total: u64,
}

pub(crate) fn register_observability_routes(
    registrar: &mut HttpRegistrar,
    config: &ObservabilityConfig,
    options: &ObservabilityOptions,
) -> Result<()> {
    let route_options = options.http_route_options();
    registrar.route_with_options(
        &join_route(&config.base_path, "health"),
        get(observability_liveness),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ready"),
        get(observability_readiness),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "runtime"),
        get(observability_runtime),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "http/stats"),
        get(http_stats),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "metrics"),
        get(observability_metrics),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "jobs/stats"),
        get(jobs_stats),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "jobs/failed"),
        get(jobs_failed),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "sql"),
        get(slow_queries),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/presence/{channel}"),
        get(ws_presence),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/channels"),
        get(ws_channels),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/history/{channel}"),
        get(ws_history),
        route_options.clone(),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/stats"),
        get(ws_stats),
        route_options,
    );
    Ok(())
}

async fn observability_liveness(State(app): State<AppContext>) -> Response {
    match app.diagnostics() {
        Ok(diagnostics) => (StatusCode::OK, Json(diagnostics.liveness())).into_response(),
        Err(error) => internal_error_response(error),
    }
}

async fn observability_readiness(State(app): State<AppContext>) -> Response {
    match app.diagnostics() {
        Ok(diagnostics) => match diagnostics.run_readiness_checks(&app).await {
            Ok(report) => {
                let status = if report.state.is_healthy() {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                (status, Json(report)).into_response()
            }
            Err(error) => internal_error_response(error),
        },
        Err(error) => internal_error_response(error),
    }
}

async fn observability_runtime(State(app): State<AppContext>) -> Response {
    match app.diagnostics() {
        Ok(diagnostics) => (StatusCode::OK, Json(diagnostics.snapshot())).into_response(),
        Err(error) => internal_error_response(error),
    }
}

async fn http_stats(State(app): State<AppContext>) -> Response {
    match app.diagnostics() {
        Ok(diagnostics) => (
            StatusCode::OK,
            Json(diagnostics.http_observability_snapshot()),
        )
            .into_response(),
        Err(error) => internal_error_response(error),
    }
}

async fn observability_metrics(State(app): State<AppContext>) -> Response {
    match app.diagnostics() {
        Ok(diagnostics) => {
            let body = metrics::format_prometheus(&diagnostics.snapshot());
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, metrics::PROMETHEUS_CONTENT_TYPE)],
                body,
            )
                .into_response()
        }
        Err(error) => internal_error_response(error),
    }
}

async fn jobs_stats(State(app): State<AppContext>) -> Response {
    let db = match app.database() {
        Ok(db) => db,
        Err(error) => return internal_error_response(error),
    };

    match Query::table("job_history")
        .select(["status"])
        .select_expr(Sql::count_all(), "count")
        .group_by("status")
        .order_by(OrderBy::asc("status"))
        .get(db.as_ref())
        .await
    {
        Ok(rows) => {
            let stats = rows
                .iter()
                .map(|row| JobStatusCountResponse {
                    status: db_string(row.get("status")).unwrap_or_else(|| "unknown".into()),
                    count: db_i64(row.get("count")).unwrap_or(0),
                })
                .collect();
            (StatusCode::OK, Json(JobsStatsResponse { stats })).into_response()
        }
        Err(error) => internal_error_response(error),
    }
}

async fn jobs_failed(State(app): State<AppContext>) -> Response {
    let db = match app.database() {
        Ok(db) => db,
        Err(error) => return internal_error_response(error),
    };

    match Query::table("job_history")
        .select([
            "job_id",
            "queue",
            "status",
            "attempt",
            "error",
            "started_at",
            "completed_at",
            "duration_ms",
            "created_at",
        ])
        .select_expr(
            Expr::column("payload")
                .json()
                .key("trace")
                .key("request_id")
                .as_text(),
            "request_id",
        )
        .select_expr(
            Expr::column("payload")
                .json()
                .key("trace")
                .key("trace_id")
                .as_text(),
            "trace_id",
        )
        .where_in("status", ["dead_lettered", "retried"])
        .order_by(OrderBy::desc("created_at"))
        .limit(50)
        .get(db.as_ref())
        .await
    {
        Ok(rows) => {
            let failed_jobs = rows
                .iter()
                .map(|row| FailedJobResponse {
                    job_id: db_string(row.get("job_id")).unwrap_or_else(|| "unknown".into()),
                    queue: db_string(row.get("queue")).unwrap_or_else(|| "unknown".into()),
                    status: db_string(row.get("status")).unwrap_or_else(|| "unknown".into()),
                    attempt: db_i64(row.get("attempt")),
                    error: db_string(row.get("error")),
                    started_at: db_string(row.get("started_at")),
                    completed_at: db_string(row.get("completed_at")),
                    duration_ms: db_i64(row.get("duration_ms")),
                    created_at: db_string(row.get("created_at")),
                    request_id: db_string(row.get("request_id")),
                    trace_id: db_string(row.get("trace_id")),
                })
                .collect();
            (StatusCode::OK, Json(JobsFailedResponse { failed_jobs })).into_response()
        }
        Err(error) => internal_error_response(error),
    }
}

async fn slow_queries(State(app): State<AppContext>) -> Response {
    let database_config = match app.config().database() {
        Ok(config) => config,
        Err(error) => return internal_error_response(error),
    };
    let snapshot = crate::database::sql_observability_snapshot(
        database_config.slow_query_threshold_ms,
        database_config.slow_query_retention,
    );
    (StatusCode::OK, Json(snapshot)).into_response()
}

async fn ws_channels(State(app): State<AppContext>) -> Response {
    let registry = match app.websocket_channels() {
        Ok(registry) => registry,
        Err(error) => return internal_error_response(error),
    };
    (
        StatusCode::OK,
        Json(WebSocketChannelsResponse {
            channels: registry.descriptors(),
        }),
    )
        .into_response()
}

async fn ws_presence(
    State(app): State<AppContext>,
    axum::extract::Path(channel): axum::extract::Path<crate::support::ChannelId>,
) -> Response {
    let registry = match app.websocket_channels() {
        Ok(registry) => registry,
        Err(error) => return internal_error_response(error),
    };
    let descriptor = match registry.find(&channel) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "channel not registered" })),
            )
                .into_response();
        }
    };
    if !descriptor.presence {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "presence not enabled for channel" })),
        )
            .into_response();
    }

    let backend = match crate::support::runtime::RuntimeBackend::from_config(app.config()) {
        Ok(b) => b,
        Err(error) => return internal_error_response(error),
    };
    let raw = match backend
        .smembers(&crate::websocket::presence_key(&channel))
        .await
    {
        Ok(members) => members,
        Err(error) => return internal_error_response(error),
    };

    let members: Vec<WebSocketPresenceMemberResponse> = raw
        .iter()
        .filter_map(|s| serde_json::from_str::<crate::websocket::PresenceInfo>(s).ok())
        .map(|info| WebSocketPresenceMemberResponse {
            actor_id: info.actor_id,
            joined_at: info.joined_at,
        })
        .collect();

    (
        StatusCode::OK,
        Json(WebSocketPresenceResponse {
            channel: channel.as_str().to_string(),
            count: members.len(),
            members,
        }),
    )
        .into_response()
}

fn db_string(value: Option<&DbValue>) -> Option<String> {
    match value {
        Some(DbValue::Text(value)) => Some(value.clone()),
        Some(DbValue::Uuid(value)) => Some(value.to_string()),
        Some(DbValue::TimestampTz(value)) => Some(value.to_string()),
        Some(DbValue::Timestamp(value)) => Some(value.to_string()),
        Some(DbValue::Date(value)) => Some(value.to_string()),
        Some(DbValue::Time(value)) => Some(value.to_string()),
        Some(DbValue::Null(_)) | None => None,
        Some(value) => Some(value.relation_key()),
    }
}

fn db_i64(value: Option<&DbValue>) -> Option<i64> {
    match value {
        Some(DbValue::Int16(value)) => Some(i64::from(*value)),
        Some(DbValue::Int32(value)) => Some(i64::from(*value)),
        Some(DbValue::Int64(value)) => Some(*value),
        Some(DbValue::Null(_)) | None => None,
        _ => None,
    }
}

fn internal_error_response(error: Error) -> Response {
    let error_text = error.to_string();
    let chain = error.source_chain();
    let mut response = (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "message": Error::internal_server_error_message(),
        })),
    )
        .into_response();
    crate::logging::mark_handler_error_response(
        &mut response,
        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        error_text,
        chain,
    );
    response
}

/// Cached OpenAPI spec shared across requests.
static OPENAPI_SPEC: OnceLock<serde_json::Value> = OnceLock::new();

/// Store the OpenAPI spec for serving. Call this at bootstrap with
/// the collected documented routes.
pub(crate) fn set_openapi_spec(title: &str, version: &str, routes: &[DocumentedRoute]) {
    let spec = generate_openapi_spec(title, version, routes);
    let _ = OPENAPI_SPEC.set(spec);
}

pub(crate) fn register_openapi_route(
    registrar: &mut HttpRegistrar,
    config: &ObservabilityConfig,
    options: &ObservabilityOptions,
) -> Result<()> {
    registrar.route_with_options(
        &join_route(&config.base_path, "openapi.json"),
        get(openapi_spec_handler),
        options.http_route_options(),
    );
    Ok(())
}

async fn openapi_spec_handler() -> Response {
    match OPENAPI_SPEC.get() {
        Some(spec) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(spec.clone()),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"message": "OpenAPI spec not available"})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct WsHistoryQuery {
    limit: Option<i64>,
}

async fn ws_history(
    State(app): State<AppContext>,
    axum::extract::Path(channel): axum::extract::Path<crate::support::ChannelId>,
    axum::extract::Query(params): axum::extract::Query<WsHistoryQuery>,
) -> Response {
    let registry = match app.websocket_channels() {
        Ok(registry) => registry,
        Err(error) => return internal_error_response(error),
    };
    if registry.find(&channel).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "channel not registered" })),
        )
            .into_response();
    }

    let websocket_config = match app.config().websocket() {
        Ok(config) => config,
        Err(error) => return internal_error_response(error),
    };
    let history_buffer_max = websocket_config.history_buffer_size.max(1) as i64;
    let limit = params
        .limit
        .unwrap_or(history_buffer_max)
        .clamp(1, history_buffer_max);

    let backend = match crate::support::runtime::RuntimeBackend::from_config(app.config()) {
        Ok(backend) => backend,
        Err(error) => return internal_error_response(error),
    };

    let history_key = format!("ws:history:{}", channel.as_str());
    let entries = match backend.lrange(&history_key, 0, limit - 1).await {
        Ok(e) => e,
        Err(error) => return internal_error_response(error),
    };

    let include_payloads = match app.config().observability() {
        Ok(cfg) => cfg.websocket.include_payloads,
        Err(error) => return internal_error_response(error),
    };

    let messages: Vec<WebSocketHistoryMessageResponse> = entries
        .iter()
        .filter_map(|raw| {
            let message = serde_json::from_str::<crate::websocket::ServerMessage>(raw).ok()?;
            let (payload, payload_size_bytes) = if include_payloads {
                (Some(message.payload), None)
            } else {
                let payload_size_bytes = serde_json::to_vec(&message.payload)
                    .map(|value| value.len() as u64)
                    .unwrap_or(0);
                (None, Some(payload_size_bytes))
            };
            Some(WebSocketHistoryMessageResponse {
                channel: message.channel.as_str().to_string(),
                event: message.event.as_str().to_string(),
                room: message.room,
                payload,
                payload_size_bytes,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(WebSocketHistoryResponse {
            channel: channel.as_str().to_string(),
            messages,
        }),
    )
        .into_response()
}

async fn ws_stats(State(app): State<AppContext>) -> Response {
    let diagnostics = match app.diagnostics() {
        Ok(d) => d,
        Err(error) => return internal_error_response(error),
    };
    let ws = diagnostics.snapshot().websocket;

    let channels: Vec<WebSocketChannelStatsResponse> = ws
        .channels
        .iter()
        .map(|channel| WebSocketChannelStatsResponse {
            id: channel.id.as_str().to_string(),
            subscriptions_total: channel.subscriptions_total,
            unsubscribes_total: channel.unsubscribes_total,
            active_subscriptions: channel.active_subscriptions,
            inbound_messages_total: channel.inbound_messages_total,
            outbound_messages_total: channel.outbound_messages_total,
        })
        .collect();

    (
        StatusCode::OK,
        Json(WebSocketStatsResponse {
            global: WebSocketGlobalStatsResponse {
                active_connections: ws.active_connections,
                active_subscriptions: ws.active_subscriptions,
                subscriptions_total: ws.subscriptions_total,
                unsubscribes_total: ws.unsubscribes_total,
                inbound_messages_total: ws.inbound_messages_total,
                outbound_messages_total: ws.outbound_messages_total,
                opened_total: ws.opened_total,
                closed_total: ws.closed_total,
            },
            channels,
        }),
    )
        .into_response()
}

fn join_route(base_path: &str, suffix: &str) -> String {
    let trimmed = base_path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        format!("/{suffix}")
    } else {
        format!("{trimmed}/{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    #[tokio::test]
    async fn internal_observability_errors_use_generic_public_message() {
        let response = internal_error_response(Error::message(
            "database URL postgres://user:secret@example.test/app leaked",
        ));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["message"], Error::internal_server_error_message());
        assert!(!payload["message"].as_str().unwrap().contains("secret"));
        assert!(!payload["message"].as_str().unwrap().contains("postgres://"));
    }
}
