use axum::response::{IntoResponse, Response};
use axum::{http::StatusCode, Json};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("validation failed")]
pub struct ValidationErrors {
    pub errors: Vec<FieldError>,
}

impl ValidationErrors {
    pub fn new(errors: Vec<FieldError>) -> Self {
        Self { errors }
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl IntoResponse for ValidationErrors {
    fn into_response(self) -> Response {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "message": "Validation failed",
                "status": 422,
                "errors": self.errors,
            })),
        )
            .into_response()
    }
}
