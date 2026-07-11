use std::fmt;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{HttpClientError, HttpClientResult};

/// Buffered, transport-neutral HTTP response with typed decoding helpers.
#[derive(Clone)]
pub struct HttpResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    pub fn from_json<T>(status: StatusCode, value: &T) -> HttpClientResult<Self>
    where
        T: Serialize + ?Sized,
    {
        let body = serde_json::to_vec(value).map_err(|error| HttpClientError::Encode {
            target: "response JSON",
            message: error.to_string(),
        })?;
        let mut response = Self::new(status).with_body(body);
        response
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(response)
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.body
    }

    pub fn text(&self) -> HttpClientResult<&str> {
        std::str::from_utf8(&self.body).map_err(|error| HttpClientError::Decode {
            target: "UTF-8 text",
            message: error.to_string(),
        })
    }

    pub fn json<T>(&self) -> HttpClientResult<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).map_err(|error| HttpClientError::Decode {
            target: "JSON",
            message: error.to_string(),
        })
    }

    pub fn ensure_success(&self) -> HttpClientResult<()> {
        if self.status.is_success() {
            Ok(())
        } else {
            Err(HttpClientError::Status {
                status: self.status,
            })
        }
    }

    pub fn error_for_status(self) -> HttpClientResult<Self> {
        if self.status.is_client_error() || self.status.is_server_error() {
            Err(HttpClientError::Status {
                status: self.status,
            })
        } else {
            Ok(self)
        }
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body_bytes", &self.body.len())
            .finish()
    }
}
