use std::time::Duration;

use reqwest::StatusCode;

pub type HttpClientResult<T> = std::result::Result<T, HttpClientError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpClientErrorKind {
    InvalidUrl,
    InvalidHeader,
    Build,
    Encode,
    Transport,
    Timeout,
    ConcurrencyClosed,
    Decode,
    Status,
    FakeExhausted,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpClientError {
    #[error("invalid HTTP URL: {message}")]
    InvalidUrl { message: String },

    #[error("invalid HTTP header `{name}`: {message}")]
    InvalidHeader { name: String, message: String },

    #[error("failed to build HTTP client: {message}")]
    Build { message: String },

    #[error("failed to encode HTTP {target}: {message}")]
    Encode {
        target: &'static str,
        message: String,
    },

    #[error("HTTP transport failed: {message}")]
    Transport { message: String },

    #[error("HTTP request timed out after {timeout:?}")]
    Timeout { timeout: Duration },

    #[error("HTTP client concurrency limiter is closed")]
    ConcurrencyClosed,

    #[error("failed to decode HTTP response as {target}: {message}")]
    Decode {
        target: &'static str,
        message: String,
    },

    #[error("HTTP response returned status {status}")]
    Status { status: StatusCode },

    #[error("HTTP client fake has no response or error queued")]
    FakeExhausted,
}

impl HttpClientError {
    pub fn kind(&self) -> HttpClientErrorKind {
        match self {
            Self::InvalidUrl { .. } => HttpClientErrorKind::InvalidUrl,
            Self::InvalidHeader { .. } => HttpClientErrorKind::InvalidHeader,
            Self::Build { .. } => HttpClientErrorKind::Build,
            Self::Encode { .. } => HttpClientErrorKind::Encode,
            Self::Transport { .. } => HttpClientErrorKind::Transport,
            Self::Timeout { .. } => HttpClientErrorKind::Timeout,
            Self::ConcurrencyClosed => HttpClientErrorKind::ConcurrencyClosed,
            Self::Decode { .. } => HttpClientErrorKind::Decode,
            Self::Status { .. } => HttpClientErrorKind::Status,
            Self::FakeExhausted => HttpClientErrorKind::FakeExhausted,
        }
    }

    pub fn transport(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
        }
    }

    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Status { status } => Some(*status),
            _ => None,
        }
    }

    pub fn timeout_duration(&self) -> Option<Duration> {
        match self {
            Self::Timeout { timeout } => Some(*timeout),
            _ => None,
        }
    }
}

pub(crate) fn reqwest_transport_error(error: reqwest::Error) -> HttpClientError {
    HttpClientError::Transport {
        message: error.without_url().to_string(),
    }
}

impl From<HttpClientError> for crate::foundation::Error {
    fn from(error: HttpClientError) -> Self {
        Self::other(error)
    }
}
