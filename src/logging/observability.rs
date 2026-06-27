use std::sync::OnceLock;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::Serialize;

use super::diagnostics::RuntimeSnapshot;
use super::metrics;
use super::probes::{LivenessReport, ReadinessReport};
use crate::app_enum::FoundryAppEnum;
use crate::auth::AccessScope;
use crate::config::ObservabilityConfig;
use crate::database::{DbValue, Expr, OrderBy, Query, Sql};
use crate::foundation::{AppContext, Error, ErrorResponse, Result};
use crate::http::{
    wrap_http_authorize_callback, HttpAuthorizeContext, HttpRegistrar, HttpRouteOptions,
};
use crate::jobs::JobHistoryStatus;
use crate::openapi::spec::{try_generate_openapi_spec_with_validation_rules, DocumentedRoute};
use crate::support::{GuardId, PermissionId};
use crate::validation::ValidationRuleDescriptor;

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

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct JobsStatsResponse {
    stats: Vec<JobStatusCountResponse>,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct JobStatusCountResponse {
    status: JobHistoryStatus,
    #[ts(type = "number")]
    count: i64,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct JobsFailedResponse {
    failed_jobs: Vec<FailedJobResponse>,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS)]
struct FailedJobResponse {
    job_id: String,
    queue: String,
    status: JobHistoryStatus,
    #[ts(type = "number | null")]
    attempt: Option<i64>,
    error: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    #[ts(type = "number | null")]
    duration_ms: Option<i64>,
    created_at: Option<String>,
    request_id: Option<String>,
    trace_id: Option<String>,
}

impl crate::openapi::ApiSchema for FailedJobResponse {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string" },
                "queue": { "type": "string" },
                "status": <JobHistoryStatus as crate::openapi::ApiSchema>::schema(),
                "attempt": { "type": "integer", "format": "int64", "nullable": true },
                "error": { "type": "string", "nullable": true },
                "started_at": { "type": "string", "nullable": true },
                "completed_at": { "type": "string", "nullable": true },
                "duration_ms": { "type": "integer", "format": "int64", "nullable": true },
                "created_at": { "type": "string", "nullable": true },
                "request_id": { "type": "string", "nullable": true },
                "trace_id": { "type": "string", "nullable": true },
            },
            "required": [
                "job_id",
                "queue",
                "status",
                "attempt",
                "error",
                "started_at",
                "completed_at",
                "duration_ms",
                "created_at",
                "request_id",
                "trace_id",
            ],
        })
    }

    fn schema_name() -> &'static str {
        "FailedJobResponse"
    }
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct WebSocketChannelsResponse {
    channels: Vec<crate::websocket::WebSocketChannelDescriptor>,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct WebSocketPresenceResponse {
    channel: String,
    #[ts(type = "number")]
    count: usize,
    members: Vec<WebSocketPresenceMemberResponse>,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct WebSocketPresenceMemberResponse {
    actor_id: String,
    #[ts(type = "number")]
    joined_at: i64,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct WebSocketHistoryResponse {
    channel: String,
    messages: Vec<WebSocketHistoryMessageResponse>,
}

#[derive(Debug, Serialize, foundry_macros::TS)]
struct WebSocketHistoryMessageResponse {
    channel: String,
    event: String,
    room: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    payload_size_bytes: Option<u64>,
}

impl ts_rs::TS for WebSocketHistoryMessageResponse {
    type WithoutGenerics = Self;

    fn name() -> String {
        "WebSocketHistoryMessageResponse".to_string()
    }

    fn decl() -> String {
        concat!(
            "type WebSocketHistoryMessageResponse = { ",
            "channel: string, ",
            "event: string, ",
            "room: string | null, ",
            "payload?: JsonValue, ",
            "payload_size_bytes?: number, ",
            "};",
        )
        .to_string()
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn inline() -> String {
        concat!(
            "{ ",
            "channel: string, ",
            "event: string, ",
            "room: string | null, ",
            "payload?: JsonValue, ",
            "payload_size_bytes?: number, ",
            "}",
        )
        .to_string()
    }

    fn inline_flattened() -> String {
        Self::inline()
    }

    fn output_path() -> Option<&'static std::path::Path> {
        Some(std::path::Path::new("WebSocketHistoryMessageResponse.ts"))
    }

    fn visit_dependencies(v: &mut impl ts_rs::TypeVisitor)
    where
        Self: 'static,
    {
        v.visit::<serde_json::Value>();
    }
}

impl crate::openapi::ApiSchema for WebSocketHistoryMessageResponse {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string" },
                "event": { "type": "string" },
                "room": { "type": "string", "nullable": true },
                "payload": {},
                "payload_size_bytes": { "type": "integer" },
            },
            "required": ["channel", "event", "room"],
        })
    }

    fn schema_name() -> &'static str {
        "WebSocketHistoryMessageResponse"
    }
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct WebSocketStatsResponse {
    global: WebSocketGlobalStatsResponse,
    channels: Vec<WebSocketChannelStatsResponse>,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct WebSocketGlobalStatsResponse {
    #[ts(type = "number")]
    active_connections: u64,
    #[ts(type = "number")]
    active_subscriptions: u64,
    #[ts(type = "number")]
    subscriptions_total: u64,
    #[ts(type = "number")]
    unsubscribes_total: u64,
    #[ts(type = "number")]
    inbound_messages_total: u64,
    #[ts(type = "number")]
    outbound_messages_total: u64,
    #[ts(type = "number")]
    opened_total: u64,
    #[ts(type = "number")]
    closed_total: u64,
}

#[derive(Debug, Serialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
struct WebSocketChannelStatsResponse {
    id: String,
    #[ts(type = "number")]
    subscriptions_total: u64,
    #[ts(type = "number")]
    unsubscribes_total: u64,
    #[ts(type = "number")]
    active_subscriptions: u64,
    #[ts(type = "number")]
    inbound_messages_total: u64,
    #[ts(type = "number")]
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
        observability_json_route::<LivenessReport>(
            &route_options,
            "foundryHealth",
            "Get Foundry liveness",
        ),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ready"),
        get(observability_readiness),
        observability_json_route::<ReadinessReport>(
            &route_options,
            "foundryReadiness",
            "Get Foundry readiness",
        ),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "runtime"),
        get(observability_runtime),
        observability_json_route::<RuntimeSnapshot>(
            &route_options,
            "foundryRuntime",
            "Get Foundry runtime diagnostics",
        ),
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
        observability_json_route::<JobsStatsResponse>(
            &route_options,
            "foundryJobsStats",
            "Get Foundry job status counts",
        ),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "jobs/failed"),
        get(jobs_failed),
        observability_json_route::<JobsFailedResponse>(
            &route_options,
            "foundryJobsFailed",
            "List recent failed or retried jobs",
        ),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "sql"),
        get(slow_queries),
        observability_json_route::<crate::database::SqlObservabilitySnapshot>(
            &route_options,
            "foundrySqlObservability",
            "Get SQL observability snapshot",
        ),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/presence/{channel}"),
        get(ws_presence),
        observability_json_route::<WebSocketPresenceResponse>(
            &route_options,
            "foundryWebSocketPresence",
            "Get WebSocket presence members",
        )
        .response::<ErrorResponse>(404),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/channels"),
        get(ws_channels),
        observability_json_route::<WebSocketChannelsResponse>(
            &route_options,
            "foundryWebSocketChannels",
            "List registered WebSocket channels",
        ),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/history/{channel}"),
        get(ws_history),
        observability_json_route::<WebSocketHistoryResponse>(
            &route_options,
            "foundryWebSocketHistory",
            "Get WebSocket channel history",
        )
        .response::<ErrorResponse>(404),
    );
    registrar.route_with_options(
        &join_route(&config.base_path, "ws/stats"),
        get(ws_stats),
        observability_json_route::<WebSocketStatsResponse>(
            &route_options,
            "foundryWebSocketStats",
            "Get WebSocket runtime stats",
        ),
    );
    Ok(())
}

fn observability_json_route<T: crate::openapi::ApiSchema>(
    base: &HttpRouteOptions,
    operation_id: &str,
    summary: &str,
) -> HttpRouteOptions {
    base.clone()
        .tag("Observability")
        .operation_id(operation_id)
        .summary(summary)
        .response::<T>(200)
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
                .filter_map(|row| {
                    let status = db_job_history_status(row.get("status"))?;
                    Some(JobStatusCountResponse {
                        status,
                        count: db_i64(row.get("count")).unwrap_or(0),
                    })
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
                .filter_map(|row| {
                    let status = db_job_history_status(row.get("status"))?;
                    Some(FailedJobResponse {
                        job_id: db_string(row.get("job_id")).unwrap_or_else(|| "unknown".into()),
                        queue: db_string(row.get("queue")).unwrap_or_else(|| "unknown".into()),
                        status,
                        attempt: db_i64(row.get("attempt")),
                        error: db_string(row.get("error")),
                        started_at: db_string(row.get("started_at")),
                        completed_at: db_string(row.get("completed_at")),
                        duration_ms: db_i64(row.get("duration_ms")),
                        created_at: db_string(row.get("created_at")),
                        request_id: db_string(row.get("request_id")),
                        trace_id: db_string(row.get("trace_id")),
                    })
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
            return error_response(StatusCode::NOT_FOUND, "channel not registered");
        }
    };
    if !descriptor.presence {
        return error_response(StatusCode::NOT_FOUND, "presence not enabled for channel");
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

fn db_job_history_status(value: Option<&DbValue>) -> Option<JobHistoryStatus> {
    let status = db_string(value)?;
    JobHistoryStatus::parse_key(&status)
}

fn internal_error_response(error: Error) -> Response {
    let error_text = error.to_string();
    let chain = error.source_chain();
    let mut response = (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new(
            Error::internal_server_error_message(),
            StatusCode::INTERNAL_SERVER_ERROR,
        )),
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

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(ErrorResponse::new(message, status))).into_response()
}

/// Cached OpenAPI spec shared across requests.
static OPENAPI_SPEC: OnceLock<serde_json::Value> = OnceLock::new();

/// Store the OpenAPI spec for serving. Call this at bootstrap with
/// the collected documented routes.
pub(crate) fn set_openapi_spec(
    title: &str,
    version: &str,
    routes: &[DocumentedRoute],
    validation_rules: &[ValidationRuleDescriptor],
) -> Result<()> {
    let spec = try_generate_openapi_spec_with_validation_rules(
        title,
        version,
        routes,
        validation_rules,
        true,
    )?;
    let _ = OPENAPI_SPEC.set(spec);
    Ok(())
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
        None => error_response(StatusCode::NOT_FOUND, "OpenAPI spec not available"),
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
        return error_response(StatusCode::NOT_FOUND, "channel not registered");
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

pub(crate) fn normalized_observability_base_path(base_path: &str) -> String {
    let trimmed = base_path.trim_end_matches('/');
    match trimmed {
        "" | "/" => "/".to_string(),
        value if value.starts_with('/') => value.to_string(),
        value => format!("/{value}"),
    }
}

fn join_route(base_path: &str, suffix: &str) -> String {
    let normalized = normalized_observability_base_path(base_path);
    if normalized == "/" {
        format!("/{suffix}")
    } else {
        format!("{normalized}/{suffix}")
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

    #[test]
    fn observability_route_join_normalizes_configured_base_path() {
        assert_eq!(join_route("/_foundry/", "health"), "/_foundry/health");
        assert_eq!(join_route("_ops", "runtime"), "/_ops/runtime");
        assert_eq!(join_route("/", "metrics"), "/metrics");
        assert_eq!(join_route("", "ready"), "/ready");
    }

    #[test]
    fn observability_json_routes_are_documented_for_openapi() {
        let mut registrar = HttpRegistrar::new();
        register_observability_routes(
            &mut registrar,
            &ObservabilityConfig::default(),
            &ObservabilityOptions::new(),
        )
        .unwrap();

        let docs = registrar.collect_documented_routes();
        let spec = crate::openapi::spec::generate_openapi_spec("Foundry", "1.0.0", &docs);

        let health = &spec["paths"]["/_foundry/health"]["get"];
        assert_eq!(health["operationId"], serde_json::json!("foundryHealth"));
        assert_eq!(health["tags"], serde_json::json!(["Observability"]));
        assert_eq!(
            health["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/LivenessReport")
        );
        assert_eq!(
            spec["components"]["schemas"]["LivenessReport"]["properties"]["state"]["enum"],
            serde_json::json!(["healthy", "unhealthy"])
        );

        let readiness = &spec["paths"]["/_foundry/ready"]["get"];
        assert_eq!(
            readiness["operationId"],
            serde_json::json!("foundryReadiness")
        );
        assert_eq!(
            readiness["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/ReadinessReport")
        );
        let probe =
            &spec["components"]["schemas"]["ReadinessReport"]["properties"]["probes"]["items"];
        assert_eq!(
            probe["properties"]["id"],
            serde_json::json!({ "type": "string" })
        );
        assert_eq!(
            probe["properties"]["state"]["enum"],
            serde_json::json!(["healthy", "unhealthy"])
        );
        assert!(probe["required"].as_array().is_none_or(|required| !required
            .iter()
            .any(|field| field.as_str() == Some("message"))));

        let runtime = &spec["paths"]["/_foundry/runtime"]["get"];
        assert_eq!(runtime["operationId"], serde_json::json!("foundryRuntime"));
        assert_eq!(
            runtime["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/RuntimeSnapshot")
        );
        assert_eq!(
            spec["components"]["schemas"]["RuntimeSnapshot"]["properties"]["backend"]["enum"],
            serde_json::json!(["redis", "memory"])
        );

        let sql = &spec["paths"]["/_foundry/sql"]["get"];
        assert_eq!(
            sql["operationId"],
            serde_json::json!("foundrySqlObservability")
        );
        assert_eq!(sql["tags"], serde_json::json!(["Observability"]));
        assert_eq!(
            sql["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/SqlObservabilitySnapshot")
        );
        let sql_snapshot = &spec["components"]["schemas"]["SqlObservabilitySnapshot"];
        let top_slowest_item = &sql_snapshot["properties"]["top_slowest"]["items"];
        assert_eq!(
            top_slowest_item["properties"]["request_id"],
            serde_json::json!({ "type": "string", "nullable": true })
        );
        assert!(top_slowest_item["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field.as_str() == Some("duration_ms"))));
        let n_plus_one_item = &sql_snapshot["properties"]["n_plus_one_suspects"]["items"];
        assert_eq!(
            n_plus_one_item["properties"]["labels"]["x-foundry-item-schema"],
            serde_json::json!("String")
        );
        assert_eq!(
            sql_snapshot["properties"]["stats"]["properties"]["max_duration_ms"],
            serde_json::json!({ "type": "integer", "nullable": true })
        );

        let jobs_stats = &spec["paths"]["/_foundry/jobs/stats"]["get"];
        assert_eq!(
            jobs_stats["operationId"],
            serde_json::json!("foundryJobsStats")
        );
        assert_eq!(
            jobs_stats["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/JobsStatsResponse")
        );
        let job_status_count =
            &spec["components"]["schemas"]["JobsStatsResponse"]["properties"]["stats"]["items"];
        assert_eq!(
            job_status_count["properties"]["status"]["enum"],
            serde_json::json!(["succeeded", "retried", "dead_lettered"])
        );
        assert_eq!(
            job_status_count["properties"]["count"],
            serde_json::json!({ "type": "integer", "format": "int64" })
        );

        let jobs_failed = &spec["paths"]["/_foundry/jobs/failed"]["get"];
        assert_eq!(
            jobs_failed["operationId"],
            serde_json::json!("foundryJobsFailed")
        );
        assert_eq!(
            jobs_failed["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/JobsFailedResponse")
        );
        let failed_job = &spec["components"]["schemas"]["JobsFailedResponse"]["properties"]
            ["failed_jobs"]["items"];
        assert_eq!(
            failed_job["properties"]["status"]["enum"],
            serde_json::json!(["succeeded", "retried", "dead_lettered"])
        );
        assert_eq!(
            failed_job["properties"]["attempt"],
            serde_json::json!({ "type": "integer", "format": "int64", "nullable": true })
        );
        assert_eq!(
            failed_job["properties"]["request_id"],
            serde_json::json!({ "type": "string", "nullable": true })
        );
        assert!(failed_job["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field.as_str() == Some("request_id"))));

        let channels = &spec["paths"]["/_foundry/ws/channels"]["get"];
        assert_eq!(
            channels["operationId"],
            serde_json::json!("foundryWebSocketChannels")
        );
        assert_eq!(channels["tags"], serde_json::json!(["Observability"]));
        assert_eq!(
            channels["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/WebSocketChannelsResponse")
        );

        let channel_descriptor = &spec["components"]["schemas"]["WebSocketChannelsResponse"]
            ["properties"]["channels"]["items"];
        assert_eq!(
            channel_descriptor["properties"]["guard"],
            serde_json::json!({ "type": "string", "nullable": true })
        );
        assert!(channel_descriptor["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field.as_str() == Some("guard"))));

        let presence = &spec["paths"]["/_foundry/ws/presence/{channel}"]["get"];
        assert_eq!(
            presence["operationId"],
            serde_json::json!("foundryWebSocketPresence")
        );
        assert!(presence["parameters"]
            .as_array()
            .is_some_and(|parameters| parameters.iter().any(|parameter| parameter
                .get("name")
                .and_then(serde_json::Value::as_str)
                == Some("channel"))));
        assert_eq!(
            presence["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/WebSocketPresenceResponse")
        );
        assert_eq!(
            presence["responses"]["404"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/ErrorResponse")
        );

        let history = &spec["paths"]["/_foundry/ws/history/{channel}"]["get"];
        assert_eq!(
            history["operationId"],
            serde_json::json!("foundryWebSocketHistory")
        );
        assert_eq!(
            history["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/WebSocketHistoryResponse")
        );
        assert_eq!(
            history["responses"]["404"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/ErrorResponse")
        );
        let history_message = &spec["components"]["schemas"]["WebSocketHistoryResponse"]
            ["properties"]["messages"]["items"];
        assert_eq!(
            history_message["properties"]["room"],
            serde_json::json!({ "type": "string", "nullable": true })
        );
        assert!(history_message["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field.as_str() == Some("room"))));
        assert!(history_message["required"]
            .as_array()
            .is_none_or(|required| !required
                .iter()
                .any(|field| field.as_str() == Some("payload"))));

        let stats = &spec["paths"]["/_foundry/ws/stats"]["get"];
        assert_eq!(
            stats["operationId"],
            serde_json::json!("foundryWebSocketStats")
        );
        assert_eq!(
            stats["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/WebSocketStatsResponse")
        );
    }
}
