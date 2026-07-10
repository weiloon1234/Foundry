use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::foundation::{Error, Result};
use crate::support::runtime::RuntimeBackend;

/// Distributed lock for coordinating concurrent workers.
///
/// ```ignore
/// if let Some(guard) = app.lock()?.acquire("payment:123", Duration::from_secs(30)).await? {
///     process_payment(123).await?;
///     // auto-releases on drop
/// }
/// ```
pub struct DistributedLock {
    backend: Arc<RuntimeBackend>,
}

impl DistributedLock {
    pub(crate) fn new(backend: Arc<RuntimeBackend>) -> Self {
        Self { backend }
    }

    /// Try to acquire a lock. Returns `Some(guard)` if acquired, `None` if already held.
    pub async fn acquire(&self, key: &str, ttl: Duration) -> Result<Option<LockGuard>> {
        validate_lock_key(key)?;
        let lock_key = format!("lock:{key}");
        self.acquire_validated_storage_key(&lock_key, ttl).await
    }

    /// Acquire an already-namespaced runtime storage key.
    ///
    /// Framework subsystems use this to preserve deployed coordination keys while
    /// sharing the same owner-token, renewal, and compare-and-delete semantics.
    pub(crate) async fn acquire_storage_key(
        &self,
        lock_key: &str,
        ttl: Duration,
    ) -> Result<Option<LockGuard>> {
        validate_lock_key(lock_key)?;
        self.acquire_validated_storage_key(lock_key, ttl).await
    }

    async fn acquire_validated_storage_key(
        &self,
        lock_key: &str,
        ttl: Duration,
    ) -> Result<Option<LockGuard>> {
        let owner = uuid::Uuid::now_v7().to_string();
        let ttl_secs = ttl.as_secs().max(1);

        let acquired = self
            .backend
            .set_nx_value(lock_key, &owner, ttl_secs)
            .await?;
        if acquired {
            Ok(Some(LockGuard {
                backend: self.backend.clone(),
                key: lock_key.to_string(),
                owner,
            }))
        } else {
            Ok(None)
        }
    }

    /// Block until a lock is acquired, with a timeout.
    pub async fn block(
        &self,
        key: &str,
        ttl: Duration,
        wait_timeout: Duration,
    ) -> Result<LockGuard> {
        validate_lock_key(key)?;
        let lock_key = format!("lock:{key}");
        let deadline = tokio::time::Instant::now() + wait_timeout;
        loop {
            if let Some(guard) = self.acquire_validated_storage_key(&lock_key, ttl).await? {
                return Ok(guard);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::message(format!(
                    "failed to acquire lock '{key}' within {}ms",
                    wait_timeout.as_millis()
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// Guard that releases the distributed lock on drop.
pub struct LockGuard {
    backend: Arc<RuntimeBackend>,
    key: String,
    owner: String,
}

impl LockGuard {
    /// Explicitly release the lock (instead of waiting for drop).
    pub async fn release(mut self) -> Result<bool> {
        let key = std::mem::take(&mut self.key);
        let owner = std::mem::take(&mut self.owner);
        if key.is_empty() {
            return Ok(false);
        }
        self.backend.del_if_value(&key, &owner).await
    }

    /// Extend the lock TTL, but only if this guard still owns the lock.
    pub async fn extend(&self, ttl: Duration) -> Result<bool> {
        if self.key.is_empty() {
            return Ok(false);
        }
        self.backend
            .expire_if_value(&self.key, &self.owner, ttl.as_secs().max(1))
            .await
    }

    /// Start a background heartbeat that periodically extends this lock while the
    /// returned guard is alive.
    pub fn start_heartbeat(&self, ttl: Duration, interval: Duration) -> LockHeartbeat {
        if self.key.is_empty() {
            return LockHeartbeat::inactive();
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                target: "foundry.lock",
                key = %self.key,
                "distributed lock heartbeat not started because no Tokio runtime is active"
            );
            return LockHeartbeat::inactive();
        };

        let backend = self.backend.clone();
        let key = self.key.clone();
        let owner = self.owner.clone();
        let ttl_secs = ttl.as_secs().max(1);
        let interval = if interval.is_zero() {
            Duration::from_millis(100)
        } else {
            interval
        };
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let task = handle.spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = tokio::time::sleep(interval) => {
                        match backend.expire_if_value(&key, &owner, ttl_secs).await {
                            Ok(true) => {}
                            Ok(false) => {
                                tracing::warn!(
                                    target: "foundry.lock",
                                    key = %key,
                                    "distributed lock heartbeat stopped because ownership was lost"
                                );
                                break;
                            }
                            Err(error) => {
                                tracing::warn!(
                                    target: "foundry.lock",
                                    key = %key,
                                    error = %error,
                                    "distributed lock heartbeat failed"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        });

        LockHeartbeat {
            stop: Some(stop_tx),
            task: Some(task),
        }
    }
}

/// Background task that keeps a distributed lock lease alive until dropped.
pub struct LockHeartbeat {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl LockHeartbeat {
    fn inactive() -> Self {
        Self {
            stop: None,
            task: None,
        }
    }
}

impl Drop for LockHeartbeat {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let backend = self.backend.clone();
        let key = std::mem::take(&mut self.key);
        let owner = std::mem::take(&mut self.owner);
        if !key.is_empty() {
            // Use try_current to avoid panic if the runtime is shutting down
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = backend.del_if_value(&key, &owner).await;
                });
            }
        }
    }
}

fn validate_lock_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(Error::message("lock key cannot be empty"));
    }
    if key.chars().any(char::is_control) {
        return Err(Error::message("lock key cannot contain control characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use uuid::Uuid;

    use super::{DistributedLock, LockGuard};
    use crate::support::runtime::RuntimeBackend;

    fn backend() -> Arc<RuntimeBackend> {
        Arc::new(RuntimeBackend::memory(&format!(
            "lock-test-{}",
            Uuid::now_v7()
        )))
    }

    async fn memory_expiration(backend: &RuntimeBackend, key: &str) -> Instant {
        match backend {
            RuntimeBackend::Memory(runtime) => runtime
                .unique_keys
                .lock()
                .await
                .get(key)
                .map(|(_, expires_at)| *expires_at)
                .expect("lock key should exist"),
            RuntimeBackend::Redis(_) => unreachable!("test uses memory runtime"),
        }
    }

    #[tokio::test]
    async fn lock_keys_are_validated() {
        let lock = DistributedLock::new(backend());

        let empty = match lock.acquire("", Duration::from_secs(1)).await {
            Ok(_) => panic!("empty lock key should fail"),
            Err(error) => error,
        };
        assert_eq!(empty.to_string(), "lock key cannot be empty");

        let control = match lock.acquire("bad\nkey", Duration::from_secs(1)).await {
            Ok(_) => panic!("control-character lock key should fail"),
            Err(error) => error,
        };
        assert_eq!(
            control.to_string(),
            "lock key cannot contain control characters"
        );

        assert!(lock
            .acquire("jobs:history/prune.lock", Duration::from_secs(1))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn storage_key_acquisition_preserves_exact_key() {
        let backend = backend();
        let lock = DistributedLock::new(backend.clone());
        let guard = lock
            .acquire_storage_key("schedule:nightly", Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();

        assert!(backend.key_exists("schedule:nightly").await.unwrap());
        assert!(!backend.key_exists("lock:schedule:nightly").await.unwrap());
        assert!(guard.release().await.unwrap());
    }

    #[tokio::test]
    async fn extend_only_refreshes_current_lock_owner() {
        let backend = backend();
        let lock = DistributedLock::new(backend.clone());
        let guard = lock
            .acquire("owned", Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        let stale = LockGuard {
            backend: guard.backend.clone(),
            key: guard.key.clone(),
            owner: "not-the-owner".to_string(),
        };

        assert!(!stale.extend(Duration::from_secs(5)).await.unwrap());
        assert!(guard.extend(Duration::from_secs(5)).await.unwrap());
        assert!(!stale.release().await.unwrap());
        assert!(guard.release().await.unwrap());
    }

    #[tokio::test]
    async fn heartbeat_extends_lock_until_dropped() {
        let backend = backend();
        let lock = DistributedLock::new(backend.clone());
        let guard = lock
            .acquire("heartbeat", Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        let initial = memory_expiration(&backend, &guard.key).await;
        let heartbeat = guard.start_heartbeat(Duration::from_secs(5), Duration::from_millis(10));

        tokio::time::sleep(Duration::from_millis(30)).await;
        let extended = memory_expiration(&backend, &guard.key).await;
        assert!(extended > initial + Duration::from_secs(3));

        drop(heartbeat);
        assert!(guard.release().await.unwrap());
    }
}
