use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::foundation::Result;
use crate::redis::{RedisConnection, RedisKey, RedisManager};

use super::CacheStore;

const CACHE_INTERNAL_PREFIX: &str = "\u{001f}foundry-cache:";

pub struct RedisCacheStore {
    redis: Arc<RedisManager>,
    prefix: String,
}

impl RedisCacheStore {
    pub fn new(redis: Arc<RedisManager>, prefix: String) -> Self {
        Self { redis, prefix }
    }

    fn generation_key(&self) -> RedisKey {
        self.redis
            .key(format!("{}{CACHE_INTERNAL_PREFIX}generation", self.prefix))
    }

    fn data_key(&self, generation: i64, key: &str) -> RedisKey {
        self.redis.key(format!(
            "{}{CACHE_INTERNAL_PREFIX}data:{generation}:{key}",
            self.prefix
        ))
    }

    fn control_key(&self, key: &str) -> RedisKey {
        self.redis.key(format!(
            "{}{CACHE_INTERNAL_PREFIX}control:{key}",
            self.prefix
        ))
    }

    async fn current_generation(&self, connection: &mut RedisConnection) -> Result<i64> {
        Ok(connection
            .get_optional::<i64>(&self.generation_key())
            .await?
            .unwrap_or(0))
    }
}

#[async_trait]
impl CacheStore for RedisCacheStore {
    async fn get_raw(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.redis.connection().await?;
        let generation = self.current_generation(&mut conn).await?;
        let redis_key = self.data_key(generation, key);
        conn.get_optional::<String>(&redis_key).await
    }

    async fn put_raw(&self, key: &str, value: &str, ttl: Duration) -> Result<()> {
        let mut conn = self.redis.connection().await?;
        let generation = self.current_generation(&mut conn).await?;
        let redis_key = self.data_key(generation, key);
        conn.set_ex(&redis_key, value, ttl.as_secs()).await
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let mut conn = self.redis.connection().await?;
        let generation = self.current_generation(&mut conn).await?;
        let redis_key = self.data_key(generation, key);
        let deleted = conn.del(&redis_key).await?;
        Ok(deleted > 0)
    }

    async fn flush(&self) -> Result<()> {
        let mut conn = self.redis.connection().await?;
        conn.incr(&self.generation_key()).await?;
        Ok(())
    }

    async fn get_control_raw(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.redis.connection().await?;
        conn.get_optional::<String>(&self.control_key(key)).await
    }

    async fn put_control_raw(&self, key: &str, value: &str) -> Result<()> {
        let mut conn = self.redis.connection().await?;
        conn.set(&self.control_key(key), value).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use uuid::Uuid;

    use super::RedisCacheStore;
    use crate::cache::{CacheManager, CacheStore};
    use crate::config::{CacheConfig, ConfigRepository};
    use crate::redis::RedisManager;

    fn redis_url(test_name: &str) -> Option<String> {
        match std::env::var("FOUNDRY_REDIS_URL").or_else(|_| std::env::var("REDIS_URL")) {
            Ok(url) => Some(url),
            Err(_) => {
                eprintln!(
                    "skipping redis cache test `{test_name}`: set FOUNDRY_REDIS_URL or REDIS_URL"
                );
                None
            }
        }
    }

    async fn redis_manager(
        url: &str,
        namespace: &str,
        test_name: &str,
    ) -> Option<Arc<RedisManager>> {
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
            Ok(_) => Some(manager),
            Err(error) => {
                eprintln!("skipping redis cache test `{test_name}`: {error}");
                None
            }
        }
    }

    async fn redis_store(test_name: &str) -> Option<RedisCacheStore> {
        let url = redis_url(test_name)?;
        let namespace = format!("foundry-cache-tests:{test_name}:{}", Uuid::now_v7());
        let manager = redis_manager(&url, &namespace, test_name).await?;
        Some(RedisCacheStore::new(manager, "cache:".to_string()))
    }

    async fn separate_redis_managers(
        test_name: &str,
    ) -> Option<(Arc<RedisManager>, Arc<RedisManager>)> {
        let url = redis_url(test_name)?;
        let namespace = format!("foundry-cache-tests:{test_name}:{}", Uuid::now_v7());
        let first = redis_manager(&url, &namespace, test_name).await?;
        let second = redis_manager(&url, &namespace, test_name).await?;
        Some((first, second))
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

    #[tokio::test]
    async fn redis_flush_advances_shared_namespace_generation_only() {
        let Some((first_manager, second_manager)) =
            separate_redis_managers("generation-flush").await
        else {
            return;
        };
        let first = RedisCacheStore::new(first_manager.clone(), "cache:".to_string());
        let second = RedisCacheStore::new(second_manager, "cache:".to_string());
        let unrelated = RedisCacheStore::new(first_manager.clone(), "other-cache:".to_string());
        let outside_key = first_manager.key("outside-cache");

        first
            .put_raw("shared", "before", Duration::from_secs(60))
            .await
            .unwrap();
        unrelated
            .put_raw("kept", "unrelated", Duration::from_secs(60))
            .await
            .unwrap();
        first_manager
            .connection()
            .await
            .unwrap()
            .set(&outside_key, "outside")
            .await
            .unwrap();
        assert_eq!(
            second.get_raw("shared").await.unwrap(),
            Some("before".to_string())
        );

        second.flush().await.unwrap();

        assert_eq!(first.get_raw("shared").await.unwrap(), None);
        assert_eq!(
            unrelated.get_raw("kept").await.unwrap(),
            Some("unrelated".to_string())
        );
        assert_eq!(
            first_manager
                .connection()
                .await
                .unwrap()
                .get_optional::<String>(&outside_key)
                .await
                .unwrap(),
            Some("outside".to_string())
        );

        first
            .put_raw("shared", "after", Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(
            second.get_raw("shared").await.unwrap(),
            Some("after".to_string())
        );
    }

    #[tokio::test]
    async fn redis_tag_flush_is_visible_across_managers() {
        let Some((first_manager, second_manager)) = separate_redis_managers("tag-flush").await
        else {
            return;
        };
        let first = CacheManager::with_config(
            Arc::new(RedisCacheStore::new(first_manager, "cache:".to_string())),
            CacheConfig::default(),
            None,
        );
        let second = CacheManager::with_config(
            Arc::new(RedisCacheStore::new(second_manager, "cache:".to_string())),
            CacheConfig::default(),
            None,
        );

        first
            .tags(["users"])
            .put("profile", &"cached", Duration::from_secs(60))
            .await
            .unwrap();
        first
            .tags(["admins"])
            .put("profile", &"admin", Duration::from_secs(60))
            .await
            .unwrap();
        second.tags(["users"]).flush().await.unwrap();

        assert_eq!(
            first
                .tags(["users"])
                .get::<String>("profile")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            second
                .tags(["admins"])
                .get::<String>("profile")
                .await
                .unwrap(),
            Some("admin".to_string())
        );

        second.flush().await.unwrap();
        assert_eq!(
            first
                .tags(["admins"])
                .get::<String>("profile")
                .await
                .unwrap(),
            None
        );
    }
}
