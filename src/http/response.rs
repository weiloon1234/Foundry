use serde::{Deserialize, Serialize};

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
