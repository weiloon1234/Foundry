use async_trait::async_trait;

use super::{reqwest_transport_error, HttpClientResult, HttpRequest, HttpResponse};

/// Pluggable execution boundary for outbound HTTP requests.
#[async_trait]
pub trait HttpTransport: Send + Sync + 'static {
    async fn send(&self, request: HttpRequest) -> HttpClientResult<HttpResponse>;
}

/// Production transport backed by a pooled [`reqwest::Client`].
#[derive(Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub fn raw(&self) -> &reqwest::Client {
        &self.client
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> HttpClientResult<HttpResponse> {
        let (method, url, headers, body) = request.into_parts();
        let mut request = self.client.request(method, url).headers(headers);
        if let Some(body) = body {
            request = request.body(body);
        }

        let response = request.send().await.map_err(reqwest_transport_error)?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(reqwest_transport_error)?
            .to_vec();

        Ok(HttpResponse::new(status)
            .with_headers(headers)
            .with_body(body))
    }
}
