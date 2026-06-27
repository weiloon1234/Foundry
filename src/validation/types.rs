use axum::response::{IntoResponse, Response};
use axum::{http::StatusCode, Json};
use serde::{Deserialize, Serialize};

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
pub struct FieldError {
    pub field: String,
    pub code: String,
    pub message: String,
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
pub struct ValidationErrorResponse {
    pub message: String,
    pub status: u32,
    pub errors: Vec<FieldError>,
}

impl ValidationErrorResponse {
    pub fn new(errors: Vec<FieldError>) -> Self {
        Self {
            message: "Validation failed".to_string(),
            status: StatusCode::UNPROCESSABLE_ENTITY.as_u16().into(),
            errors,
        }
    }
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
            Json(ValidationErrorResponse::new(self.errors)),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{FieldError, ValidationErrorResponse};

    #[test]
    fn validation_error_response_serializes_standard_422_envelope() {
        let response = ValidationErrorResponse::new(vec![FieldError {
            field: "email".to_string(),
            code: "required".to_string(),
            message: "The email field is required.".to_string(),
        }]);

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "message": "Validation failed",
                "status": 422,
                "errors": [
                    {
                        "field": "email",
                        "code": "required",
                        "message": "The email field is required."
                    }
                ]
            })
        );
    }
}
