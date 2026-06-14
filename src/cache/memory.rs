use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::foundation::Result;

use super::CacheStore;

struct CacheEntry {
    value: String,
    expires_at: Instant,
}

pub struct MemoryCacheStore {
    entries: Arc<Mutex<HashMap<String, CacheEntry>>>,
    max_entries: usize,
}

impl MemoryCacheStore {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            max_entries,
        }
    }
}

#[async_trait]
impl CacheStore for MemoryCacheStore {
    async fn get_raw(&self, key: &str) -> Result<Option<String>> {
        let mut store = self.entries.lock().await;
        if let Some(entry) = store.get(key) {
            if Instant::now() < entry.expires_at {
                return Ok(Some(entry.value.clone()));
            }
            store.remove(key);
        }
        Ok(None)
    }

    async fn put_raw(&self, key: &str, value: &str, ttl: Duration) -> Result<()> {
        let mut store = self.entries.lock().await;

        if store.len() >= self.max_entries {
            let now = Instant::now();
            store.retain(|_, entry| now < entry.expires_at);

            // If still at capacity after evicting expired entries, remove one arbitrary entry
            if store.len() >= self.max_entries {
                if let Some(victim) = store.keys().next().cloned() {
                    store.remove(&victim);
                }
            }
        }

        store.insert(
            key.to_string(),
            CacheEntry {
                value: value.to_string(),
                expires_at: Instant::now() + ttl,
            },
        );
        Ok(())
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let mut store = self.entries.lock().await;
        Ok(store.remove(key).is_some())
    }

    async fn flush(&self) -> Result<()> {
        let mut store = self.entries.lock().await;
        store.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_returns_none_for_missing_key() {
        let store = MemoryCacheStore::new(100);
        let result = store.get_raw("missing").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn put_then_get_returns_value() {
        let store = MemoryCacheStore::new(100);
        store
            .put_raw("key", "value", Duration::from_secs(60))
            .await
            .unwrap();
        let result = store.get_raw("key").await.unwrap();
        assert_eq!(result, Some("value".to_string()));
    }

    #[tokio::test]
    async fn expired_entries_return_none() {
        let store = MemoryCacheStore::new(100);
        store
            .put_raw("key", "value", Duration::from_millis(1))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let result = store.get_raw("key").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn forget_removes_entry() {
        let store = MemoryCacheStore::new(100);
        store
            .put_raw("key", "value", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(store.forget("key").await.unwrap());
        assert!(store.get_raw("key").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn forget_returns_false_for_missing() {
        let store = MemoryCacheStore::new(100);
        assert!(!store.forget("missing").await.unwrap());
    }

    #[tokio::test]
    async fn flush_clears_all() {
        let store = MemoryCacheStore::new(100);
        store
            .put_raw("a", "1", Duration::from_secs(60))
            .await
            .unwrap();
        store
            .put_raw("b", "2", Duration::from_secs(60))
            .await
            .unwrap();
        store.flush().await.unwrap();
        assert!(store.get_raw("a").await.unwrap().is_none());
        assert!(store.get_raw("b").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn eviction_when_over_capacity() {
        let store = MemoryCacheStore::new(2);
        store
            .put_raw("a", "1", Duration::from_millis(1))
            .await
            .unwrap();
        store
            .put_raw("b", "2", Duration::from_millis(1))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        // Both expired — adding a third should evict them
        store
            .put_raw("c", "3", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(store.get_raw("a").await.unwrap().is_none());
        assert!(store.get_raw("b").await.unwrap().is_none());
        assert_eq!(store.get_raw("c").await.unwrap(), Some("3".to_string()));
    }
}
