use std::fmt;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use serde::de::DeserializeOwned;

use crate::support::redaction::{redact_sensitive_text, REDACTED};

use super::{HttpClientError, HttpClientResult};

/// Transport-neutral outbound request used by production and fake transports.
#[derive(Clone)]
pub struct HttpRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: HeaderMap::new(),
            body: None,
        }
    }

    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn redacted_url(&self) -> String {
        redact_http_url(&self.url)
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }

    pub fn json_body<T>(&self) -> HttpClientResult<T>
    where
        T: DeserializeOwned,
    {
        let body = self.body.as_deref().unwrap_or_default();
        serde_json::from_slice(body).map_err(|error| HttpClientError::Decode {
            target: "request JSON",
            message: error.to_string(),
        })
    }

    pub fn query_pairs(&self) -> Vec<(String, String)> {
        self.url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    pub(crate) fn into_parts(self) -> (Method, Url, HeaderMap, Option<Vec<u8>>) {
        (self.method, self.url, self.headers, self.body)
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.redacted_url())
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body_bytes", &self.body.as_ref().map_or(0, Vec::len))
            .finish()
    }
}

pub(crate) fn redact_http_url(url: &Url) -> String {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_fragment(None);

    let query_keys = redacted
        .query_pairs()
        .map(|(key, _)| key.into_owned())
        .collect::<Vec<_>>();
    redacted.set_query(None);
    if !query_keys.is_empty() {
        let mut query = redacted.query_pairs_mut();
        for key in query_keys {
            query.append_pair(&key, REDACTED);
        }
    }

    redact_sensitive_text(redacted.as_str())
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderValue, AUTHORIZATION};
    use reqwest::{Method, Url};

    use super::HttpRequest;

    #[test]
    fn debug_and_trace_url_do_not_expose_credentials_headers_query_values_or_body() {
        let request = HttpRequest::new(
            Method::POST,
            Url::parse("https://user:password@example.test/path?token=secret&email=user@example.test#private")
                .unwrap(),
        )
        .with_header(AUTHORIZATION, HeaderValue::from_static("Bearer secret-token"))
        .with_body("super-secret-body");

        let redacted_url = request.redacted_url();
        let debug = format!("{request:?}");

        for secret in [
            "user",
            "password",
            "secret",
            "user@example.test",
            "private",
            "secret-token",
            "super-secret-body",
        ] {
            assert!(!redacted_url.contains(secret));
            assert!(!debug.contains(secret));
        }
        assert!(redacted_url.contains("token="));
        assert!(redacted_url.contains("email="));
        assert!(debug.contains("authorization"));
        assert!(debug.contains("body_bytes"));
    }
}
