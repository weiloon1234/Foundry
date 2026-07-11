use std::fmt;
use std::ops::Deref;

use axum::extract::FromRequestParts;
use axum::http::{request::Parts, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

pub const REQUEST_ID_HEADER: &str = "x-request-id";
pub const REQUEST_ID_MAX_LENGTH: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RequestIdError {
    #[error("request ID cannot be empty")]
    Empty,
    #[error("request ID exceeds the {max} byte limit")]
    TooLong { max: usize },
    #[error("request ID contains an invalid character at byte {index}")]
    InvalidCharacter { index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(String);

impl RequestId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("request ID must be non-empty, bounded, visible ASCII")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, RequestIdError> {
        let value = value.into();
        validate_request_id(&value)?;
        Ok(Self(value))
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
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

impl std::str::FromStr for RequestId {
    type Err = RequestIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value.to_string())
    }
}

impl TryFrom<String> for RequestId {
    type Error = RequestIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl TryFrom<&str> for RequestId {
    type Error = RequestIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_new(value.to_string())
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

fn validate_request_id(value: &str) -> Result<(), RequestIdError> {
    if value.is_empty() {
        return Err(RequestIdError::Empty);
    }
    if value.len() > REQUEST_ID_MAX_LENGTH {
        return Err(RequestIdError::TooLong {
            max: REQUEST_ID_MAX_LENGTH,
        });
    }
    if let Some((index, _)) = value
        .bytes()
        .enumerate()
        .find(|(_, byte)| !byte.is_ascii_graphic())
    {
        return Err(RequestIdError::InvalidCharacter { index });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{RequestId, RequestIdError, REQUEST_ID_MAX_LENGTH};

    #[test]
    fn request_id_accepts_bounded_visible_ascii() {
        let value = "client.request_01:region/eu";
        assert_eq!(RequestId::try_new(value).unwrap().as_str(), value);
        assert!(RequestId::try_new("x".repeat(REQUEST_ID_MAX_LENGTH)).is_ok());
    }

    #[test]
    fn request_id_rejects_empty_oversized_whitespace_and_non_ascii_values() {
        assert_eq!(RequestId::try_new("").unwrap_err(), RequestIdError::Empty);
        assert!(matches!(
            RequestId::try_new("x".repeat(REQUEST_ID_MAX_LENGTH + 1)),
            Err(RequestIdError::TooLong { .. })
        ));
        for value in ["has space", "has\ttab", "ümlaut"] {
            assert!(matches!(
                RequestId::try_new(value),
                Err(RequestIdError::InvalidCharacter { .. })
            ));
        }
    }

    #[test]
    fn generated_request_ids_are_unique_uuid_v7_values() {
        let first = RequestId::generate();
        let second = RequestId::generate();
        assert_ne!(first, second);

        for request_id in [first, second] {
            let uuid = Uuid::parse_str(request_id.as_str()).unwrap();
            assert_eq!(uuid.get_version_num(), 7);
        }
    }
}
