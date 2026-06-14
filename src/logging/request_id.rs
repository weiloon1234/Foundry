use std::fmt;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::FromRequestParts;
use axum::http::{request::Parts, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

pub const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(String);

impl RequestId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for RequestId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for RequestId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<S> FromRequestParts<S> for RequestId
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        parts.extensions.get::<RequestId>().cloned().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "message": "request id missing from request context",
                })),
            )
                .into_response()
        })
    }
}

pub(super) fn generate_request_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("foundry-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}
