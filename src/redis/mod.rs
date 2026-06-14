use std::fmt;
use std::sync::Arc;

use ::redis::aio::MultiplexedConnection;
use ::redis::{FromRedisValue, ToRedisArgs};
use tokio::sync::OnceCell;

use crate::config::ConfigRepository;
use crate::foundation::{Error, Result};

pub(crate) fn namespaced_value(namespace: &str, suffix: &str) -> String {
    if namespace.trim().is_empty() {
        suffix.to_string()
    } else {
        format!("{namespace}:{suffix}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RedisKey {
    suffix: String,
    full: String,
}

impl RedisKey {
    fn new(namespace: &str, suffix: impl AsRef<str>) -> Self {
        let suffix = suffix.as_ref().to_string();
        let full = namespaced_value(namespace, &suffix);
        Self { suffix, full }
    }

    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    pub fn as_str(&self) -> &str {
        &self.full
    }
}

impl AsRef<str> for RedisKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for RedisKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RedisChannel {
    suffix: String,
    full: String,
}

impl RedisChannel {
    fn new(namespace: &str, suffix: impl AsRef<str>) -> Self {
        let suffix = suffix.as_ref().to_string();
        let full = namespaced_value(namespace, &suffix);
        Self { suffix, full }
    }

    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    pub fn as_str(&self) -> &str {
        &self.full
    }
}

impl AsRef<str> for RedisChannel {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for RedisChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct RedisManager {
    client: Option<::redis::Client>,
    namespace: Arc<str>,
    cached_connection: Arc<OnceCell<MultiplexedConnection>>,
}

impl RedisManager {
    pub fn from_config(config: &ConfigRepository) -> Result<Self> {
        let redis = config.redis()?;
        let app = config.app()?;
        let client = if redis.url.trim().is_empty() {
            None
        } else {
            Some(::redis::Client::open(redis.url.as_str()).map_err(Error::other)?)
        };

        // Auto-derive namespace from app.name:app.environment when using the
        // default value. This ensures multi-project safety on shared Redis
        // instances without explicit configuration.
        let namespace = if redis.namespace == "foundry" {
            format!("{}:{}", slugify(&app.name), app.environment)
        } else {
            redis.namespace
        };

        Ok(Self {
            client,
            namespace: Arc::<str>::from(namespace),
            cached_connection: Arc::new(OnceCell::new()),
        })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn key(&self, suffix: impl AsRef<str>) -> RedisKey {
        RedisKey::new(self.namespace(), suffix)
    }

    pub fn key_in_namespace(
        &self,
        namespace: impl AsRef<str>,
        suffix: impl AsRef<str>,
    ) -> RedisKey {
        RedisKey::new(namespace.as_ref(), suffix)
    }

    pub fn channel(&self, suffix: impl AsRef<str>) -> RedisChannel {
        RedisChannel::new(self.namespace(), suffix)
    }

    pub fn channel_in_namespace(
        &self,
        namespace: impl AsRef<str>,
        suffix: impl AsRef<str>,
    ) -> RedisChannel {
        RedisChannel::new(namespace.as_ref(), suffix)
    }

    pub async fn connection(&self) -> Result<RedisConnection> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| Error::message("redis is not configured"))?;

        let conn = self
            .cached_connection
            .get_or_try_init(|| async {
                client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)
            })
            .await?;

        Ok(RedisConnection {
            connection: conn.clone(),
        })
    }
}

#[derive(Debug)]
pub struct RedisConnection {
    connection: MultiplexedConnection,
}

impl RedisConnection {
    pub async fn get<T>(&mut self, key: &RedisKey) -> Result<T>
    where
        T: FromRedisValue,
    {
        ::redis::cmd("GET")
            .arg(key.as_str())
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn get_optional<T>(&mut self, key: &RedisKey) -> Result<Option<T>>
    where
        T: FromRedisValue,
    {
        ::redis::cmd("GET")
            .arg(key.as_str())
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn set<V>(&mut self, key: &RedisKey, value: V) -> Result<()>
    where
        V: ToRedisArgs,
    {
        let _: () = ::redis::cmd("SET")
            .arg(key.as_str())
            .arg(value)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)?;
        Ok(())
    }

    pub async fn set_ex<V>(&mut self, key: &RedisKey, value: V, seconds: u64) -> Result<()>
    where
        V: ToRedisArgs,
    {
        let _: () = ::redis::cmd("SETEX")
            .arg(key.as_str())
            .arg(seconds)
            .arg(value)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)?;
        Ok(())
    }

    pub async fn del(&mut self, key: &RedisKey) -> Result<usize> {
        ::redis::cmd("DEL")
            .arg(key.as_str())
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn del_many(&mut self, keys: &[&RedisKey]) -> Result<usize> {
        if keys.is_empty() {
            return Ok(0);
        }
        let mut cmd = ::redis::cmd("DEL");
        for key in keys {
            cmd.arg(key.as_str());
        }
        cmd.query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn exists(&mut self, key: &RedisKey) -> Result<bool> {
        let exists: i64 = ::redis::cmd("EXISTS")
            .arg(key.as_str())
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)?;
        Ok(exists > 0)
    }

    pub async fn expire(&mut self, key: &RedisKey, seconds: u64) -> Result<bool> {
        let updated: i64 = ::redis::cmd("EXPIRE")
            .arg(key.as_str())
            .arg(seconds)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)?;
        Ok(updated == 1)
    }

    pub async fn incr(&mut self, key: &RedisKey) -> Result<i64> {
        ::redis::cmd("INCR")
            .arg(key.as_str())
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn publish<V>(&mut self, channel: &RedisChannel, value: V) -> Result<usize>
    where
        V: ToRedisArgs,
    {
        ::redis::cmd("PUBLISH")
            .arg(channel.as_str())
            .arg(value)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn hget<T, F>(&mut self, key: &RedisKey, field: F) -> Result<T>
    where
        T: FromRedisValue,
        F: ToRedisArgs,
    {
        ::redis::cmd("HGET")
            .arg(key.as_str())
            .arg(field)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn hset<F, V>(&mut self, key: &RedisKey, field: F, value: V) -> Result<usize>
    where
        F: ToRedisArgs,
        V: ToRedisArgs,
    {
        ::redis::cmd("HSET")
            .arg(key.as_str())
            .arg(field)
            .arg(value)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn sadd<V>(&mut self, key: &RedisKey, value: V) -> Result<usize>
    where
        V: ToRedisArgs,
    {
        ::redis::cmd("SADD")
            .arg(key.as_str())
            .arg(value)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn srem<V>(&mut self, key: &RedisKey, value: V) -> Result<usize>
    where
        V: ToRedisArgs,
    {
        ::redis::cmd("SREM")
            .arg(key.as_str())
            .arg(value)
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn smembers<T>(&mut self, key: &RedisKey) -> Result<Vec<T>>
    where
        T: FromRedisValue,
    {
        ::redis::cmd("SMEMBERS")
            .arg(key.as_str())
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }
}

/// Convert an app name to a Redis-safe key prefix.
/// "Super AI" → "super_ai", "My App" → "my_app"
fn slugify(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !result.is_empty() {
            result.push('_');
            prev_underscore = true;
        }
    }
    result.trim_end_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::{namespaced_value, RedisManager};
    use crate::config::ConfigRepository;

    fn config_with_redis(url: &str, namespace: &str) -> ConfigRepository {
        let defaults = toml::from_str::<toml::Value>(&format!(
            r#"
                [redis]
                url = "{url}"
                namespace = "{namespace}"
            "#
        ))
        .unwrap();
        ConfigRepository::with_env_overlay_and_defaults([defaults]).unwrap()
    }

    #[test]
    fn keys_and_channels_are_namespaced() {
        let manager = RedisManager::from_config(&config_with_redis(
            "redis://127.0.0.1:6379/9",
            "foundry-tests",
        ))
        .unwrap();

        let key = manager.key("foo");
        let channel = manager.channel("events:user");

        assert_eq!(key.suffix(), "foo");
        assert_eq!(key.as_str(), "foundry-tests:foo");
        assert_eq!(channel.suffix(), "events:user");
        assert_eq!(channel.as_str(), "foundry-tests:events:user");
    }

    #[test]
    fn keys_and_channels_can_target_a_different_namespace_explicitly() {
        let manager = RedisManager::from_config(&config_with_redis(
            "redis://127.0.0.1:6379/9",
            "foundry-tests",
        ))
        .unwrap();

        let key = manager.key_in_namespace("analytics:prod", "daily:users");
        let channel = manager.channel_in_namespace("analytics:prod", "events");

        assert_eq!(key.as_str(), "analytics:prod:daily:users");
        assert_eq!(channel.as_str(), "analytics:prod:events");
    }

    #[test]
    fn empty_namespace_keeps_suffix_unchanged() {
        assert_eq!(
            namespaced_value("", "jobs:ready:default"),
            "jobs:ready:default"
        );
    }

    #[test]
    fn redis_url_db_index_is_preserved() {
        let manager = RedisManager::from_config(&config_with_redis(
            "redis://127.0.0.1:6379/12",
            "foundry-tests",
        ))
        .unwrap();

        let client = manager.client.as_ref().unwrap();
        assert_eq!(client.get_connection_info().redis_settings().db(), 12);
    }

    #[tokio::test]
    async fn connection_errors_when_redis_is_not_configured() {
        let manager = RedisManager::from_config(&config_with_redis("", "foundry-tests")).unwrap();

        let error = manager.connection().await.unwrap_err();
        assert_eq!(error.to_string(), "redis is not configured");
    }
}
