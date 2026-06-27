use serde::{Deserialize, Serialize};

/// Simple health/status response shape.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct StatusResponse {
    pub status: String,
}

impl StatusResponse {
    pub fn new(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
        }
    }

    pub fn ok() -> Self {
        Self::new("ok")
    }
}

/// Small typed success payload for simple endpoints.
///
/// ```ignore
/// async fn ping() -> impl IntoResponse {
///     Json(MessageResponse::ok())
/// }
/// ```
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct MessageResponse {
    pub message: String,
}

impl MessageResponse {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn ok() -> Self {
        Self::new("ok")
    }
}

/// CSRF token payload returned by endpoints that expose the current token.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct CsrfTokenResponse {
    pub token: String,
}

impl CsrfTokenResponse {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}
