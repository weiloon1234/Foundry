use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Serialize;

use crate::http_client::{
    HttpClient, HttpClientBuilder, HttpClientError, HttpClientResult, HttpRequest, HttpResponse,
    HttpTransport,
};
use crate::support::sync::lock_unpoisoned;

/// Deterministic outbound HTTP transport with queued results and recorded requests.
#[derive(Clone, Default)]
pub struct HttpClientFake {
    state: Arc<Mutex<HttpClientFakeState>>,
}

#[derive(Default)]
struct HttpClientFakeState {
    queued: VecDeque<HttpClientResult<HttpResponse>>,
    requests: Vec<HttpRequest>,
}

impl HttpClientFake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn client(&self) -> HttpClient {
        HttpClient::from_transport(self.clone())
            .expect("the default fake HTTP client configuration is valid")
    }

    pub fn client_builder(&self) -> HttpClientBuilder {
        HttpClient::builder().transport(self.clone())
    }

    pub fn respond(&self, response: HttpResponse) -> &Self {
        lock_unpoisoned(&self.state, "HTTP client fake")
            .queued
            .push_back(Ok(response));
        self
    }

    pub fn respond_json<T>(&self, status: reqwest::StatusCode, value: &T) -> HttpClientResult<&Self>
    where
        T: Serialize + ?Sized,
    {
        self.respond(HttpResponse::from_json(status, value)?);
        Ok(self)
    }

    pub fn fail(&self, error: HttpClientError) -> &Self {
        lock_unpoisoned(&self.state, "HTTP client fake")
            .queued
            .push_back(Err(error));
        self
    }

    pub fn sequence<I>(&self, sequence: I) -> &Self
    where
        I: IntoIterator<Item = HttpClientResult<HttpResponse>>,
    {
        lock_unpoisoned(&self.state, "HTTP client fake")
            .queued
            .extend(sequence);
        self
    }

    pub fn requests(&self) -> Vec<HttpRequest> {
        lock_unpoisoned(&self.state, "HTTP client fake")
            .requests
            .clone()
    }

    pub fn pending_responses(&self) -> usize {
        lock_unpoisoned(&self.state, "HTTP client fake")
            .queued
            .len()
    }

    pub fn reset(&self) -> &Self {
        *lock_unpoisoned(&self.state, "HTTP client fake") = HttpClientFakeState::default();
        self
    }

    pub fn assert_sent_count(&self, expected: usize) -> &Self {
        let actual = lock_unpoisoned(&self.state, "HTTP client fake")
            .requests
            .len();
        assert_eq!(
            actual, expected,
            "expected {expected} outbound HTTP requests, recorded {actual}"
        );
        self
    }

    pub fn assert_sent<F>(&self, predicate: F) -> &Self
    where
        F: Fn(&HttpRequest) -> bool,
    {
        let requests = self.requests();
        assert!(
            requests.iter().any(predicate),
            "no recorded outbound HTTP request matched the assertion; requests: {:?}",
            requests
        );
        self
    }

    pub fn assert_not_sent<F>(&self, predicate: F) -> &Self
    where
        F: Fn(&HttpRequest) -> bool,
    {
        let requests = self.requests();
        assert!(
            !requests.iter().any(predicate),
            "a recorded outbound HTTP request unexpectedly matched the assertion; requests: {:?}",
            requests
        );
        self
    }

    pub fn assert_nothing_sent(&self) -> &Self {
        self.assert_sent_count(0)
    }
}

#[async_trait]
impl HttpTransport for HttpClientFake {
    async fn send(&self, request: HttpRequest) -> HttpClientResult<HttpResponse> {
        let mut state = lock_unpoisoned(&self.state, "HTTP client fake");
        state.requests.push(request);
        state
            .queued
            .pop_front()
            .unwrap_or(Err(HttpClientError::FakeExhausted))
    }
}
