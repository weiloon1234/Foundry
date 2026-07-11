//! Typed outbound HTTP requests with shared policy, retry safety, observability,
//! transport substitution, and deterministic testing support.

mod client;
mod error;
mod request;
mod response;
mod retry;
mod transport;

pub use client::{HttpClient, HttpClientBuilder, HttpRequestBuilder};
pub(crate) use error::reqwest_transport_error;
pub use error::{HttpClientError, HttpClientErrorKind, HttpClientResult};
pub use request::HttpRequest;
pub use response::HttpResponse;
pub use retry::RetryPolicy;
pub use transport::{HttpTransport, ReqwestTransport};

pub use reqwest::header::{
    HeaderMap as HttpHeaderMap, HeaderName as HttpHeaderName, HeaderValue as HttpHeaderValue,
};

#[cfg(test)]
mod tests;
pub use reqwest::{
    Client as RawHttpClient, Method as HttpMethod, StatusCode as HttpStatus, Url as HttpUrl,
};
