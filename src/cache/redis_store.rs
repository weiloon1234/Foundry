use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::foundation::Result;
use crate::redis::RedisManager;

use super::CacheStore;

pub struct RedisCacheStore {
    redis: Arc<RedisManager>,
    prefix: String,
}

impl RedisCacheStore {
    pub fn new(redis: Arc<RedisManager>, prefix: String) -> Self {
        Self { redis, prefix }
    }
}

#[async_trait]
impl CacheStore for RedisCacheStore {
    async fn get_raw(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.redis.connection().await?;
        let redis_key = self.redis.key(format!("{}{}", self.prefix, key));
        conn.get_optional::<String>(&redis_key).await
    }

    async fn put_raw(&self, key: &str, value: &str, ttl: Duration) -> Result<()> {
        let mut conn = self.redis.connection().await?;
        let redis_key = self.redis.key(format!("{}{}", self.prefix, key));
        conn.set_ex(&redis_key, value, ttl.as_secs()).await
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let mut conn = self.redis.connection().await?;
        let redis_key = self.redis.key(format!("{}{}", self.prefix, key));
        let deleted = conn.del(&redis_key).await?;
        Ok(deleted > 0)
    }

    async fn flush(&self) -> Result<()> {
        Err(crate::foundation::Error::message(
            "cache flush is not supported on Redis store; use specific forget() calls",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use uuid::Uuid;

    use super::RedisCacheStore;
    use crate::cache::CacheStore;
    use crate::config::ConfigRepository;
    use crate::redis::RedisManager;

    async fn redis_store(test_name: &str) -> Option<RedisCacheStore> {
        let url = std::env::var("FOUNDRY_REDIS_URL")
            .or_else(|_| std::env::var("REDIS_URL"))
            .unwrap_or_else(|_| "redis://127.0.0.1:6379/15".to_string());
        let namespace = format!("foundry-cache-tests:{test_name}:{}", Uuid::now_v7());
        let defaults = toml::from_str::<toml::Value>(&format!(
            r#"
                [redis]
                url = "{url}"
                namespace = "{namespace}"
            "#
        ))
        .unwrap();
        let config = ConfigRepository::with_env_overlay_and_defaults([defaults]).unwrap();
        let manager = match RedisManager::from_config(&config) {
            Ok(manager) => Arc::new(manager),
            Err(error) => {
                eprintln!("skipping redis cache test `{test_name}`: {error}");
                return None;
            }
        };
        match manager.connection().await {
            Ok(_) => Some(RedisCacheStore::new(manager, "cache:".to_string())),
            Err(error) => {
                eprintln!("skipping redis cache test `{test_name}`: {error}");
                None
            }
        }
    }

    #[tokio::test]
    async fn redis_get_distinguishes_missing_from_empty_value() {
        let Some(store) = redis_store("missing-vs-empty").await else {
            return;
        };

        assert_eq!(store.get_raw("missing").await.unwrap(), None);
        store
            .put_raw("empty", "", Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(store.get_raw("empty").await.unwrap(), Some(String::new()));
    }
}
