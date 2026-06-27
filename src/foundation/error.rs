use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::error::Error as _;

/// Unified error type for the Foundry framework.
///
/// Produces consistent JSON error responses across HTTP, validation,
/// auth, and internal errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A plain message error (used throughout the framework).
    /// Maps to HTTP 500 Internal Server Error.
    #[error("{0}")]
    Message(String),

    /// An HTTP error with a specific status code.
    #[error("{message}")]
    Http {
        status: u16,
        message: String,
        error_code: Option<String>,
        message_key: Option<String>,
    },

    /// Validation errors with per-field detail. Maps to HTTP 422.
    #[error("validation failed")]
    Validation(crate::validation::ValidationErrors),

    /// A "not found" error. Maps to HTTP 404.
    #[error("{0}")]
    NotFound(String),

    /// Wraps anyhow::Error for backward compatibility.
    /// Maps to HTTP 500 Internal Server Error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    const INTERNAL_SERVER_ERROR_MESSAGE: &'static str = "Internal server error";

    /// Public message used for server-side failures.
    ///
    /// The full internal error stays available to logs and error reporters via
    /// the response extension added in [`IntoResponse`].
    pub const fn internal_server_error_message() -> &'static str {
        Self::INTERNAL_SERVER_ERROR_MESSAGE
    }

    /// Create a message error (replaces old `Error::message()`).
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    /// Create an HTTP error with a specific status code.
    pub fn http(status: u16, message: impl Into<String>) -> Self {
        Self::Http {
            status,
            message: message.into(),
            error_code: None,
            message_key: None,
        }
    }

    /// Create an HTTP error with a specific status code and error code.
    pub fn http_with_code(
        status: u16,
        message: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        Self::Http {
            status,
            message: message.into(),
            error_code: Some(code.into()),
            message_key: None,
        }
    }

    /// Create an HTTP error with optional machine-readable metadata.
    pub fn http_with_metadata(
        status: u16,
        message: impl Into<String>,
        error_code: Option<String>,
        message_key: Option<String>,
    ) -> Self {
        Self::Http {
            status,
            message: message.into(),
            error_code,
            message_key,
        }
    }

    /// Create a 404 Not Found error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    /// Wrap an arbitrary error.
    pub fn other<E>(error: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::Other(error.into())
    }

    fn status_code(&self) -> StatusCode {
        match self {
            Error::Message(_) | Error::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Http { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            }
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    fn public_message(&self) -> String {
        let status = self.status_code();
        if status.is_server_error() {
            return Self::internal_server_error_message().to_string();
        }

        self.to_string()
    }

    pub fn source_chain(&self) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = self.source();
        while let Some(error) = current {
            chain.push(error.to_string());
            current = error.source();
        }
        chain
    }

    pub fn payload(&self) -> serde_json::Value {
        let status = self.status_code();
        let (error_code, message_key) = match self {
            Error::Http {
                error_code,
                message_key,
                ..
            } => (error_code.clone(), message_key.clone()),
            _ => (None, None),
        };

        let mut payload = serde_json::json!({
            "message": self.public_message(),
            "status": status.as_u16(),
        });

        if let Some(error_code) = error_code {
            payload["error_code"] = serde_json::Value::String(error_code);
        }
        if let Some(message_key) = message_key {
            payload["message_key"] = serde_json::Value::String(message_key);
        }

        payload
    }
}

/// The standard JSON error response body.
#[derive(
    Debug,
    Serialize,
    serde::Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct ErrorResponse {
    pub message: String,
    pub status: u16,
    pub error_code: Option<String>,
    pub message_key: Option<String>,
    pub errors: Option<Vec<crate::validation::FieldError>>,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        // Validation errors delegate to their own structured response.
        if let Error::Validation(errors) = self {
            return errors.into_response();
        }

        let status = self.status_code();
        let body = ErrorResponse {
            message: self.public_message(),
            status: status.as_u16(),
            error_code: match &self {
                Error::Http { error_code, .. } => error_code.clone(),
                _ => None,
            },
            message_key: match &self {
                Error::Http { message_key, .. } => message_key.clone(),
                _ => None,
            },
            errors: None,
        };
        let error_text = self.to_string();
        let chain = self.source_chain();
        let mut response = (status, Json(body)).into_response();
        crate::logging::mark_handler_error_response(
            &mut response,
            status.as_u16(),
            error_text,
            chain,
        );
        response
    }
}

/// Allow `ValidationErrors` to be converted into `Error`.
impl From<crate::validation::ValidationErrors> for Error {
    fn from(errors: crate::validation::ValidationErrors) -> Self {
        Self::Validation(errors)
    }
}

/// Allow `AuthError` to be converted into `Error`.
impl From<crate::auth::AuthError> for Error {
    fn from(error: crate::auth::AuthError) -> Self {
        let status = error.status_code().as_u16();
        let message = error.message().to_string();
        let error_code = error.code().map(|code| code.as_str().to_string());
        let message_key = error.code().map(|code| code.translation_key().to_string());
        Self::http_with_metadata(status, message, error_code, message_key)
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use crate::auth::{AuthError, AuthErrorCode};

    use super::Error;

    #[test]
    fn auth_error_conversion_preserves_message_metadata() {
        let error = Error::from(AuthError::unauthorized_code(
            AuthErrorCode::MissingAuthCredentials,
        ));
        let payload = error.payload();

        assert_eq!(
            payload["message"],
            "Authentication credentials are required."
        );
        assert_eq!(payload["error_code"], "missing_auth_credentials");
        assert_eq!(payload["message_key"], "auth.missing_auth_credentials");
    }

    #[test]
    fn internal_error_payload_uses_generic_public_message() {
        let leaked_sql = r#"database query failed while running `INSERT INTO "users"`: password authentication failed"#;
        let payload = Error::other(anyhow::anyhow!(leaked_sql)).payload();

        assert_eq!(payload["status"], 500);
        assert_eq!(payload["message"], Error::INTERNAL_SERVER_ERROR_MESSAGE);
        assert!(!payload["message"].as_str().unwrap().contains("INSERT INTO"));
        assert!(!payload["message"].as_str().unwrap().contains("password"));
    }

    #[test]
    fn client_error_payload_preserves_public_message() {
        let payload = Error::http(422, "The email field is invalid.").payload();

        assert_eq!(payload["status"], 422);
        assert_eq!(payload["message"], "The email field is invalid.");
    }

    #[tokio::test]
    async fn internal_error_response_body_uses_generic_public_message() {
        let leaked_sql = r#"database query failed while running `INSERT INTO "users"`"#;
        let response = Error::message(leaked_sql).into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["status"], 500);
        assert_eq!(payload["message"], Error::INTERNAL_SERVER_ERROR_MESSAGE);
        assert!(!payload["message"].as_str().unwrap().contains("INSERT INTO"));
    }
}
