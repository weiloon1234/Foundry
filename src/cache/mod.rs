mod memory;
mod redis_store;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::Mutex;

use crate::config::{CacheConfig, CacheErrorMode};
use crate::foundation::{Error, Result};
use crate::logging::{catch_async_panic, panic_payload_message};
use crate::support::lock::DistributedLock;

pub use memory::MemoryCacheStore;
pub use redis_store::RedisCacheStore;

/// Trait for cache store backends.
#[async_trait]
pub trait CacheStore: Send + Sync + 'static {
    async fn get_raw(&self, key: &str) -> Result<Option<String>>;
    async fn put_raw(&self, key: &str, value: &str, ttl: Duration) -> Result<()>;
    async fn forget(&self, key: &str) -> Result<bool>;
    async fn flush(&self) -> Result<()>;
}

/// Framework cache manager, accessible via `app.cache()`.
pub struct CacheManager {
    store: Arc<dyn CacheStore>,
    config: CacheConfig,
    distributed_lock: Option<Arc<DistributedLock>>,
    singleflight: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl CacheManager {
    #[cfg(test)]
    pub(crate) fn new(store: Arc<dyn CacheStore>) -> Self {
        Self::with_config(store, CacheConfig::default(), None)
    }

    pub(crate) fn with_config(
        store: Arc<dyn CacheStore>,
        config: CacheConfig,
        distributed_lock: Option<Arc<DistributedLock>>,
    ) -> Self {
        Self {
            store,
            config,
            distributed_lock,
            singleflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get a value from cache. Returns None if not found or expired.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.validate_key(key)?;
        let raw = match self.store.get_raw(key).await {
            Ok(raw) => raw,
            Err(error) => return self.handle_store_error("get", key, error, None),
        };
        match raw {
            Some(raw) => Ok(Some(serde_json::from_str(&raw).map_err(Error::other)?)),
            None => Ok(None),
        }
    }

    /// Store a value in cache with a TTL.
    pub async fn put<T: Serialize>(&self, key: &str, value: &T, ttl: Duration) -> Result<()> {
        self.validate_key(key)?;
        let raw = serde_json::to_string(value).map_err(Error::other)?;
        match self.store.put_raw(key, &raw, ttl).await {
            Ok(()) => Ok(()),
            Err(error) => self.handle_store_error("put", key, error, ()),
        }
    }

    /// Get from cache, or compute + store with TTL.
    pub async fn remember<T, F, Fut>(&self, key: &str, ttl: Duration, f: F) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        self.validate_key(key)?;
        if let Some(cached) = self.get::<T>(key).await? {
            return Ok(cached);
        }
        if !self.config.remember_singleflight {
            return self.remember_after_miss(key, ttl, f).await;
        }

        let singleflight = self.singleflight_lock(key).await;
        let guard = singleflight.lock().await;
        let result = async {
            if let Some(cached) = self.get::<T>(key).await? {
                return Ok(cached);
            }
            self.remember_after_miss(key, ttl, f).await
        }
        .await;
        drop(guard);
        self.remove_singleflight_lock(key, &singleflight).await;
        result
    }

    /// Remove a value from cache.
    pub async fn forget(&self, key: &str) -> Result<bool> {
        self.validate_key(key)?;
        match self.store.forget(key).await {
            Ok(removed) => Ok(removed),
            Err(error) => self.handle_store_error("forget", key, error, false),
        }
    }

    /// Clear all cached values.
    pub async fn flush(&self) -> Result<()> {
        match self.store.flush().await {
            Ok(()) => Ok(()),
            Err(error) => self.handle_store_error("flush", "*", error, ()),
        }
    }

    async fn remember_after_miss<T, F, Fut>(&self, key: &str, ttl: Duration, f: F) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        if !self.config.remember_distributed_lock {
            return self.compute_and_store(key, ttl, f).await;
        }

        let Some(lock) = self.distributed_lock.as_ref() else {
            tracing::warn!(
                target: "foundry.cache",
                key = key,
                "cache remember distributed lock requested but distributed lock service is unavailable"
            );
            return self.compute_and_store(key, ttl, f).await;
        };

        let lock_key = remember_lock_key(key);
        let lock_ttl = Duration::from_millis(self.config.remember_lock_ttl_ms.max(1));
        match lock.acquire(&lock_key, lock_ttl).await {
            Ok(Some(_guard)) => self.compute_and_store(key, ttl, f).await,
            Ok(None) => {
                let wait_timeout = Duration::from_millis(self.config.remember_lock_wait_timeout_ms);
                let poll = Duration::from_millis(self.config.remember_lock_poll_ms.max(1));
                if let Some(cached) = self.wait_for_remember_fill(key, wait_timeout, poll).await? {
                    return Ok(cached);
                }
                tracing::warn!(
                    target: "foundry.cache",
                    key = key,
                    wait_timeout_ms = self.config.remember_lock_wait_timeout_ms,
                    "cache remember distributed lock wait timed out; computing locally"
                );
                self.compute_and_store(key, ttl, f).await
            }
            Err(error) => {
                tracing::warn!(
                    target: "foundry.cache",
                    key = key,
                    error = %error,
                    "cache remember distributed lock failed; computing locally"
                );
                self.compute_and_store(key, ttl, f).await
            }
        }
    }

    async fn compute_and_store<T, F, Fut>(&self, key: &str, ttl: Duration, f: F) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let value = run_cache_remember_callback(key, f).await?;
        self.put(key, &value, ttl).await?;
        Ok(value)
    }

    async fn wait_for_remember_fill<T>(
        &self,
        key: &str,
        wait_timeout: Duration,
        poll: Duration,
    ) -> Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        let deadline = tokio::time::Instant::now() + wait_timeout;
        loop {
            if let Some(cached) = self.get::<T>(key).await? {
                return Ok(Some(cached));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(poll).await;
        }
    }

    async fn singleflight_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut locks = self.singleflight.lock().await;
        locks
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn remove_singleflight_lock(&self, key: &str, singleflight: &Arc<Mutex<()>>) {
        let mut locks = self.singleflight.lock().await;
        if locks
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, singleflight))
        {
            locks.remove(key);
        }
    }

    fn validate_key(&self, key: &str) -> Result<()> {
        validate_cache_key(key, self.config.key_max_length)
    }

    fn handle_store_error<T>(
        &self,
        operation: &'static str,
        key: &str,
        error: Error,
        fallback: T,
    ) -> Result<T> {
        if self.config.error_mode == CacheErrorMode::FailOpen && matches!(&error, Error::Other(_)) {
            tracing::warn!(
                target: "foundry.cache",
                operation = operation,
                key = key,
                error = %error,
                "cache backend operation failed; continuing because cache.error_mode is fail_open"
            );
            Ok(fallback)
        } else {
            Err(error)
        }
    }
}

fn validate_cache_key(key: &str, max_length: usize) -> Result<()> {
    if key.is_empty() {
        return Err(Error::message("cache key cannot be empty"));
    }
    if key.chars().any(char::is_control) {
        return Err(Error::message(
            "cache key cannot contain control characters",
        ));
    }
    if max_length > 0 && key.len() > max_length {
        return Err(Error::message(format!(
            "cache key exceeds maximum length of {max_length} bytes"
        )));
    }
    Ok(())
}

fn remember_lock_key(key: &str) -> String {
    format!("cache:remember:{key}")
}

async fn run_cache_remember_callback<T, F, Fut>(key: &str, callback: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match catch_async_panic(callback).await {
        Ok(result) => result,
        Err(panic) => Err(cache_remember_panic_error(key, panic)),
    }
}

fn cache_remember_panic_error(key: &str, panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.cache",
        key = key,
        panic = %message,
        "cache remember callback panicked"
    );
    Error::message(format!("cache remember callback panicked: {message}"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use uuid::Uuid;

    use super::{CacheManager, CacheStore, MemoryCacheStore};
    use crate::config::{CacheConfig, CacheErrorMode};
    use crate::foundation::Error;
    use crate::support::lock::DistributedLock;
    use crate::support::runtime::RuntimeBackend;

    fn manager() -> CacheManager {
        CacheManager::new(Arc::new(MemoryCacheStore::new(100)))
    }

    fn manager_with_config(config: CacheConfig) -> CacheManager {
        CacheManager::with_config(Arc::new(MemoryCacheStore::new(100)), config, None)
    }

    struct FailingStore;

    #[async_trait]
    impl CacheStore for FailingStore {
        async fn get_raw(&self, _key: &str) -> Result<Option<String>, Error> {
            Err(Error::other(anyhow::anyhow!("cache backend down")))
        }

        async fn put_raw(&self, _key: &str, _value: &str, _ttl: Duration) -> Result<(), Error> {
            Err(Error::other(anyhow::anyhow!("cache backend down")))
        }

        async fn forget(&self, _key: &str) -> Result<bool, Error> {
            Err(Error::other(anyhow::anyhow!("cache backend down")))
        }

        async fn flush(&self) -> Result<(), Error> {
            Err(Error::other(anyhow::anyhow!("cache backend down")))
        }
    }

    #[tokio::test]
    async fn remember_computes_and_stores_missing_value() {
        let cache = manager();

        let value = cache
            .remember("remember.success", Duration::from_secs(60), || async {
                Ok::<_, Error>("computed".to_string())
            })
            .await
            .unwrap();

        assert_eq!(value, "computed");
        assert_eq!(
            cache.get::<String>("remember.success").await.unwrap(),
            Some("computed".to_string())
        );
    }

    #[tokio::test]
    async fn remember_cache_hit_skips_callback() {
        let cache = manager();
        cache
            .put(
                "remember.hit",
                &"cached".to_string(),
                Duration::from_secs(60),
            )
            .await
            .unwrap();

        let value = cache
            .remember("remember.hit", Duration::from_secs(60), || async {
                panic!("remember callback should not run");
                #[allow(unreachable_code)]
                Ok::<_, Error>("computed".to_string())
            })
            .await
            .unwrap();

        assert_eq!(value, "cached");
    }

    #[tokio::test]
    async fn remember_callback_error_remains_unchanged() {
        let cache = manager();

        let error = cache
            .remember("remember.error", Duration::from_secs(60), || async {
                Err::<String, _>(Error::message("compute failed"))
            })
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "compute failed");
        assert!(cache
            .get::<String>("remember.error")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn remember_factory_panic_becomes_error() {
        let cache = manager();

        let error = cache
            .remember(
                "remember.factory-panic",
                Duration::from_secs(60),
                || -> std::future::Ready<crate::Result<String>> {
                    panic!("remember factory explode")
                },
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "cache remember callback panicked: remember factory explode"
        );
        assert!(cache
            .get::<String>("remember.factory-panic")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn remember_future_panic_becomes_error() {
        let cache = manager();

        let error = cache
            .remember("remember.future-panic", Duration::from_secs(60), || async {
                panic!("remember future explode");
                #[allow(unreachable_code)]
                Ok::<_, Error>("computed".to_string())
            })
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "cache remember callback panicked: remember future explode"
        );
        assert!(cache
            .get::<String>("remember.future-panic")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn public_cache_keys_are_validated() {
        let cache = manager_with_config(CacheConfig {
            key_max_length: 4,
            ..CacheConfig::default()
        });

        let empty = cache.get::<String>("").await.unwrap_err();
        assert_eq!(empty.to_string(), "cache key cannot be empty");

        let control = cache.get::<String>("bad\nkey").await.unwrap_err();
        assert_eq!(
            control.to_string(),
            "cache key cannot contain control characters"
        );

        let oversized = cache.get::<String>("abcde").await.unwrap_err();
        assert_eq!(
            oversized.to_string(),
            "cache key exceeds maximum length of 4 bytes"
        );

        manager()
            .put(
                "user:1/profile.cache-key",
                &"ok".to_string(),
                Duration::from_secs(60),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn strict_cache_errors_propagate_backend_failures() {
        let cache = CacheManager::with_config(Arc::new(FailingStore), CacheConfig::default(), None);

        let error = cache.get::<String>("strict").await.unwrap_err();
        assert!(error.to_string().contains("cache backend down"));
    }

    #[tokio::test]
    async fn fail_open_cache_errors_continue_for_backend_failures() {
        let cache = CacheManager::with_config(
            Arc::new(FailingStore),
            CacheConfig {
                error_mode: CacheErrorMode::FailOpen,
                ..CacheConfig::default()
            },
            None,
        );

        assert_eq!(cache.get::<String>("fail-open").await.unwrap(), None);
        cache
            .put("fail-open", &"value".to_string(), Duration::from_secs(60))
            .await
            .unwrap();
        assert!(!cache.forget("fail-open").await.unwrap());
        cache.flush().await.unwrap();
    }

    #[tokio::test]
    async fn fail_open_does_not_hide_callback_errors() {
        let cache = manager_with_config(CacheConfig {
            error_mode: CacheErrorMode::FailOpen,
            ..CacheConfig::default()
        });

        let error = cache
            .remember("callback-error", Duration::from_secs(60), || async {
                Err::<String, _>(Error::message("compute failed"))
            })
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "compute failed");
    }

    #[tokio::test]
    async fn remember_local_singleflight_runs_one_callback_per_cold_key() {
        let cache = Arc::new(manager());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..12 {
            let cache = cache.clone();
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .remember("singleflight", Duration::from_secs(60), || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Ok::<_, Error>("computed".to_string())
                        }
                    })
                    .await
                    .unwrap()
            }));
        }

        for task in tasks {
            assert_eq!(task.await.unwrap(), "computed");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn remember_distributed_lock_coordinates_cache_managers() {
        let namespace = format!("cache-distributed-{}", Uuid::now_v7());
        let backend = Arc::new(RuntimeBackend::memory(&namespace));
        let distributed_lock = Arc::new(DistributedLock::new(backend));
        let store = Arc::new(MemoryCacheStore::new(100));
        let config = CacheConfig {
            remember_singleflight: false,
            remember_distributed_lock: true,
            remember_lock_wait_timeout_ms: 500,
            remember_lock_poll_ms: 5,
            ..CacheConfig::default()
        };
        let first = Arc::new(CacheManager::with_config(
            store.clone(),
            config.clone(),
            Some(distributed_lock.clone()),
        ));
        let second = Arc::new(CacheManager::with_config(
            store,
            config,
            Some(distributed_lock),
        ));
        let calls = Arc::new(AtomicUsize::new(0));

        let first_task = {
            let cache = first.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                cache
                    .remember("distributed", Duration::from_secs(60), || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(40)).await;
                            Ok::<_, Error>("distributed-value".to_string())
                        }
                    })
                    .await
                    .unwrap()
            })
        };
        tokio::time::sleep(Duration::from_millis(5)).await;
        let second_task = {
            let cache = second.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                cache
                    .remember("distributed", Duration::from_secs(60), || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, Error>("fallback-value".to_string())
                        }
                    })
                    .await
                    .unwrap()
            })
        };

        assert_eq!(first_task.await.unwrap(), "distributed-value");
        assert_eq!(second_task.await.unwrap(), "distributed-value");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
