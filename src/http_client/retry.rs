use std::collections::HashSet;
use std::time::Duration;

use reqwest::{Method, StatusCode};

use super::HttpClientError;

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    max_attempts: usize,
    initial_backoff: Duration,
    max_backoff: Duration,
    methods: HashSet<Method>,
    statuses: HashSet<StatusCode>,
    retry_transport_errors: bool,
}

impl RetryPolicy {
    /// A conservative retry policy for read-only, idempotent requests.
    ///
    /// GET, HEAD, and OPTIONS retry transport failures plus common transient
    /// statuses up to three total attempts. Mutation methods are not retried
    /// unless explicitly added with [`Self::retry_method`].
    pub fn idempotent() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
            methods: [Method::GET, Method::HEAD, Method::OPTIONS]
                .into_iter()
                .collect(),
            statuses: [
                StatusCode::REQUEST_TIMEOUT,
                StatusCode::TOO_MANY_REQUESTS,
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::BAD_GATEWAY,
                StatusCode::SERVICE_UNAVAILABLE,
                StatusCode::GATEWAY_TIMEOUT,
            ]
            .into_iter()
            .collect(),
            retry_transport_errors: true,
        }
    }

    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            methods: HashSet::new(),
            statuses: HashSet::new(),
            retry_transport_errors: false,
        }
    }

    pub fn max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    pub fn backoff(mut self, initial: Duration, maximum: Duration) -> Self {
        self.initial_backoff = initial;
        self.max_backoff = maximum.max(initial);
        self
    }

    pub fn retry_method(mut self, method: Method) -> Self {
        self.methods.insert(method);
        self
    }

    pub fn do_not_retry_method(mut self, method: &Method) -> Self {
        self.methods.remove(method);
        self
    }

    pub fn retry_status(mut self, status: StatusCode) -> Self {
        self.statuses.insert(status);
        self
    }

    pub fn do_not_retry_status(mut self, status: &StatusCode) -> Self {
        self.statuses.remove(status);
        self
    }

    pub fn retry_transport_errors(mut self, retry: bool) -> Self {
        self.retry_transport_errors = retry;
        self
    }

    pub fn attempts(&self) -> usize {
        self.max_attempts
    }

    pub fn retries_method(&self, method: &Method) -> bool {
        self.max_attempts > 1 && self.methods.contains(method)
    }

    pub fn retries_status(&self, status: StatusCode) -> bool {
        self.statuses.contains(&status)
    }

    pub(crate) fn retries_error(&self, error: &HttpClientError) -> bool {
        self.retry_transport_errors
            && matches!(
                error,
                HttpClientError::Transport { .. } | HttpClientError::Timeout { .. }
            )
    }

    pub(crate) fn backoff_for_retry(&self, retry_index: usize) -> Duration {
        if self.initial_backoff.is_zero() {
            return Duration::ZERO;
        }

        let exponent = retry_index.saturating_sub(1).min(31) as u32;
        self.initial_backoff
            .checked_mul(2_u32.saturating_pow(exponent))
            .unwrap_or(self.max_backoff)
            .min(self.max_backoff)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::idempotent()
    }
}
