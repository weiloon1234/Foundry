use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::config::LockoutConfig;
use crate::events::Event;
use crate::foundation::{AppContext, Error, Result};
use crate::support::runtime::RuntimeBackend;
use crate::support::{sha256_hex_str, EventId};

const LOCKED_PREFIX: &str = "LOCKED:";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoginLockedOutEvent {
    pub identifier: String,
    pub locked_until: DateTime<Utc>,
}

impl Event for LoginLockedOutEvent {
    const ID: EventId = EventId::new("auth.login_locked_out");
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LockoutError {
    #[error("Too many failed login attempts. Try again at {until}.")]
    LockedOut { until: DateTime<Utc> },
}

impl LockoutError {
    pub fn retry_after_seconds(&self) -> u64 {
        match self {
            Self::LockedOut { until } => (*until - Utc::now()).num_seconds().max(0) as u64,
        }
    }
}

impl From<LockoutError> for Error {
    fn from(error: LockoutError) -> Self {
        match error {
            LockoutError::LockedOut { until } => Error::http_with_metadata(
                429,
                format!("Too many failed login attempts. Try again at {until}."),
                Some("login_locked_out".to_string()),
                Some("auth.login_locked_out".to_string()),
            ),
        }
    }
}

#[async_trait]
pub trait LockoutStore: Send + Sync + 'static {
    async fn get(&self, key: &str) -> Result<Option<String>>;

    async fn increment_failures(&self, key: &str, window: Duration) -> Result<u64>;

    async fn set_locked_until(&self, key: &str, until: DateTime<Utc>, ttl: Duration) -> Result<()>;

    async fn clear(&self, key: &str) -> Result<()>;
}

pub struct LoginThrottle {
    app: AppContext,
    store: Arc<dyn LockoutStore>,
    enabled: bool,
    max_failures: u32,
    lockout_duration: Duration,
    window: Duration,
}

impl LoginThrottle {
    pub fn new(app: &AppContext) -> Result<Self> {
        Self::with_store(app, Arc::new(RuntimeLockoutStore::from_app(app)?))
    }

    pub fn with_store(app: &AppContext, store: Arc<dyn LockoutStore>) -> Result<Self> {
        let config = app.config().auth()?.lockout;
        Ok(Self::from_config(app.clone(), store, config))
    }

    pub(crate) fn from_config(
        app: AppContext,
        store: Arc<dyn LockoutStore>,
        config: LockoutConfig,
    ) -> Self {
        Self {
            app,
            store,
            enabled: config.enabled,
            max_failures: config.max_failures.max(1),
            lockout_duration: Duration::from_secs(config.lockout_minutes.max(1) * 60),
            window: Duration::from_secs(config.window_minutes.max(1) * 60),
        }
    }

    pub async fn before_attempt(&self, identifier: &str) -> std::result::Result<(), LockoutError> {
        if !self.enabled {
            return Ok(());
        }

        let key = lockout_key(identifier);
        let Some(value) = self.store.get(&key).await.map_err(internal_lockout_error)? else {
            return Ok(());
        };

        let Some(until) = parse_locked_until(&value) else {
            return Ok(());
        };
        if until <= Utc::now() {
            let _ = self.store.clear(&key).await;
            return Ok(());
        }

        Err(LockoutError::LockedOut { until })
    }

    pub async fn record_failure(&self, identifier: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let key = lockout_key(identifier);
        let failures = self.store.increment_failures(&key, self.window).await?;
        if failures < self.lock_threshold() {
            return Ok(());
        }

        let until =
            Utc::now() + chrono::Duration::from_std(self.lockout_duration).map_err(Error::other)?;
        self.store
            .set_locked_until(&key, until, self.lockout_duration)
            .await?;

        if let Ok(events) = self.app.events() {
            events
                .dispatch(LoginLockedOutEvent {
                    identifier: identifier.to_string(),
                    locked_until: until,
                })
                .await?;
        }

        Ok(())
    }

    pub async fn record_success(&self, identifier: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        self.store.clear(&lockout_key(identifier)).await
    }

    fn lock_threshold(&self) -> u64 {
        self.max_failures.saturating_sub(1).max(1) as u64
    }
}

pub struct RuntimeLockoutStore {
    backend: RuntimeBackend,
}

impl RuntimeLockoutStore {
    pub fn from_app(app: &AppContext) -> Result<Self> {
        Ok(Self::new(app.resolve::<RuntimeBackend>()?.as_ref().clone()))
    }

    pub(crate) fn new(backend: RuntimeBackend) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl LockoutStore for RuntimeLockoutStore {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        self.backend.get_value(key).await
    }

    async fn increment_failures(&self, key: &str, window: Duration) -> Result<u64> {
        self.backend
            .incr_with_ttl(key, window.as_secs().max(1))
            .await
    }

    async fn set_locked_until(&self, key: &str, until: DateTime<Utc>, ttl: Duration) -> Result<()> {
        self.backend
            .set_value(
                key,
                &format!("{LOCKED_PREFIX}{}", until.timestamp()),
                ttl.as_secs().max(1),
            )
            .await
    }

    async fn clear(&self, key: &str) -> Result<()> {
        let _ = self.backend.del_key(key).await?;
        Ok(())
    }
}

fn lockout_key(identifier: &str) -> String {
    format!("foundry:lockout:{}", sha256_hex_str(identifier))
}

fn parse_locked_until(value: &str) -> Option<DateTime<Utc>> {
    let timestamp = value.strip_prefix(LOCKED_PREFIX)?.parse::<i64>().ok()?;
    DateTime::<Utc>::from_timestamp(timestamp, 0)
}

fn internal_lockout_error(error: Error) -> LockoutError {
    tracing::warn!(
        target: "foundry.auth.lockout",
        error = %error,
        "lockout store lookup failed; treating as a temporary lockout"
    );
    LockoutError::LockedOut {
        until: Utc::now() + chrono::Duration::seconds(1),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::Duration as ChronoDuration;

    use super::*;
    use crate::config::ConfigRepository;
    use crate::foundation::Container;
    use crate::validation::RuleRegistry;

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<std::collections::HashMap<String, String>>,
    }

    #[async_trait]
    impl LockoutStore for MemoryStore {
        async fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        async fn increment_failures(&self, key: &str, _window: Duration) -> Result<u64> {
            let mut values = self.values.lock().unwrap();
            let next = values
                .get(key)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
                + 1;
            values.insert(key.to_string(), next.to_string());
            Ok(next)
        }

        async fn set_locked_until(
            &self,
            key: &str,
            until: DateTime<Utc>,
            _ttl: Duration,
        ) -> Result<()> {
            self.values.lock().unwrap().insert(
                key.to_string(),
                format!("{LOCKED_PREFIX}{}", until.timestamp()),
            );
            Ok(())
        }

        async fn clear(&self, key: &str) -> Result<()> {
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn locks_on_fifth_attempt_window() {
        let store = Arc::new(MemoryStore::default());
        let throttle = LoginThrottle::from_config(
            test_app(),
            store,
            LockoutConfig {
                enabled: true,
                max_failures: 5,
                lockout_minutes: 15,
                window_minutes: 15,
            },
        );

        for _ in 0..4 {
            throttle.record_failure("person@example.com").await.unwrap();
        }

        let error = throttle
            .before_attempt("person@example.com")
            .await
            .unwrap_err();
        assert!(matches!(error, LockoutError::LockedOut { .. }));
    }

    #[tokio::test]
    async fn success_clears_counter() {
        let store = Arc::new(MemoryStore::default());
        let throttle = LoginThrottle::from_config(
            test_app(),
            store.clone(),
            LockoutConfig {
                enabled: true,
                max_failures: 5,
                lockout_minutes: 15,
                window_minutes: 15,
            },
        );

        throttle.record_failure("person@example.com").await.unwrap();
        throttle.record_success("person@example.com").await.unwrap();

        assert!(store
            .get(&lockout_key("person@example.com"))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn expired_lock_allows_next_attempt() {
        let store = Arc::new(MemoryStore::default());
        let app = test_app();
        let throttle = LoginThrottle::from_config(
            app,
            store.clone(),
            LockoutConfig {
                enabled: true,
                max_failures: 5,
                lockout_minutes: 15,
                window_minutes: 15,
            },
        );
        let key = lockout_key("person@example.com");
        store
            .set_locked_until(
                &key,
                Utc::now() - ChronoDuration::minutes(1),
                Duration::from_secs(60),
            )
            .await
            .unwrap();

        assert!(throttle.before_attempt("person@example.com").await.is_ok());
    }
}
