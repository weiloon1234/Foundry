use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use foundry::prelude::*;
use foundry::testing::TestApp;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tempfile::tempdir;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        #[derive(Clone, Copy)]
        pub enum ProbeKey {
            Database,
        }

        impl From<ProbeKey> for ProbeId {
            fn from(value: ProbeKey) -> Self {
                match value {
                    ProbeKey::Database => ProbeId::new("database.ready"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum AuthGuard {
            Api,
        }

        impl From<AuthGuard> for GuardId {
            fn from(value: AuthGuard) -> Self {
                match value {
                    AuthGuard::Api => GuardId::new("api"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum Ability {
            SecureView,
        }

        impl From<Ability> for PermissionId {
            fn from(value: Ability) -> Self {
                match value {
                    Ability::SecureView => PermissionId::new("secure:view"),
                }
            }
        }

        pub const AUDIT_JOB: JobId = JobId::new("audit.job");
        pub const HEARTBEAT_SCHEDULE: ScheduleId = ScheduleId::new("heartbeat");
        pub const CHAT_CHANNEL: ChannelId = ChannelId::new("chat");
        pub const ECHO_EVENT: ChannelEventId = ChannelEventId::new("echo");
    }

    pub mod domain {
        use super::*;

        #[derive(Debug, Serialize, Deserialize)]
        pub struct AuditJob {
            pub marker: String,
        }

        #[async_trait]
        impl Job for AuditJob {
            const ID: JobId = ids::AUDIT_JOB;

            async fn handle(&self, _context: JobContext) -> Result<()> {
                Ok(())
            }
        }
    }

    pub mod providers {
        use super::*;

        #[derive(Clone, Copy)]
        pub enum ProbeBehavior {
            Healthy,
            Unhealthy,
            Panic,
        }

        #[derive(Clone, Copy)]
        pub struct HttpServiceProvider {
            pub probe: ProbeBehavior,
        }

        pub struct DatabaseProbe {
            pub behavior: ProbeBehavior,
        }

        #[async_trait]
        impl ReadinessCheck for DatabaseProbe {
            async fn run(&self, _app: &AppContext) -> Result<ProbeResult> {
                match self.behavior {
                    ProbeBehavior::Healthy => Ok(ProbeResult::healthy(ids::ProbeKey::Database)),
                    ProbeBehavior::Unhealthy => Ok(ProbeResult::unhealthy(
                        ids::ProbeKey::Database,
                        "database offline",
                    )),
                    ProbeBehavior::Panic => panic!("database probe exploded"),
                }
            }
        }

        #[async_trait]
        impl ServiceProvider for HttpServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.register_guard(
                    ids::AuthGuard::Api,
                    StaticBearerAuthenticator::new().token(
                        "viewer-token",
                        Actor::new("viewer-1", ids::AuthGuard::Api)
                            .with_permissions([ids::Ability::SecureView]),
                    ),
                )?;
                registrar.register_readiness_check(
                    ids::ProbeKey::Database,
                    DatabaseProbe {
                        behavior: self.probe,
                    },
                )?;
                Ok(())
            }
        }

        #[derive(Clone)]
        pub struct WorkerServiceProvider;

        #[async_trait]
        impl ServiceProvider for WorkerServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.register_job::<domain::AuditJob>()?;
                Ok(())
            }
        }
    }

    pub mod http {
        use super::*;

        pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route("/public", get(public));
            registrar.route_with_options(
                "/secure",
                get(secure),
                HttpRouteOptions::new()
                    .guard(ids::AuthGuard::Api)
                    .permission(ids::Ability::SecureView),
            );
            Ok(())
        }

        async fn public(request_id: RequestId, actor: OptionalActor) -> impl IntoResponse {
            Json(serde_json::json!({
                "request_id": request_id.to_string(),
                "actor_id": actor.as_ref().map(|actor| actor.id.clone()),
            }))
        }

        async fn secure(actor: CurrentActor) -> impl IntoResponse {
            Json(serde_json::json!({
                "actor_id": actor.id,
            }))
        }
    }

    pub mod realtime {
        use super::*;

        pub fn register(registrar: &mut WebSocketRegistrar) -> Result<()> {
            registrar.channel(
                ids::CHAT_CHANNEL,
                |context: WebSocketContext, payload: serde_json::Value| async move {
                    context.publish(ids::ECHO_EVENT, payload).await
                },
            )?;
            Ok(())
        }
    }

    pub mod schedules {
        use super::*;

        pub fn register(registry: &mut ScheduleRegistry) -> Result<()> {
            registry.cron(
                ids::HEARTBEAT_SCHEDULE,
                CronExpression::parse("*/1 * * * * *")?,
                |_invocation| async move { Ok(()) },
            )?;
            Ok(())
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn unique_namespace(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::now_v7())
}

fn database_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn write_http_config(dir: &Path, server_port: u16, namespace: &str) {
    fs::write(
        dir.join("00-runtime.toml"),
        format!(
            r#"
            [server]
            host = "127.0.0.1"
            port = {server_port}

            [redis]
            namespace = "{namespace}"
        "#
        ),
    )
    .unwrap();
}

fn write_database_config(dir: &Path, url: &str) {
    fs::write(
        dir.join("00-database.toml"),
        format!(
            r#"
            [database]
            url = "{url}"
        "#
        ),
    )
    .unwrap();
}

async fn recreate_job_history_table(db: &DatabaseManager) {
    db.raw_execute("DROP TABLE IF EXISTS job_history", &[])
        .await
        .unwrap();
    db.raw_execute(
        r#"
        CREATE TABLE job_history (
            id UUID PRIMARY KEY,
            job_id TEXT NOT NULL,
            queue TEXT NOT NULL,
            status TEXT NOT NULL,
            payload JSONB,
            attempt INT NOT NULL DEFAULT 1,
            error TEXT,
            started_at TIMESTAMPTZ,
            completed_at TIMESTAMPTZ,
            duration_ms BIGINT,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
        &[],
    )
    .await
    .unwrap();
}

#[derive(Debug, Deserialize)]
struct JobsStatsContract {
    stats: Vec<JobStatusContract>,
}

#[derive(Debug, Deserialize)]
struct JobStatusContract {
    status: String,
    count: i64,
}

#[derive(Debug, Deserialize)]
struct JobsFailedContract {
    failed_jobs: Vec<FailedJobContract>,
}

#[derive(Debug, Deserialize)]
struct FailedJobContract {
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

#[derive(Debug, Deserialize)]
struct HttpStatsContract {
    stats: HttpStatsSummaryContract,
    top_slowest_routes: Vec<HttpRouteRankingContract>,
    top_error_routes: Vec<HttpRouteRankingContract>,
    recent_slow_requests: Vec<HttpRequestSampleContract>,
    recent_error_requests: Vec<HttpRequestSampleContract>,
}

#[derive(Debug, Deserialize)]
struct HttpStatsSummaryContract {
    requests_total: u64,
    retained_request_count: usize,
    retention_capacity: usize,
    slow_request_threshold_ms: u64,
    route_count: usize,
    slow_request_count: usize,
    error_request_count: usize,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct HttpRouteRankingContract {
    method: String,
    path: String,
    requests_total: u64,
    informational_total: u64,
    success_total: u64,
    redirection_total: u64,
    client_error_total: u64,
    server_error_total: u64,
    avg_duration_ms: u64,
    max_duration_ms: u64,
    p95_duration_ms: u64,
    p99_duration_ms: u64,
    latest_recorded_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct HttpRequestSampleContract {
    method: String,
    path: String,
    status: u16,
    duration_ms: u64,
    request_id: Option<String>,
    trace_id: Option<String>,
    recorded_at: String,
}

#[derive(Debug, Deserialize)]
struct SlowQueriesContract {
    stats: SqlStatsContract,
    top_slowest: Vec<SlowQueryContract>,
    n_plus_one_suspects: Vec<NPlusOneSuspectContract>,
    slow_queries: Vec<SlowQueryContract>,
}

#[derive(Debug, Deserialize)]
struct SqlStatsContract {
    retained_count: usize,
    capacity: usize,
    slow_query_threshold_ms: u64,
    max_duration_ms: Option<u64>,
    avg_duration_ms: Option<u64>,
    latest_recorded_at: Option<String>,
    n_plus_one_suspect_count: usize,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SlowQueryContract {
    sql: String,
    duration_ms: u64,
    label: Option<String>,
    request_id: Option<String>,
    trace_id: Option<String>,
    recorded_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct NPlusOneSuspectContract {
    method: String,
    path: String,
    request_id: Option<String>,
    trace_id: Option<String>,
    fingerprint: String,
    repeat_count: u64,
    total_duration_ms: u64,
    max_duration_ms: u64,
    avg_duration_ms: u64,
    rows_total: u64,
    labels: Vec<String>,
    kinds: Vec<String>,
    sample_sql: String,
    first_recorded_at: String,
    latest_recorded_at: String,
}

fn write_websocket_config(dir: &Path, websocket_port: u16, namespace: &str) {
    fs::write(
        dir.join("00-runtime.toml"),
        format!(
            r#"
            [websocket]
            host = "127.0.0.1"
            port = {websocket_port}
            path = "/ws"

            [redis]
            namespace = "{namespace}"
        "#
        ),
    )
    .unwrap();
}

fn write_scheduler_config(dir: &Path, namespace: &str) {
    fs::write(
        dir.join("00-runtime.toml"),
        format!(
            r#"
            [redis]
            namespace = "{namespace}"

            [jobs]
            queue = "default"
            max_retries = 3
            poll_interval_ms = 10
        "#
        ),
    )
    .unwrap();
}

fn build_http_app(config_dir: &Path, probe: app::providers::ProbeBehavior) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::HttpServiceProvider { probe })
        .register_routes(app::http::router)
        .enable_public_observability()
}

fn build_websocket_app(config_dir: &Path) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_websocket_routes(app::realtime::register)
}

fn build_scheduler_app(config_dir: &Path) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::WorkerServiceProvider)
        .register_schedule(app::schedules::register)
}

#[tokio::test]
async fn sql_observability_endpoint_exposes_typed_stats_contract() {
    let _guard = database_lock().lock().await;
    let app = TestApp::builder()
        .enable_public_observability()
        .build()
        .await
        .unwrap();

    let response = app.client().get("/_foundry/sql").send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: SlowQueriesContract = response.json().unwrap();

    assert_eq!(body.stats.capacity, 100);
    assert_eq!(body.stats.slow_query_threshold_ms, 500);
    assert_eq!(body.stats.retained_count, body.slow_queries.len());
    assert_eq!(
        body.stats.n_plus_one_suspect_count,
        body.n_plus_one_suspects.len()
    );
    assert_eq!(body.top_slowest.len(), body.slow_queries.len());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn observability_enabled_false_skips_foundry_routes() {
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    fs::write(
        config_dir.path().join("00-observability.toml"),
        r#"
            [observability]
            enabled = false
        "#,
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(config_dir.path())
        .enable_public_observability()
        .build()
        .await
        .unwrap();

    let response = app.client().get("/_foundry/health").send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn observability_capture_disabled_keeps_routes_with_empty_counters() {
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    fs::write(
        config_dir.path().join("00-observability.toml"),
        r#"
            [observability]
            capture_enabled = false
        "#,
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(config_dir.path())
        .enable_public_observability()
        .build()
        .await
        .unwrap();

    let response = app.client().get("/_foundry/runtime").send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: RuntimeSnapshot = response.json().unwrap();
    assert_eq!(body.http.requests_total, 0);
    assert_eq!(body.jobs.enqueued_total, 0);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn enable_observability_requires_auth_by_default() {
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    write_http_config(
        config_dir.path(),
        free_port(),
        &unique_namespace("observability-guarded"),
    );

    let app = TestApp::builder()
        .load_config_dir(config_dir.path())
        .register_provider(app::providers::HttpServiceProvider {
            probe: app::providers::ProbeBehavior::Healthy,
        })
        .enable_observability()
        .build()
        .await
        .unwrap();

    let unauthenticated = app.client().get("/_foundry/runtime").send().await.unwrap();
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authenticated = app
        .client()
        .get("/_foundry/runtime")
        .bearer_auth("viewer-token")
        .send()
        .await
        .unwrap();
    assert_eq!(authenticated.status(), reqwest::StatusCode::OK);
    app.shutdown().await.unwrap();
}

async fn wait_for_http_ready(base_url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..40 {
        if client
            .get(format!("{base_url}/_foundry/health"))
            .send()
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("http server did not become ready");
}

async fn connect_websocket(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    for _ in 0..40 {
        if let Ok((socket, _)) = connect_async(url).await {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("websocket server did not become ready");
}

async fn wait_for_scheduler_executions(app: &AppContext, expected: u64) {
    for _ in 0..40 {
        let snapshot = app.diagnostics().unwrap().snapshot();
        if snapshot.scheduler.executed_schedules_total >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("scheduler diagnostics did not reach the expected execution count");
}

#[tokio::test]
async fn observability_endpoints_expose_liveness_readiness_and_runtime_snapshot() {
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    write_http_config(
        config_dir.path(),
        server_port,
        &unique_namespace("observability-http"),
    );

    let server = tokio::spawn({
        let builder = build_http_app(config_dir.path(), app::providers::ProbeBehavior::Healthy);
        async move { builder.run_http_async().await.unwrap() }
    });

    let base_url = format!("http://127.0.0.1:{server_port}");
    wait_for_http_ready(&base_url).await;
    let client = reqwest::Client::new();

    let liveness: LivenessReport = client
        .get(format!("{base_url}/_foundry/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(liveness.state, ProbeState::Healthy);

    let readiness_response = client
        .get(format!("{base_url}/_foundry/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(readiness_response.status(), reqwest::StatusCode::OK);
    let readiness: ReadinessReport = readiness_response.json().await.unwrap();
    assert_eq!(readiness.state, ProbeState::Healthy);
    assert!(readiness
        .probes
        .iter()
        .any(|probe| probe.id == app::ids::ProbeKey::Database.into()));

    let public = client
        .get(format!("{base_url}/public"))
        .header("x-request-id", "observability-request")
        .send()
        .await
        .unwrap();
    assert_eq!(
        public.headers().get("x-request-id").unwrap(),
        "observability-request"
    );

    let unauthorized = client
        .get(format!("{base_url}/secure"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authorized = client
        .get(format!("{base_url}/secure"))
        .header("authorization", "Bearer viewer-token")
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), reqwest::StatusCode::OK);

    let snapshot: RuntimeSnapshot = client
        .get(format!("{base_url}/_foundry/runtime"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot.backend, RuntimeBackendKind::Memory);
    assert!(snapshot.bootstrap_complete);
    assert!(snapshot.http.requests_total >= 5);
    assert!(snapshot.http.success_total >= 3);
    assert!(snapshot.http.client_error_total >= 1);
    assert!(snapshot.http.duration_ms.count >= 5);
    assert!(!snapshot.http.duration_ms.buckets.is_empty());
    assert!(snapshot.auth.success_total >= 1);
    assert!(snapshot.auth.unauthorized_total >= 1);
    assert_eq!(snapshot.jobs.enqueued_total, 0);
    assert_eq!(snapshot.jobs.leased_total, 0);
    assert_eq!(snapshot.jobs.expired_requeues_total, 0);
    assert_eq!(snapshot.scheduler.leadership_acquired_total, 0);
    assert_eq!(snapshot.scheduler.leadership_lost_total, 0);
    assert_eq!(snapshot.websocket.subscriptions_total, 0);
    assert_eq!(snapshot.websocket.unsubscribes_total, 0);

    let http_stats: HttpStatsContract = client
        .get(format!("{base_url}/_foundry/http/stats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(http_stats.stats.requests_total >= snapshot.http.requests_total);
    assert!(http_stats.stats.retained_request_count >= 3);
    assert_eq!(http_stats.stats.retention_capacity, 500);
    assert_eq!(http_stats.stats.slow_request_threshold_ms, 1_000);
    assert!(http_stats.stats.route_count >= 2);
    assert!(http_stats.stats.error_request_count >= 1);
    assert!(!http_stats.top_slowest_routes.is_empty());
    assert!(http_stats
        .top_slowest_routes
        .iter()
        .any(|route| route.path == "/public"));
    assert!(http_stats
        .top_error_routes
        .iter()
        .any(|route| route.path == "/secure" && route.client_error_total >= 1));
    assert!(http_stats
        .recent_error_requests
        .iter()
        .any(|request| request.path == "/secure" && request.status == 401));
    assert_eq!(
        http_stats.recent_slow_requests.len(),
        http_stats.stats.slow_request_count.min(50)
    );

    let metrics = client
        .get(format!("{base_url}/_foundry/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(metrics.status(), reqwest::StatusCode::OK);
    assert_eq!(
        metrics
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    let metrics_body = metrics.text().await.unwrap();
    assert!(metrics_body.contains("# TYPE foundry_http_request_duration_ms histogram"));
    assert!(metrics_body.contains("foundry_http_request_duration_ms_bucket{le=\"5\"}"));
    assert!(metrics_body.contains("foundry_http_request_duration_ms_bucket{le=\"+Inf\"}"));
    assert!(metrics_body.contains("foundry_http_request_duration_ms_sum "));
    assert!(metrics_body.contains("foundry_http_request_duration_ms_count "));
    assert!(metrics_body.contains("foundry_websocket_connection_events_total{state=\"closed\"}"));
    assert!(metrics_body
        .contains("foundry_websocket_subscription_events_total{action=\"unsubscribe\"}"));
    assert!(metrics_body.contains("foundry_scheduler_leadership_total{state=\"lost\"}"));
    assert!(metrics_body.contains("foundry_jobs_total{outcome=\"leased\"}"));
    assert!(metrics_body.contains("foundry_jobs_total{outcome=\"expired_lease_requeued\"}"));

    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn jobs_observability_json_endpoints_have_typed_stable_contracts() {
    let Some(url) = postgres_url() else {
        return;
    };
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    write_database_config(config_dir.path(), &url);

    let app = TestApp::builder()
        .load_config_dir(config_dir.path())
        .enable_public_observability()
        .build()
        .await
        .unwrap();
    let db = app.app().database().unwrap();
    recreate_job_history_table(db.as_ref()).await;

    db.raw_execute(
        r#"
        INSERT INTO job_history
            (id, job_id, queue, status, payload, attempt, error, started_at, completed_at, duration_ms, created_at)
        VALUES
            ('00000000-0000-0000-0000-000000000001', 'job-succeeded', 'default', 'succeeded', NULL, 1, NULL, '2026-04-08T12:00:00Z', '2026-04-08T12:00:01Z', 100, '2026-04-08T12:00:01Z'),
            ('00000000-0000-0000-0000-000000000002', 'job-retried', 'emails', 'retried', '{"trace":{"trace_id":"trace-job-retried","request_id":"req-job-retried"}}'::jsonb, 2, 'retry me', '2026-04-08T12:01:00Z', '2026-04-08T12:01:03Z', 3000, '2026-04-08T12:01:03Z'),
            ('00000000-0000-0000-0000-000000000003', 'job-dead', 'critical', 'dead_lettered', NULL, 3, NULL, NULL, NULL, NULL, '2026-04-08T12:02:00Z')
        "#,
        &[],
    )
    .await
    .unwrap();

    let stats_response = app
        .client()
        .get("/_foundry/jobs/stats")
        .send()
        .await
        .unwrap();
    assert_eq!(stats_response.status(), reqwest::StatusCode::OK);
    let stats: JobsStatsContract = stats_response.json().unwrap();
    assert_eq!(
        stats
            .stats
            .iter()
            .map(|entry| (entry.status.as_str(), entry.count))
            .collect::<Vec<_>>(),
        vec![("dead_lettered", 1), ("retried", 1), ("succeeded", 1)]
    );

    let failed_response = app
        .client()
        .get("/_foundry/jobs/failed")
        .send()
        .await
        .unwrap();
    assert_eq!(failed_response.status(), reqwest::StatusCode::OK);
    let failed: JobsFailedContract = failed_response.json().unwrap();
    assert_eq!(failed.failed_jobs.len(), 2);
    assert_eq!(failed.failed_jobs[0].job_id, "job-dead");
    assert_eq!(failed.failed_jobs[0].queue, "critical");
    assert_eq!(failed.failed_jobs[0].status, "dead_lettered");
    assert_eq!(failed.failed_jobs[0].attempt, Some(3));
    assert_eq!(failed.failed_jobs[0].error, None);
    assert_eq!(failed.failed_jobs[0].started_at, None);
    assert_eq!(failed.failed_jobs[0].completed_at, None);
    assert_eq!(failed.failed_jobs[0].duration_ms, None);
    assert_eq!(failed.failed_jobs[0].request_id, None);
    assert_eq!(failed.failed_jobs[0].trace_id, None);
    DateTime::parse(failed.failed_jobs[0].created_at.as_deref().unwrap()).unwrap();

    assert_eq!(failed.failed_jobs[1].job_id, "job-retried");
    assert_eq!(failed.failed_jobs[1].queue, "emails");
    assert_eq!(failed.failed_jobs[1].status, "retried");
    assert_eq!(failed.failed_jobs[1].attempt, Some(2));
    assert_eq!(failed.failed_jobs[1].error.as_deref(), Some("retry me"));
    assert_eq!(failed.failed_jobs[1].duration_ms, Some(3000));
    assert_eq!(
        failed.failed_jobs[1].request_id.as_deref(),
        Some("req-job-retried")
    );
    assert_eq!(
        failed.failed_jobs[1].trace_id.as_deref(),
        Some("trace-job-retried")
    );
    DateTime::parse(failed.failed_jobs[1].started_at.as_deref().unwrap()).unwrap();
    DateTime::parse(failed.failed_jobs[1].completed_at.as_deref().unwrap()).unwrap();
    DateTime::parse(failed.failed_jobs[1].created_at.as_deref().unwrap()).unwrap();

    let sql_response = app.client().get("/_foundry/sql").send().await.unwrap();
    assert_eq!(sql_response.status(), reqwest::StatusCode::OK);
    let slow_queries: SlowQueriesContract = sql_response.json().unwrap();
    assert_eq!(slow_queries.stats.capacity, 100);
    assert_eq!(slow_queries.stats.slow_query_threshold_ms, 500);
    assert_eq!(
        slow_queries.stats.retained_count,
        slow_queries.slow_queries.len()
    );
    assert_eq!(
        slow_queries.stats.n_plus_one_suspect_count,
        slow_queries.n_plus_one_suspects.len()
    );
    assert_eq!(
        slow_queries.top_slowest.len(),
        slow_queries.slow_queries.len()
    );
    if slow_queries.slow_queries.is_empty() {
        assert_eq!(slow_queries.stats.max_duration_ms, None);
        assert_eq!(slow_queries.stats.avg_duration_ms, None);
        assert_eq!(slow_queries.stats.latest_recorded_at, None);
    }
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn worker_records_job_history_when_diagnostic_capture_is_disabled() {
    let Some(url) = postgres_url() else {
        return;
    };
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    let namespace = unique_namespace("observability-history-capture");
    fs::write(
        config_dir.path().join("00-runtime.toml"),
        format!(
            r#"
            [database]
            url = "{url}"

            [redis]
            namespace = "{namespace}"

            [jobs]
            track_history = true
            max_retries = 1
            poll_interval_ms = 1

            [observability]
            capture_enabled = false
            "#
        ),
    )
    .unwrap();

    let kernel = App::builder()
        .load_config_dir(config_dir.path())
        .register_provider(app::providers::WorkerServiceProvider)
        .build_worker_kernel()
        .await
        .unwrap();
    let app = kernel.app().clone();
    let db = app.database().unwrap();
    recreate_job_history_table(db.as_ref()).await;
    db.raw_execute(
        "ALTER TABLE job_history ALTER COLUMN id SET DEFAULT gen_random_uuid()",
        &[],
    )
    .await
    .unwrap();

    app.jobs()
        .unwrap()
        .dispatch(app::domain::AuditJob {
            marker: "capture-disabled".to_string(),
        })
        .await
        .unwrap();
    assert!(Worker::from_app(app.clone())
        .unwrap()
        .run_once()
        .await
        .unwrap());

    let rows = db
        .raw_query(
            "SELECT job_id, status FROM job_history ORDER BY created_at",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text("job_id"), app::ids::AUDIT_JOB.to_string());
    assert_eq!(rows[0].text("status"), "succeeded");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn worker_prunes_job_history_with_retention_and_distributed_lock() {
    let Some(url) = postgres_url() else {
        return;
    };
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    let namespace = unique_namespace("observability-history-prune");
    fs::write(
        config_dir.path().join("00-runtime.toml"),
        format!(
            r#"
            [database]
            url = "{url}"

            [redis]
            namespace = "{namespace}"

            [jobs]
            history_retention_days = 30
            history_prune_interval_ms = 1
            history_prune_batch_size = 10
        "#
        ),
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(config_dir.path())
        .build()
        .await
        .unwrap();
    let db = app.app().database().unwrap();
    recreate_job_history_table(db.as_ref()).await;
    db.raw_execute(
        r#"
        INSERT INTO job_history (id, job_id, queue, status, attempt, created_at)
        VALUES
            ('00000000-0000-0000-0000-000000000011', 'old-job', 'default', 'succeeded', 1, NOW() - INTERVAL '45 days'),
            ('00000000-0000-0000-0000-000000000012', 'new-job', 'default', 'succeeded', 1, NOW())
        "#,
        &[],
    )
    .await
    .unwrap();

    let lock = app
        .app()
        .lock()
        .unwrap()
        .acquire("jobs:history_prune", Duration::from_secs(60))
        .await
        .unwrap()
        .expect("expected prune lock");
    Worker::from_app(app.app().clone())
        .unwrap()
        .run_once()
        .await
        .unwrap();
    let rows = db
        .raw_query("SELECT job_id FROM job_history ORDER BY job_id", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    lock.release().await.unwrap();

    Worker::from_app(app.app().clone())
        .unwrap()
        .run_once()
        .await
        .unwrap();
    let rows = db
        .raw_query("SELECT job_id FROM job_history ORDER BY job_id", &[])
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.text("job_id"))
            .collect::<Vec<_>>(),
        vec!["new-job".to_string()]
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn worker_keeps_job_history_forever_when_retention_is_zero() {
    let Some(url) = postgres_url() else {
        return;
    };
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    let namespace = unique_namespace("observability-history-retain");
    fs::write(
        config_dir.path().join("00-runtime.toml"),
        format!(
            r#"
            [database]
            url = "{url}"

            [redis]
            namespace = "{namespace}"

            [jobs]
            history_retention_days = 0
            history_prune_interval_ms = 1
            history_prune_batch_size = 10
        "#
        ),
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(config_dir.path())
        .build()
        .await
        .unwrap();
    let db = app.app().database().unwrap();
    recreate_job_history_table(db.as_ref()).await;
    db.raw_execute(
        r#"
        INSERT INTO job_history (id, job_id, queue, status, attempt, created_at)
        VALUES
            ('00000000-0000-0000-0000-000000000021', 'old-job', 'default', 'succeeded', 1, NOW() - INTERVAL '45 days'),
            ('00000000-0000-0000-0000-000000000022', 'new-job', 'default', 'succeeded', 1, NOW())
        "#,
        &[],
    )
    .await
    .unwrap();

    Worker::from_app(app.app().clone())
        .unwrap()
        .run_once()
        .await
        .unwrap();

    let rows = db
        .raw_query("SELECT job_id FROM job_history ORDER BY job_id", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn readiness_endpoint_returns_503_when_provider_probe_fails() {
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    write_http_config(
        config_dir.path(),
        server_port,
        &unique_namespace("observability-ready"),
    );

    let server = tokio::spawn({
        let builder = build_http_app(config_dir.path(), app::providers::ProbeBehavior::Unhealthy);
        async move { builder.run_http_async().await.unwrap() }
    });

    let base_url = format!("http://127.0.0.1:{server_port}");
    wait_for_http_ready(&base_url).await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base_url}/_foundry/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let readiness: ReadinessReport = response.json().await.unwrap();
    assert_eq!(readiness.state, ProbeState::Unhealthy);
    let database_probe = readiness
        .probes
        .into_iter()
        .find(|probe| probe.id == app::ids::ProbeKey::Database.into())
        .unwrap();
    assert_eq!(database_probe.state, ProbeState::Unhealthy);

    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn readiness_endpoint_returns_503_when_provider_probe_panics() {
    let _guard = database_lock().lock().await;
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    write_http_config(
        config_dir.path(),
        server_port,
        &unique_namespace("observability-ready-panic"),
    );

    let server = tokio::spawn({
        let builder = build_http_app(config_dir.path(), app::providers::ProbeBehavior::Panic);
        async move { builder.run_http_async().await.unwrap() }
    });

    let base_url = format!("http://127.0.0.1:{server_port}");
    wait_for_http_ready(&base_url).await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base_url}/_foundry/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let readiness: ReadinessReport = response.json().await.unwrap();
    assert_eq!(readiness.state, ProbeState::Unhealthy);
    let database_probe = readiness
        .probes
        .into_iter()
        .find(|probe| probe.id == app::ids::ProbeKey::Database.into())
        .unwrap();
    assert_eq!(database_probe.state, ProbeState::Unhealthy);
    assert_eq!(
        database_probe.message.as_deref(),
        Some("readiness check panicked: database probe exploded")
    );

    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn diagnostics_track_websocket_job_and_scheduler_activity() {
    let _guard = database_lock().lock().await;
    let websocket_dir = tempdir().unwrap();
    let websocket_port = free_port();
    write_websocket_config(
        websocket_dir.path(),
        websocket_port,
        &unique_namespace("observability-ws"),
    );

    let websocket_kernel = build_websocket_app(websocket_dir.path())
        .build_websocket_kernel()
        .await
        .unwrap();
    let websocket_app = websocket_kernel.app().clone();
    let websocket_server = tokio::spawn(async move { websocket_kernel.serve().await.unwrap() });

    let mut socket = connect_websocket(&format!("ws://127.0.0.1:{websocket_port}/ws")).await;
    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::CHAT_CHANNEL,
                room: None,
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let _ = socket.next().await.unwrap().unwrap();

    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Message,
                channel: app::ids::CHAT_CHANNEL,
                room: None,
                payload: Some(serde_json::json!({ "body": "hello" })),
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let _ = socket.next().await.unwrap().unwrap();

    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Unsubscribe,
                channel: app::ids::CHAT_CHANNEL,
                room: None,
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let _ = socket.next().await.unwrap().unwrap();
    socket.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let websocket_snapshot = websocket_app.diagnostics().unwrap().snapshot();
    assert!(websocket_snapshot.websocket.opened_total >= 1);
    assert!(websocket_snapshot.websocket.closed_total >= 1);
    assert!(websocket_snapshot.websocket.subscriptions_total >= 1);
    assert!(websocket_snapshot.websocket.unsubscribes_total >= 1);
    assert!(websocket_snapshot.websocket.inbound_messages_total >= 3);
    assert!(websocket_snapshot.websocket.outbound_messages_total >= 2);

    websocket_server.abort();
    let _ = websocket_server.await;

    let scheduler_dir = tempdir().unwrap();
    write_scheduler_config(
        scheduler_dir.path(),
        &unique_namespace("observability-jobs"),
    );
    let scheduler = build_scheduler_app(scheduler_dir.path())
        .build_scheduler_kernel()
        .await
        .unwrap();
    let scheduler_app = scheduler.app().clone();

    scheduler_app
        .jobs()
        .unwrap()
        .dispatch(app::domain::AuditJob {
            marker: "manual".to_string(),
        })
        .await
        .unwrap();
    assert!(Worker::from_app(scheduler_app.clone())
        .unwrap()
        .run_once()
        .await
        .unwrap());

    let now = DateTime::parse("2026-04-08T12:00:00Z").unwrap();
    let executed = scheduler.tick_at(now).await.unwrap();
    assert_eq!(executed, vec![app::ids::HEARTBEAT_SCHEDULE]);
    wait_for_scheduler_executions(&scheduler_app, 1).await;

    let snapshot = scheduler_app.diagnostics().unwrap().snapshot();
    assert_eq!(snapshot.jobs.enqueued_total, 1);
    assert_eq!(snapshot.jobs.leased_total, 1);
    assert_eq!(snapshot.jobs.started_total, 1);
    assert_eq!(snapshot.jobs.succeeded_total, 1);
    assert_eq!(snapshot.jobs.retried_total, 0);
    assert_eq!(snapshot.jobs.expired_requeues_total, 0);
    assert_eq!(snapshot.jobs.dead_lettered_total, 0);
    assert_eq!(snapshot.scheduler.ticks_total, 1);
    assert_eq!(snapshot.scheduler.executed_schedules_total, 1);
    scheduler_app.shutdown().await.unwrap();
}
