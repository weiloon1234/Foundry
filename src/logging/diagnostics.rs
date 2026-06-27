use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use chrono::Utc as ChronoUtc;
use serde::{Deserialize, Serialize};

use super::probes::{LivenessReport, ProbeResult, ReadinessRegistry, ReadinessReport};
use super::types::{
    AuthOutcome, HttpOutcomeClass, JobOutcome, ProbeState, RuntimeBackendKind,
    SchedulerLeadershipState, WebSocketConnectionState,
};
use super::{catch_async_panic, panic_payload_message};
use crate::foundation::{AppContext, Result};
use crate::http::middleware::HttpEdgeRejection;
use crate::support::sync::{lock_unpoisoned, read_unpoisoned, write_unpoisoned};
use crate::support::ChannelId;

const HTTP_REQUEST_DURATION_BUCKETS_MS: [u64; 12] = [
    5, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 30_000,
];
const HTTP_REQUEST_OBSERVATION_CAPACITY: usize = 500;
const HTTP_ROUTE_RANKING_LIMIT: usize = 20;
const HTTP_RECENT_REQUEST_LIMIT: usize = 50;
const HTTP_SLOW_REQUEST_THRESHOLD_MS: u64 = 1_000;
const WEBSOCKET_CHANNEL_OBSERVATION_CAPACITY: usize = 500;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeDiagnosticsConfig {
    pub capture_enabled: bool,
    pub http_sample_retention: usize,
    pub websocket_channel_retention: usize,
}

impl Default for RuntimeDiagnosticsConfig {
    fn default() -> Self {
        Self {
            capture_enabled: true,
            http_sample_retention: HTTP_REQUEST_OBSERVATION_CAPACITY,
            websocket_channel_retention: WEBSOCKET_CHANNEL_OBSERVATION_CAPACITY,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct RuntimeSnapshot {
    pub backend: RuntimeBackendKind,
    pub bootstrap_complete: bool,
    pub http: HttpRuntimeSnapshot,
    pub auth: AuthRuntimeSnapshot,
    pub websocket: WebSocketRuntimeSnapshot,
    pub scheduler: SchedulerRuntimeSnapshot,
    pub jobs: JobRuntimeSnapshot,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpRuntimeSnapshot {
    pub requests_total: u64,
    pub informational_total: u64,
    pub success_total: u64,
    pub redirection_total: u64,
    pub client_error_total: u64,
    pub server_error_total: u64,
    pub edge_rejections: HttpEdgeRejectionSnapshot,
    pub duration_ms: HttpDurationHistogramSnapshot,
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpEdgeRejectionSnapshot {
    pub rate_limited_total: u64,
    pub payload_too_large_total: u64,
    pub timeout_total: u64,
    pub cors_rejected_total: u64,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpDurationHistogramSnapshot {
    pub count: u64,
    pub sum_ms: u64,
    pub buckets: Vec<HttpDurationBucketSnapshot>,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpDurationBucketSnapshot {
    pub le_ms: u64,
    pub cumulative_count: u64,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpObservabilitySnapshot {
    pub stats: HttpObservabilityStats,
    pub top_slowest_routes: Vec<HttpRouteRankingSnapshot>,
    pub top_error_routes: Vec<HttpRouteRankingSnapshot>,
    pub recent_slow_requests: Vec<HttpRequestSampleSnapshot>,
    pub recent_error_requests: Vec<HttpRequestSampleSnapshot>,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpObservabilityStats {
    pub requests_total: u64,
    pub retained_request_count: usize,
    pub retention_capacity: usize,
    pub slow_request_threshold_ms: u64,
    pub route_count: usize,
    pub slow_request_count: usize,
    pub error_request_count: usize,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpRouteRankingSnapshot {
    pub method: String,
    pub path: String,
    pub requests_total: u64,
    pub informational_total: u64,
    pub success_total: u64,
    pub redirection_total: u64,
    pub client_error_total: u64,
    pub server_error_total: u64,
    pub avg_duration_ms: u64,
    pub max_duration_ms: u64,
    pub p95_duration_ms: u64,
    pub p99_duration_ms: u64,
    pub latest_recorded_at: String,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct HttpRequestSampleSnapshot {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct HttpRequestRecord {
    pub method: String,
    pub path: String,
    pub status: axum::http::StatusCode,
    pub duration_ms: u64,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct AuthRuntimeSnapshot {
    pub success_total: u64,
    pub unauthorized_total: u64,
    pub forbidden_total: u64,
    pub error_total: u64,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct WebSocketRuntimeSnapshot {
    pub opened_total: u64,
    pub closed_total: u64,
    pub active_connections: u64,
    pub subscriptions_total: u64,
    pub unsubscribes_total: u64,
    pub active_subscriptions: u64,
    pub inbound_messages_total: u64,
    pub outbound_messages_total: u64,
    pub channels: Vec<WebSocketChannelSnapshot>,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct WebSocketChannelSnapshot {
    pub id: ChannelId,
    pub subscriptions_total: u64,
    pub unsubscribes_total: u64,
    pub active_subscriptions: u64,
    pub inbound_messages_total: u64,
    pub outbound_messages_total: u64,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct SchedulerRuntimeSnapshot {
    pub ticks_total: u64,
    pub executed_schedules_total: u64,
    pub leadership_acquired_total: u64,
    pub leadership_lost_total: u64,
    pub leader_active: bool,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct JobRuntimeSnapshot {
    pub enqueued_total: u64,
    pub leased_total: u64,
    pub started_total: u64,
    pub succeeded_total: u64,
    pub retried_total: u64,
    pub expired_requeues_total: u64,
    pub dead_lettered_total: u64,
}

struct HttpDurationHistogram {
    count: AtomicU64,
    sum_ms: AtomicU64,
    buckets: [AtomicU64; HTTP_REQUEST_DURATION_BUCKETS_MS.len()],
}

impl Default for HttpDurationHistogram {
    fn default() -> Self {
        Self {
            count: AtomicU64::new(0),
            sum_ms: AtomicU64::new(0),
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl HttpDurationHistogram {
    fn record(&self, duration_ms: u64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_ms.fetch_add(duration_ms, Ordering::Relaxed);

        if let Some(index) = HTTP_REQUEST_DURATION_BUCKETS_MS
            .iter()
            .position(|upper_bound_ms| duration_ms <= *upper_bound_ms)
        {
            self.buckets[index].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> HttpDurationHistogramSnapshot {
        let mut cumulative_count = 0;
        let buckets = HTTP_REQUEST_DURATION_BUCKETS_MS
            .iter()
            .enumerate()
            .map(|(index, le_ms)| {
                cumulative_count += self.buckets[index].load(Ordering::Relaxed);
                HttpDurationBucketSnapshot {
                    le_ms: *le_ms,
                    cumulative_count,
                }
            })
            .collect();

        HttpDurationHistogramSnapshot {
            count: self.count.load(Ordering::Relaxed),
            sum_ms: self.sum_ms.load(Ordering::Relaxed),
            buckets,
        }
    }
}

struct HttpCounters {
    requests_total: AtomicU64,
    informational_total: AtomicU64,
    success_total: AtomicU64,
    redirection_total: AtomicU64,
    client_error_total: AtomicU64,
    server_error_total: AtomicU64,
    rate_limited_total: AtomicU64,
    payload_too_large_total: AtomicU64,
    timeout_total: AtomicU64,
    cors_rejected_total: AtomicU64,
    duration_ms: HttpDurationHistogram,
    requests: Mutex<VecDeque<HttpRequestObservation>>,
    sample_retention: usize,
}

impl HttpCounters {
    fn new(sample_retention: usize) -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            informational_total: AtomicU64::new(0),
            success_total: AtomicU64::new(0),
            redirection_total: AtomicU64::new(0),
            client_error_total: AtomicU64::new(0),
            server_error_total: AtomicU64::new(0),
            rate_limited_total: AtomicU64::new(0),
            payload_too_large_total: AtomicU64::new(0),
            timeout_total: AtomicU64::new(0),
            cors_rejected_total: AtomicU64::new(0),
            duration_ms: HttpDurationHistogram::default(),
            requests: Mutex::new(VecDeque::with_capacity(sample_retention)),
            sample_retention,
        }
    }
}

impl HttpCounters {
    fn snapshot(&self) -> HttpRuntimeSnapshot {
        HttpRuntimeSnapshot {
            requests_total: self.requests_total.load(Ordering::Relaxed),
            informational_total: self.informational_total.load(Ordering::Relaxed),
            success_total: self.success_total.load(Ordering::Relaxed),
            redirection_total: self.redirection_total.load(Ordering::Relaxed),
            client_error_total: self.client_error_total.load(Ordering::Relaxed),
            server_error_total: self.server_error_total.load(Ordering::Relaxed),
            edge_rejections: HttpEdgeRejectionSnapshot {
                rate_limited_total: self.rate_limited_total.load(Ordering::Relaxed),
                payload_too_large_total: self.payload_too_large_total.load(Ordering::Relaxed),
                timeout_total: self.timeout_total.load(Ordering::Relaxed),
                cors_rejected_total: self.cors_rejected_total.load(Ordering::Relaxed),
            },
            duration_ms: self.duration_ms.snapshot(),
        }
    }

    fn record_edge_rejection(&self, rejection: HttpEdgeRejection) {
        match rejection {
            HttpEdgeRejection::RateLimited => {
                self.rate_limited_total.fetch_add(1, Ordering::Relaxed);
            }
            HttpEdgeRejection::PayloadTooLarge => {
                self.payload_too_large_total.fetch_add(1, Ordering::Relaxed);
            }
            HttpEdgeRejection::Timeout => {
                self.timeout_total.fetch_add(1, Ordering::Relaxed);
            }
            HttpEdgeRejection::Cors => {
                self.cors_rejected_total.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn record_request(&self, request: HttpRequestRecord) {
        if self.sample_retention == 0 {
            return;
        }

        let recorded_at = ChronoUtc::now().to_rfc3339();
        let mut requests = lock_unpoisoned(&self.requests, "http request observations");
        if requests.len() >= self.sample_retention {
            requests.pop_front();
        }
        requests.push_back(HttpRequestObservation {
            method: request.method,
            path: request.path,
            status: request.status.as_u16(),
            duration_ms: request.duration_ms,
            request_id: request.request_id,
            trace_id: request.trace_id,
            recorded_at,
        });
    }

    fn observability_snapshot(&self) -> HttpObservabilitySnapshot {
        let requests: Vec<HttpRequestObservation> =
            lock_unpoisoned(&self.requests, "http request observations")
                .iter()
                .cloned()
                .collect();
        let route_rankings = http_route_rankings(&requests);
        let route_count = route_rankings.len();
        let slow_request_count = requests
            .iter()
            .filter(|request| request.duration_ms >= HTTP_SLOW_REQUEST_THRESHOLD_MS)
            .count();
        let error_request_count = requests
            .iter()
            .filter(|request| request.status >= 400)
            .count();

        let mut top_slowest_routes = route_rankings.clone();
        top_slowest_routes.sort_by(|left, right| {
            right
                .max_duration_ms
                .cmp(&left.max_duration_ms)
                .then_with(|| right.avg_duration_ms.cmp(&left.avg_duration_ms))
                .then_with(|| right.requests_total.cmp(&left.requests_total))
                .then_with(|| left.method.cmp(&right.method))
                .then_with(|| left.path.cmp(&right.path))
        });
        top_slowest_routes.truncate(HTTP_ROUTE_RANKING_LIMIT);

        let mut top_error_routes: Vec<HttpRouteRankingSnapshot> = route_rankings
            .into_iter()
            .filter(|route| route.client_error_total + route.server_error_total > 0)
            .collect();
        top_error_routes.sort_by(|left, right| {
            (right.client_error_total + right.server_error_total)
                .cmp(&(left.client_error_total + left.server_error_total))
                .then_with(|| right.server_error_total.cmp(&left.server_error_total))
                .then_with(|| right.max_duration_ms.cmp(&left.max_duration_ms))
                .then_with(|| left.method.cmp(&right.method))
                .then_with(|| left.path.cmp(&right.path))
        });
        top_error_routes.truncate(HTTP_ROUTE_RANKING_LIMIT);

        HttpObservabilitySnapshot {
            stats: HttpObservabilityStats {
                requests_total: self.requests_total.load(Ordering::Relaxed),
                retained_request_count: requests.len(),
                retention_capacity: self.sample_retention,
                slow_request_threshold_ms: HTTP_SLOW_REQUEST_THRESHOLD_MS,
                route_count,
                slow_request_count,
                error_request_count,
            },
            top_slowest_routes,
            top_error_routes,
            recent_slow_requests: recent_requests(&requests, |request| {
                request.duration_ms >= HTTP_SLOW_REQUEST_THRESHOLD_MS
            }),
            recent_error_requests: recent_requests(&requests, |request| request.status >= 400),
        }
    }
}

#[derive(Clone, Debug)]
struct HttpRequestObservation {
    method: String,
    path: String,
    status: u16,
    duration_ms: u64,
    request_id: Option<String>,
    trace_id: Option<String>,
    recorded_at: String,
}

#[derive(Default)]
struct HttpRouteAggregate {
    method: String,
    path: String,
    informational_total: u64,
    success_total: u64,
    redirection_total: u64,
    client_error_total: u64,
    server_error_total: u64,
    duration_ms: Vec<u64>,
    latest_recorded_at: String,
}

impl HttpRouteAggregate {
    fn new(request: &HttpRequestObservation) -> Self {
        Self {
            method: request.method.clone(),
            path: request.path.clone(),
            latest_recorded_at: request.recorded_at.clone(),
            ..Self::default()
        }
    }

    fn record(&mut self, request: &HttpRequestObservation) {
        match status_class(request.status) {
            HttpOutcomeClass::Informational => self.informational_total += 1,
            HttpOutcomeClass::Success => self.success_total += 1,
            HttpOutcomeClass::Redirection => self.redirection_total += 1,
            HttpOutcomeClass::ClientError => self.client_error_total += 1,
            HttpOutcomeClass::ServerError => self.server_error_total += 1,
        }
        self.duration_ms.push(request.duration_ms);
        if request.recorded_at > self.latest_recorded_at {
            self.latest_recorded_at.clone_from(&request.recorded_at);
        }
    }

    fn into_snapshot(mut self) -> HttpRouteRankingSnapshot {
        self.duration_ms.sort_unstable();
        let requests_total = self.duration_ms.len() as u64;
        let sum_ms = self.duration_ms.iter().sum::<u64>();
        let avg_duration_ms = sum_ms.checked_div(requests_total).unwrap_or(0);
        let max_duration_ms = self.duration_ms.last().copied().unwrap_or(0);
        let p95_duration_ms = percentile(&self.duration_ms, 95).unwrap_or(0);
        let p99_duration_ms = percentile(&self.duration_ms, 99).unwrap_or(0);

        HttpRouteRankingSnapshot {
            method: self.method,
            path: self.path,
            requests_total,
            informational_total: self.informational_total,
            success_total: self.success_total,
            redirection_total: self.redirection_total,
            client_error_total: self.client_error_total,
            server_error_total: self.server_error_total,
            avg_duration_ms,
            max_duration_ms,
            p95_duration_ms,
            p99_duration_ms,
            latest_recorded_at: self.latest_recorded_at,
        }
    }
}

fn http_route_rankings(requests: &[HttpRequestObservation]) -> Vec<HttpRouteRankingSnapshot> {
    let mut routes: HashMap<(String, String), HttpRouteAggregate> = HashMap::new();
    for request in requests {
        routes
            .entry((request.method.clone(), request.path.clone()))
            .or_insert_with(|| HttpRouteAggregate::new(request))
            .record(request);
    }
    routes
        .into_values()
        .map(HttpRouteAggregate::into_snapshot)
        .collect()
}

fn recent_requests<F>(
    requests: &[HttpRequestObservation],
    mut matches: F,
) -> Vec<HttpRequestSampleSnapshot>
where
    F: FnMut(&HttpRequestObservation) -> bool,
{
    requests
        .iter()
        .rev()
        .filter(|request| matches(request))
        .take(HTTP_RECENT_REQUEST_LIMIT)
        .map(|request| HttpRequestSampleSnapshot {
            method: request.method.clone(),
            path: request.path.clone(),
            status: request.status,
            duration_ms: request.duration_ms,
            request_id: request.request_id.clone(),
            trace_id: request.trace_id.clone(),
            recorded_at: request.recorded_at.clone(),
        })
        .collect()
}

fn percentile(sorted_values: &[u64], percentile: usize) -> Option<u64> {
    if sorted_values.is_empty() {
        return None;
    }
    let rank = sorted_values
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1);
    sorted_values.get(rank).copied()
}

fn status_class(status: u16) -> HttpOutcomeClass {
    axum::http::StatusCode::from_u16(status)
        .map(HttpOutcomeClass::from_status)
        .unwrap_or(HttpOutcomeClass::ServerError)
}

#[derive(Default)]
struct AuthCounters {
    success_total: AtomicU64,
    unauthorized_total: AtomicU64,
    forbidden_total: AtomicU64,
    error_total: AtomicU64,
}

impl AuthCounters {
    fn snapshot(&self) -> AuthRuntimeSnapshot {
        AuthRuntimeSnapshot {
            success_total: self.success_total.load(Ordering::Relaxed),
            unauthorized_total: self.unauthorized_total.load(Ordering::Relaxed),
            forbidden_total: self.forbidden_total.load(Ordering::Relaxed),
            error_total: self.error_total.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct PerChannelWebSocketCounters {
    subscriptions_total: AtomicU64,
    unsubscribes_total: AtomicU64,
    active_subscriptions: AtomicU64,
    inbound_messages_total: AtomicU64,
    outbound_messages_total: AtomicU64,
}

struct WebSocketCounters {
    opened_total: AtomicU64,
    closed_total: AtomicU64,
    active_connections: AtomicU64,
    subscriptions_total: AtomicU64,
    unsubscribes_total: AtomicU64,
    active_subscriptions: AtomicU64,
    inbound_messages_total: AtomicU64,
    outbound_messages_total: AtomicU64,
    per_channel: RwLock<HashMap<ChannelId, Arc<PerChannelWebSocketCounters>>>,
    channel_retention: usize,
}

impl WebSocketCounters {
    fn new(channel_retention: usize) -> Self {
        Self {
            opened_total: AtomicU64::new(0),
            closed_total: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            subscriptions_total: AtomicU64::new(0),
            unsubscribes_total: AtomicU64::new(0),
            active_subscriptions: AtomicU64::new(0),
            inbound_messages_total: AtomicU64::new(0),
            outbound_messages_total: AtomicU64::new(0),
            per_channel: RwLock::new(HashMap::new()),
            channel_retention,
        }
    }
}

impl WebSocketCounters {
    fn snapshot(&self) -> WebSocketRuntimeSnapshot {
        let map = read_unpoisoned(&self.per_channel, "websocket per-channel counters");
        let mut channels: Vec<WebSocketChannelSnapshot> = map
            .iter()
            .map(|(id, counters)| WebSocketChannelSnapshot {
                id: id.clone(),
                subscriptions_total: counters.subscriptions_total.load(Ordering::Relaxed),
                unsubscribes_total: counters.unsubscribes_total.load(Ordering::Relaxed),
                active_subscriptions: counters.active_subscriptions.load(Ordering::Relaxed),
                inbound_messages_total: counters.inbound_messages_total.load(Ordering::Relaxed),
                outbound_messages_total: counters.outbound_messages_total.load(Ordering::Relaxed),
            })
            .collect();
        drop(map);
        channels.sort_unstable_by(|a, b| a.id.cmp(&b.id));

        WebSocketRuntimeSnapshot {
            opened_total: self.opened_total.load(Ordering::Relaxed),
            closed_total: self.closed_total.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            subscriptions_total: self.subscriptions_total.load(Ordering::Relaxed),
            unsubscribes_total: self.unsubscribes_total.load(Ordering::Relaxed),
            active_subscriptions: self.active_subscriptions.load(Ordering::Relaxed),
            inbound_messages_total: self.inbound_messages_total.load(Ordering::Relaxed),
            outbound_messages_total: self.outbound_messages_total.load(Ordering::Relaxed),
            channels,
        }
    }

    fn entry(&self, channel: &ChannelId) -> Arc<PerChannelWebSocketCounters> {
        if self.channel_retention == 0 {
            return Arc::new(PerChannelWebSocketCounters::default());
        }

        // Fast path: read lock and return if present.
        {
            let map = read_unpoisoned(&self.per_channel, "websocket per-channel counters");
            if let Some(existing) = map.get(channel) {
                return existing.clone();
            }
        }
        // Slow path: upgrade to write lock and insert.
        let mut map = write_unpoisoned(&self.per_channel, "websocket per-channel counters");
        prune_idle_websocket_channels(&mut map, self.channel_retention);
        map.entry(channel.clone())
            .or_insert_with(|| Arc::new(PerChannelWebSocketCounters::default()))
            .clone()
    }
}

fn prune_idle_websocket_channels(
    map: &mut HashMap<ChannelId, Arc<PerChannelWebSocketCounters>>,
    retention: usize,
) {
    while map.len() >= retention {
        let Some(channel) = map
            .iter()
            .filter(|(_, counters)| counters.active_subscriptions.load(Ordering::Relaxed) == 0)
            .map(|(channel, _)| channel.clone())
            .min()
        else {
            break;
        };
        map.remove(&channel);
    }
}

#[derive(Default)]
struct SchedulerCounters {
    ticks_total: AtomicU64,
    executed_schedules_total: AtomicU64,
    leadership_acquired_total: AtomicU64,
    leadership_lost_total: AtomicU64,
    leader_active: AtomicBool,
}

impl SchedulerCounters {
    fn snapshot(&self) -> SchedulerRuntimeSnapshot {
        SchedulerRuntimeSnapshot {
            ticks_total: self.ticks_total.load(Ordering::Relaxed),
            executed_schedules_total: self.executed_schedules_total.load(Ordering::Relaxed),
            leadership_acquired_total: self.leadership_acquired_total.load(Ordering::Relaxed),
            leadership_lost_total: self.leadership_lost_total.load(Ordering::Relaxed),
            leader_active: self.leader_active.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct JobCounters {
    enqueued_total: AtomicU64,
    leased_total: AtomicU64,
    started_total: AtomicU64,
    succeeded_total: AtomicU64,
    retried_total: AtomicU64,
    expired_requeues_total: AtomicU64,
    dead_lettered_total: AtomicU64,
}

impl JobCounters {
    fn snapshot(&self) -> JobRuntimeSnapshot {
        JobRuntimeSnapshot {
            enqueued_total: self.enqueued_total.load(Ordering::Relaxed),
            leased_total: self.leased_total.load(Ordering::Relaxed),
            started_total: self.started_total.load(Ordering::Relaxed),
            succeeded_total: self.succeeded_total.load(Ordering::Relaxed),
            retried_total: self.retried_total.load(Ordering::Relaxed),
            expired_requeues_total: self.expired_requeues_total.load(Ordering::Relaxed),
            dead_lettered_total: self.dead_lettered_total.load(Ordering::Relaxed),
        }
    }
}

pub struct RuntimeDiagnostics {
    backend: RuntimeBackendKind,
    bootstrap_complete: AtomicBool,
    capture_enabled: bool,
    readiness: ReadinessRegistry,
    http: HttpCounters,
    auth: AuthCounters,
    websocket: WebSocketCounters,
    scheduler: SchedulerCounters,
    jobs: JobCounters,
}

impl RuntimeDiagnostics {
    #[cfg(test)]
    pub(crate) fn new(backend: RuntimeBackendKind, readiness: ReadinessRegistry) -> Self {
        Self::new_with_config(backend, readiness, RuntimeDiagnosticsConfig::default())
    }

    pub(crate) fn new_with_config(
        backend: RuntimeBackendKind,
        readiness: ReadinessRegistry,
        config: RuntimeDiagnosticsConfig,
    ) -> Self {
        Self {
            backend,
            bootstrap_complete: AtomicBool::new(false),
            capture_enabled: config.capture_enabled,
            readiness,
            http: HttpCounters::new(config.http_sample_retention),
            auth: AuthCounters::default(),
            websocket: WebSocketCounters::new(config.websocket_channel_retention),
            scheduler: SchedulerCounters::default(),
            jobs: JobCounters::default(),
        }
    }

    pub fn backend_kind(&self) -> RuntimeBackendKind {
        self.backend
    }

    pub(crate) fn capture_enabled(&self) -> bool {
        self.capture_enabled
    }

    pub fn mark_bootstrap_complete(&self) {
        self.bootstrap_complete.store(true, Ordering::Relaxed);
    }

    pub fn bootstrap_complete(&self) -> bool {
        self.bootstrap_complete.load(Ordering::Relaxed)
    }

    pub fn liveness(&self) -> LivenessReport {
        LivenessReport {
            state: ProbeState::Healthy,
        }
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            backend: self.backend,
            bootstrap_complete: self.bootstrap_complete(),
            http: self.http.snapshot(),
            auth: self.auth.snapshot(),
            websocket: self.websocket.snapshot(),
            scheduler: self.scheduler.snapshot(),
            jobs: self.jobs.snapshot(),
        }
    }

    pub(crate) fn http_observability_snapshot(&self) -> HttpObservabilitySnapshot {
        self.http.observability_snapshot()
    }

    pub async fn run_readiness_checks(&self, app: &AppContext) -> Result<ReadinessReport> {
        let mut probes = Vec::with_capacity(self.readiness.checks.len());
        let mut state = ProbeState::Healthy;

        for registered in &self.readiness.checks {
            let probe = match catch_async_panic(|| registered.check.run(app)).await {
                Ok(Ok(mut probe)) => {
                    probe.id = registered.id.clone();
                    probe
                }
                Ok(Err(error)) => ProbeResult::unhealthy(registered.id.clone(), error.to_string()),
                Err(panic) => {
                    let message = panic_payload_message(panic);
                    tracing::error!(
                        target: "foundry.readiness",
                        probe = %registered.id,
                        panic = %message,
                        "readiness check panicked"
                    );
                    ProbeResult::unhealthy(
                        registered.id.clone(),
                        format!("readiness check panicked: {message}"),
                    )
                }
            };

            if !probe.state.is_healthy() {
                state = ProbeState::Unhealthy;
            }
            probes.push(probe);
        }

        Ok(ReadinessReport { state, probes })
    }

    pub fn record_http_response(&self, status: axum::http::StatusCode) {
        self.record_http_response_inner(status, None);
    }

    pub fn record_http_response_with_duration(
        &self,
        status: axum::http::StatusCode,
        duration_ms: u64,
    ) {
        self.record_http_response_inner(status, Some(duration_ms));
    }

    pub(crate) fn record_http_request(&self, request: HttpRequestRecord) {
        if !self.capture_enabled {
            return;
        }
        let status = request.status;
        let duration_ms = request.duration_ms;
        self.record_http_response_inner(status, Some(duration_ms));
        self.http.record_request(request);
    }

    pub(crate) fn record_http_edge_rejection(&self, rejection: HttpEdgeRejection) {
        if !self.capture_enabled {
            return;
        }
        self.http.record_edge_rejection(rejection);
    }

    fn record_http_response_inner(&self, status: axum::http::StatusCode, duration_ms: Option<u64>) {
        if !self.capture_enabled {
            return;
        }
        self.http.requests_total.fetch_add(1, Ordering::Relaxed);
        if let Some(duration_ms) = duration_ms {
            self.http.duration_ms.record(duration_ms);
        }
        match HttpOutcomeClass::from_status(status) {
            HttpOutcomeClass::Informational => {
                self.http
                    .informational_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            HttpOutcomeClass::Success => {
                self.http.success_total.fetch_add(1, Ordering::Relaxed);
            }
            HttpOutcomeClass::Redirection => {
                self.http.redirection_total.fetch_add(1, Ordering::Relaxed);
            }
            HttpOutcomeClass::ClientError => {
                self.http.client_error_total.fetch_add(1, Ordering::Relaxed);
            }
            HttpOutcomeClass::ServerError => {
                self.http.server_error_total.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn record_auth_outcome(&self, outcome: AuthOutcome) {
        if !self.capture_enabled {
            return;
        }
        match outcome {
            AuthOutcome::Success => {
                self.auth.success_total.fetch_add(1, Ordering::Relaxed);
            }
            AuthOutcome::Unauthorized => {
                self.auth.unauthorized_total.fetch_add(1, Ordering::Relaxed);
            }
            AuthOutcome::Forbidden => {
                self.auth.forbidden_total.fetch_add(1, Ordering::Relaxed);
            }
            AuthOutcome::Error => {
                self.auth.error_total.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn record_websocket_connection(&self, state: WebSocketConnectionState) {
        if !self.capture_enabled {
            return;
        }
        match state {
            WebSocketConnectionState::Opened => {
                self.websocket.opened_total.fetch_add(1, Ordering::Relaxed);
                self.websocket
                    .active_connections
                    .fetch_add(1, Ordering::Relaxed);
            }
            WebSocketConnectionState::Closed => {
                self.websocket.closed_total.fetch_add(1, Ordering::Relaxed);
                decrement_saturating(&self.websocket.active_connections);
            }
        }
    }

    #[deprecated(
        since = "0.2.0",
        note = "use `record_websocket_subscription_opened_on(&channel)` — the global-only variant bypasses per-channel tracking"
    )]
    pub fn record_websocket_subscription_opened(&self) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .subscriptions_total
            .fetch_add(1, Ordering::Relaxed);
        self.websocket
            .active_subscriptions
            .fetch_add(1, Ordering::Relaxed);
    }

    #[deprecated(
        since = "0.2.0",
        note = "use `record_websocket_subscription_closed_on(&channel)` — the global-only variant bypasses per-channel tracking"
    )]
    pub fn record_websocket_subscription_closed(&self) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .unsubscribes_total
            .fetch_add(1, Ordering::Relaxed);
        decrement_saturating(&self.websocket.active_subscriptions);
    }

    #[deprecated(
        since = "0.2.0",
        note = "use `record_websocket_inbound_message_on(&channel)` — the global-only variant bypasses per-channel tracking"
    )]
    pub fn record_websocket_inbound_message(&self) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .inbound_messages_total
            .fetch_add(1, Ordering::Relaxed);
    }

    #[deprecated(
        since = "0.2.0",
        note = "use `record_websocket_outbound_message_on(&channel)` — the global-only variant bypasses per-channel tracking"
    )]
    pub fn record_websocket_outbound_message(&self) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .outbound_messages_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_websocket_subscription_opened_on(&self, channel: &ChannelId) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .subscriptions_total
            .fetch_add(1, Ordering::Relaxed);
        self.websocket
            .active_subscriptions
            .fetch_add(1, Ordering::Relaxed);
        let entry = self.websocket.entry(channel);
        entry.subscriptions_total.fetch_add(1, Ordering::Relaxed);
        entry.active_subscriptions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_websocket_subscription_closed_on(&self, channel: &ChannelId) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .unsubscribes_total
            .fetch_add(1, Ordering::Relaxed);
        decrement_saturating(&self.websocket.active_subscriptions);
        let entry = self.websocket.entry(channel);
        entry.unsubscribes_total.fetch_add(1, Ordering::Relaxed);
        decrement_saturating(&entry.active_subscriptions);
    }

    pub fn record_websocket_inbound_message_on(&self, channel: &ChannelId) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .inbound_messages_total
            .fetch_add(1, Ordering::Relaxed);
        self.websocket
            .entry(channel)
            .inbound_messages_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_websocket_outbound_message_on(&self, channel: &ChannelId) {
        if !self.capture_enabled {
            return;
        }
        self.websocket
            .outbound_messages_total
            .fetch_add(1, Ordering::Relaxed);
        self.websocket
            .entry(channel)
            .outbound_messages_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn register_websocket_channel(&self, channel: &ChannelId) {
        if !self.capture_enabled {
            return;
        }
        let _ = self.websocket.entry(channel);
    }

    pub fn record_scheduler_tick(&self) {
        if !self.capture_enabled {
            return;
        }
        self.scheduler.ticks_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_schedule_executed(&self) {
        if !self.capture_enabled {
            return;
        }
        self.scheduler
            .executed_schedules_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_scheduler_leadership(&self, state: SchedulerLeadershipState) {
        if !self.capture_enabled {
            return;
        }
        match state {
            SchedulerLeadershipState::Acquired => {
                self.scheduler
                    .leadership_acquired_total
                    .fetch_add(1, Ordering::Relaxed);
                self.scheduler.leader_active.store(true, Ordering::Relaxed);
            }
            SchedulerLeadershipState::Lost => {
                self.scheduler
                    .leadership_lost_total
                    .fetch_add(1, Ordering::Relaxed);
                self.scheduler.leader_active.store(false, Ordering::Relaxed);
            }
        }
    }

    pub fn set_scheduler_leader_active(&self, active: bool) {
        if !self.capture_enabled {
            return;
        }
        self.scheduler
            .leader_active
            .store(active, Ordering::Relaxed);
    }

    pub fn record_job_outcome(&self, outcome: JobOutcome) {
        if !self.capture_enabled {
            return;
        }
        match outcome {
            JobOutcome::Enqueued => {
                self.jobs.enqueued_total.fetch_add(1, Ordering::Relaxed);
            }
            JobOutcome::Leased => {
                self.jobs.leased_total.fetch_add(1, Ordering::Relaxed);
            }
            JobOutcome::Started => {
                self.jobs.started_total.fetch_add(1, Ordering::Relaxed);
            }
            JobOutcome::Succeeded => {
                self.jobs.succeeded_total.fetch_add(1, Ordering::Relaxed);
            }
            JobOutcome::Retried => {
                self.jobs.retried_total.fetch_add(1, Ordering::Relaxed);
            }
            JobOutcome::ExpiredLeaseRequeued => {
                self.jobs
                    .expired_requeues_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            JobOutcome::DeadLettered => {
                self.jobs
                    .dead_lettered_total
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn decrement_saturating(value: &AtomicU64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_sub(1))
    });
}

#[cfg(test)]
impl Default for RuntimeDiagnostics {
    fn default() -> Self {
        Self::new(
            crate::logging::types::RuntimeBackendKind::Memory,
            super::probes::ReadinessRegistry { checks: Vec::new() },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostics_with_config(config: RuntimeDiagnosticsConfig) -> RuntimeDiagnostics {
        RuntimeDiagnostics::new_with_config(
            RuntimeBackendKind::Memory,
            ReadinessRegistry { checks: Vec::new() },
            config,
        )
    }

    #[test]
    fn capture_disabled_makes_runtime_records_noop() {
        use axum::http::StatusCode;

        let diagnostics = diagnostics_with_config(RuntimeDiagnosticsConfig {
            capture_enabled: false,
            ..RuntimeDiagnosticsConfig::default()
        });

        diagnostics.record_http_response_with_duration(StatusCode::OK, 12);
        diagnostics.record_auth_outcome(AuthOutcome::Success);
        diagnostics.record_websocket_connection(WebSocketConnectionState::Opened);
        diagnostics.record_scheduler_tick();
        diagnostics.record_job_outcome(JobOutcome::Succeeded);

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.http.requests_total, 0);
        assert_eq!(snapshot.auth.success_total, 0);
        assert_eq!(snapshot.websocket.opened_total, 0);
        assert_eq!(snapshot.scheduler.ticks_total, 0);
        assert_eq!(snapshot.jobs.succeeded_total, 0);
    }

    #[test]
    fn per_channel_counters_start_at_zero_and_increment() {
        use crate::support::ChannelId;

        let diagnostics = RuntimeDiagnostics::default();
        let chat = ChannelId::new("chat");

        diagnostics.record_websocket_subscription_opened_on(&chat);
        diagnostics.record_websocket_inbound_message_on(&chat);
        diagnostics.record_websocket_outbound_message_on(&chat);
        diagnostics.record_websocket_outbound_message_on(&chat);

        let snapshot = diagnostics.snapshot().websocket;
        let channel = snapshot
            .channels
            .iter()
            .find(|c| c.id == chat)
            .expect("channel snapshot missing");
        assert_eq!(channel.subscriptions_total, 1);
        assert_eq!(channel.active_subscriptions, 1);
        assert_eq!(channel.inbound_messages_total, 1);
        assert_eq!(channel.outbound_messages_total, 2);

        assert_eq!(snapshot.subscriptions_total, 1);
        assert_eq!(snapshot.inbound_messages_total, 1);
        assert_eq!(snapshot.outbound_messages_total, 2);
    }

    #[test]
    fn http_duration_histogram_tracks_cumulative_buckets() {
        use axum::http::StatusCode;

        let diagnostics = RuntimeDiagnostics::default();

        diagnostics.record_http_response_with_duration(StatusCode::OK, 12);
        diagnostics.record_http_response_with_duration(StatusCode::OK, 600);
        diagnostics.record_http_response_with_duration(StatusCode::OK, 35_000);

        let histogram = diagnostics.snapshot().http.duration_ms;
        assert_eq!(histogram.count, 3);
        assert_eq!(histogram.sum_ms, 35_612);

        let le_25 = histogram
            .buckets
            .iter()
            .find(|bucket| bucket.le_ms == 25)
            .expect("25ms bucket missing");
        assert_eq!(le_25.cumulative_count, 1);

        let le_1_000 = histogram
            .buckets
            .iter()
            .find(|bucket| bucket.le_ms == 1_000)
            .expect("1000ms bucket missing");
        assert_eq!(le_1_000.cumulative_count, 2);

        let le_30_000 = histogram
            .buckets
            .iter()
            .find(|bucket| bucket.le_ms == 30_000)
            .expect("30000ms bucket missing");
        assert_eq!(le_30_000.cumulative_count, 2);
    }

    #[test]
    fn http_observability_snapshot_ranks_routes_and_recent_samples() {
        use axum::http::StatusCode;

        let diagnostics = RuntimeDiagnostics::default();

        diagnostics.record_http_request(HttpRequestRecord {
            method: "GET".to_string(),
            path: "/slow".to_string(),
            status: StatusCode::OK,
            duration_ms: 1_500,
            request_id: Some("req-slow-1".to_string()),
            trace_id: Some("trace-slow-1".to_string()),
        });
        diagnostics.record_http_request(HttpRequestRecord {
            method: "GET".to_string(),
            path: "/slow".to_string(),
            status: StatusCode::OK,
            duration_ms: 2_500,
            request_id: Some("req-slow-2".to_string()),
            trace_id: Some("trace-slow-2".to_string()),
        });
        diagnostics.record_http_request(HttpRequestRecord {
            method: "POST".to_string(),
            path: "/errors".to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
            duration_ms: 35,
            request_id: Some("req-error".to_string()),
            trace_id: Some("trace-error".to_string()),
        });

        let snapshot = diagnostics.http_observability_snapshot();

        assert_eq!(snapshot.stats.requests_total, 3);
        assert_eq!(snapshot.stats.retained_request_count, 3);
        assert_eq!(snapshot.stats.retention_capacity, 500);
        assert_eq!(snapshot.stats.slow_request_threshold_ms, 1_000);
        assert_eq!(snapshot.stats.route_count, 2);
        assert_eq!(snapshot.stats.slow_request_count, 2);
        assert_eq!(snapshot.stats.error_request_count, 1);

        assert_eq!(snapshot.top_slowest_routes[0].path, "/slow");
        assert_eq!(snapshot.top_slowest_routes[0].requests_total, 2);
        assert_eq!(snapshot.top_slowest_routes[0].avg_duration_ms, 2_000);
        assert_eq!(snapshot.top_slowest_routes[0].max_duration_ms, 2_500);
        assert_eq!(snapshot.top_slowest_routes[0].p95_duration_ms, 2_500);
        assert_eq!(snapshot.top_slowest_routes[0].p99_duration_ms, 2_500);

        assert_eq!(snapshot.top_error_routes.len(), 1);
        assert_eq!(snapshot.top_error_routes[0].path, "/errors");
        assert_eq!(snapshot.top_error_routes[0].server_error_total, 1);

        assert_eq!(snapshot.recent_slow_requests.len(), 2);
        assert_eq!(
            snapshot.recent_slow_requests[0].request_id.as_deref(),
            Some("req-slow-2")
        );
        assert_eq!(snapshot.recent_error_requests.len(), 1);
        assert_eq!(
            snapshot.recent_error_requests[0].trace_id.as_deref(),
            Some("trace-error")
        );
    }

    #[test]
    fn http_observability_retention_is_bounded() {
        use axum::http::StatusCode;

        let diagnostics = RuntimeDiagnostics::default();

        for index in 0..(HTTP_REQUEST_OBSERVATION_CAPACITY + 1) {
            diagnostics.record_http_request(HttpRequestRecord {
                method: "GET".to_string(),
                path: format!("/items/{index}"),
                status: StatusCode::OK,
                duration_ms: if index == 0 { 10_000 } else { 5 },
                request_id: Some(format!("req-{index}")),
                trace_id: Some(format!("trace-{index}")),
            });
        }

        let snapshot = diagnostics.http_observability_snapshot();

        assert_eq!(
            snapshot.stats.retained_request_count,
            HTTP_REQUEST_OBSERVATION_CAPACITY
        );
        assert_eq!(
            snapshot.stats.route_count,
            HTTP_REQUEST_OBSERVATION_CAPACITY
        );
        assert_eq!(snapshot.top_slowest_routes[0].max_duration_ms, 5);
    }

    #[test]
    fn http_observability_respects_configured_sample_retention() {
        use axum::http::StatusCode;

        let diagnostics = diagnostics_with_config(RuntimeDiagnosticsConfig {
            http_sample_retention: 1,
            ..RuntimeDiagnosticsConfig::default()
        });

        diagnostics.record_http_request(HttpRequestRecord {
            method: "GET".to_string(),
            path: "/first".to_string(),
            status: StatusCode::OK,
            duration_ms: 2_000,
            request_id: Some("first".to_string()),
            trace_id: None,
        });
        diagnostics.record_http_request(HttpRequestRecord {
            method: "GET".to_string(),
            path: "/second".to_string(),
            status: StatusCode::OK,
            duration_ms: 5,
            request_id: Some("second".to_string()),
            trace_id: None,
        });

        let snapshot = diagnostics.http_observability_snapshot();
        assert_eq!(snapshot.stats.requests_total, 2);
        assert_eq!(snapshot.stats.retained_request_count, 1);
        assert_eq!(snapshot.stats.retention_capacity, 1);
        assert_eq!(snapshot.top_slowest_routes[0].path, "/second");
    }

    #[test]
    fn websocket_channel_retention_evicts_idle_channels() {
        use crate::support::ChannelId;

        let diagnostics = diagnostics_with_config(RuntimeDiagnosticsConfig {
            websocket_channel_retention: 1,
            ..RuntimeDiagnosticsConfig::default()
        });
        let alpha = ChannelId::new("alpha");
        let beta = ChannelId::new("beta");

        diagnostics.register_websocket_channel(&alpha);
        diagnostics.record_websocket_inbound_message_on(&beta);

        let channels = diagnostics.snapshot().websocket.channels;
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].id, beta);
    }
}
