use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ConnectInfo;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};

use crate::auth::Actor;
use crate::http::middleware::RealIp;

use super::RequestId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentRequest {
    pub request_id: Option<String>,
    pub ip: Option<IpAddr>,
    pub user_agent: Option<String>,
    pub audit_area: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionContext {
    Http {
        method: String,
        path: String,
        request_id: Option<String>,
    },
    Job {
        class: String,
        id: String,
    },
    Scheduler {
        id: String,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TraceContext {
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TraceParent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TraceParent {
    pub kind: String,
    pub id: String,
}

tokio::task_local! {
    static CURRENT_REQUEST: CurrentRequest;
}

tokio::task_local! {
    static CURRENT_ACTOR: Actor;
}

tokio::task_local! {
    static CURRENT_EXECUTION: ExecutionContext;
}

tokio::task_local! {
    static CURRENT_TRACE: TraceContext;
}

impl CurrentRequest {
    pub(crate) fn from_parts(parts: &Parts) -> Self {
        if let Some(current) = parts.extensions.get::<Self>() {
            return current.clone();
        }

        Self {
            request_id: parts
                .extensions
                .get::<RequestId>()
                .map(|value| value.as_str().to_string()),
            ip: parts
                .extensions
                .get::<RealIp>()
                .map(|value| value.0)
                .or_else(|| {
                    parts
                        .extensions
                        .get::<ConnectInfo<SocketAddr>>()
                        .map(|ConnectInfo(addr)| addr.ip())
                }),
            user_agent: parts
                .headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            audit_area: None,
        }
    }

    pub(crate) fn with_audit_area(mut self, audit_area: Option<String>) -> Self {
        self.audit_area = audit_area;
        self
    }
}

impl TraceContext {
    pub(crate) fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            request_id: None,
            parent: None,
        }
    }

    pub(crate) fn http(request_id: String) -> Self {
        Self {
            trace_id: request_id.clone(),
            request_id: Some(request_id),
            parent: None,
        }
    }

    pub(crate) fn generated() -> Self {
        Self::new(generate_trace_id())
    }

    pub(crate) fn with_parent(mut self, parent: Option<TraceParent>) -> Self {
        self.parent = parent;
        self
    }
}

impl TraceParent {
    pub(crate) fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

impl<S> FromRequestParts<S> for CurrentRequest
where
    S: Send + Sync,
{
    type Rejection = crate::foundation::Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_parts(parts))
    }
}

pub(crate) fn current_request() -> Option<CurrentRequest> {
    CURRENT_REQUEST.try_with(|request| request.clone()).ok()
}

pub(crate) fn current_actor() -> Option<Actor> {
    CURRENT_ACTOR.try_with(|actor| actor.clone()).ok()
}

pub(crate) fn current_execution() -> Option<ExecutionContext> {
    CURRENT_EXECUTION.try_with(|context| context.clone()).ok()
}

pub(crate) fn current_trace_context() -> Option<TraceContext> {
    CURRENT_TRACE.try_with(|context| context.clone()).ok()
}

pub fn current_trace_id() -> Option<String> {
    current_trace_context().map(|context| context.trace_id)
}

pub(crate) fn current_execution_trace_parent() -> Option<TraceParent> {
    current_execution().and_then(|context| trace_parent_from_execution(&context))
}

pub(crate) fn trace_context_for_child(parent: Option<TraceParent>) -> TraceContext {
    current_trace_context()
        .unwrap_or_else(TraceContext::generated)
        .with_parent(parent)
}

pub(crate) async fn scope_current_request<F, T>(request: CurrentRequest, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_REQUEST.scope(request, future).await
}

pub(crate) async fn scope_current_actor<F, T>(actor: Actor, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_ACTOR.scope(actor, future).await
}

pub(crate) async fn scope_current_execution<F, T>(context: ExecutionContext, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_EXECUTION.scope(context, future).await
}

pub(crate) async fn scope_current_trace<F, T>(context: TraceContext, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_TRACE.scope(context, future).await
}

fn trace_parent_from_execution(context: &ExecutionContext) -> Option<TraceParent> {
    match context {
        ExecutionContext::Http { request_id, .. } => request_id
            .as_ref()
            .map(|request_id| TraceParent::new("http", request_id.clone())),
        ExecutionContext::Job { id, .. } => Some(TraceParent::new("job", id.clone())),
        ExecutionContext::Scheduler { id } => Some(TraceParent::new("scheduler", id.clone())),
        ExecutionContext::Other => None,
    }
}

fn generate_trace_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("foundry-trace-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::http::Request;

    use super::*;

    #[test]
    fn current_request_uses_real_ip_before_connect_info() {
        let (mut parts, _) = Request::builder().body(()).unwrap().into_parts();
        parts.extensions.insert(ConnectInfo(
            "203.0.113.10:4321".parse::<SocketAddr>().unwrap(),
        ));
        parts
            .extensions
            .insert(RealIp("198.51.100.7".parse().unwrap()));

        let current = CurrentRequest::from_parts(&parts);

        assert_eq!(current.ip, Some("198.51.100.7".parse().unwrap()));
    }

    #[test]
    fn current_request_falls_back_to_connect_info() {
        let (mut parts, _) = Request::builder().body(()).unwrap().into_parts();
        parts.extensions.insert(ConnectInfo(
            "203.0.113.10:4321".parse::<SocketAddr>().unwrap(),
        ));

        let current = CurrentRequest::from_parts(&parts);

        assert_eq!(current.ip, Some("203.0.113.10".parse().unwrap()));
    }
}
