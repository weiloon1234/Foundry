use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use ::redis::aio::{ConnectionLike, MultiplexedConnection};
use ::redis::{AsyncConnectionConfig, FromRedisValue, RedisResult, ToRedisArgs, Value};
use tokio::sync::Mutex;

use crate::config::ConfigRepository;
use crate::foundation::{Error, Result};

pub(crate) fn namespaced_value(namespace: &str, suffix: &str) -> String {
    if namespace.trim().is_empty() {
        suffix.to_string()
    } else {
        format!("{namespace}:{suffix}")
    }
}

#[derive(Clone)]
pub(crate) struct RedisConnectionProvider {
    inner: Arc<RedisConnectionProviderInner>,
}

struct RedisConnectionProviderInner {
    client: Arc<::redis::Client>,
    connect_timeout: Duration,
    command_timeout: Duration,
    state: Mutex<RedisConnectionState>,
}

#[derive(Default)]
struct RedisConnectionState {
    generation: u64,
    cached: Option<CachedRedisConnection>,
}

struct CachedRedisConnection {
    generation: u64,
    connection: MultiplexedConnection,
}

impl fmt::Debug for RedisConnectionProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisConnectionProvider")
            .field("connect_timeout", &self.inner.connect_timeout)
            .field("command_timeout", &self.inner.command_timeout)
            .finish_non_exhaustive()
    }
}

impl RedisConnectionProvider {
    pub(crate) fn new(
        client: Arc<::redis::Client>,
        connect_timeout: Duration,
        command_timeout: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(RedisConnectionProviderInner {
                client,
                connect_timeout,
                command_timeout,
                state: Mutex::new(RedisConnectionState::default()),
            }),
        }
    }

    pub(crate) fn client(&self) -> &Arc<::redis::Client> {
        &self.inner.client
    }

    pub(crate) fn connect_timeout(&self) -> Duration {
        self.inner.connect_timeout
    }

    pub(crate) fn command_timeout(&self) -> Duration {
        self.inner.command_timeout
    }

    fn connection_config(&self) -> AsyncConnectionConfig {
        AsyncConnectionConfig::new()
            .set_connection_timeout(Some(self.connect_timeout()))
            .set_response_timeout(Some(self.command_timeout()))
    }

    async fn cached_connection(&self) -> RedisResult<(MultiplexedConnection, u64)> {
        let mut state = self.inner.state.lock().await;
        if let Some(cached) = &state.cached {
            return Ok((cached.connection.clone(), cached.generation));
        }

        let connection = self
            .inner
            .client
            .get_multiplexed_async_connection_with_config(&self.connection_config())
            .await?;
        state.generation = state.generation.wrapping_add(1).max(1);
        let generation = state.generation;
        state.cached = Some(CachedRedisConnection {
            generation,
            connection: connection.clone(),
        });
        Ok((connection, generation))
    }

    async fn invalidate(&self, generation: u64) {
        let mut state = self.inner.state.lock().await;
        if state
            .cached
            .as_ref()
            .is_some_and(|cached| cached.generation == generation)
        {
            state.cached = None;
        }
    }

    async fn command(&self, command: &::redis::Cmd) -> RedisResult<Value> {
        let (mut connection, generation) = self.cached_connection().await?;
        let result = connection.req_packed_command(command).await;
        self.handle_request_result(generation, result).await
    }

    async fn pipeline(
        &self,
        pipeline: &::redis::Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisResult<Vec<Value>> {
        let (mut connection, generation) = self.cached_connection().await?;
        let result = connection
            .req_packed_commands(pipeline, offset, count)
            .await;
        self.handle_request_result(generation, result).await
    }

    async fn handle_request_result<T>(
        &self,
        generation: u64,
        result: RedisResult<T>,
    ) -> RedisResult<T> {
        if result.as_ref().is_err_and(redis_error_requires_reconnect) {
            self.invalidate(generation).await;
        }
        result
    }

    pub(crate) async fn connection(&self) -> RedisResult<ManagedRedisConnection> {
        self.cached_connection().await?;
        Ok(ManagedRedisConnection {
            provider: self.clone(),
        })
    }
}

fn redis_error_requires_reconnect(error: &::redis::RedisError) -> bool {
    error.is_connection_dropped() || error.is_timeout() || error.is_unrecoverable_error()
}

#[derive(Clone)]
pub(crate) struct ManagedRedisConnection {
    provider: RedisConnectionProvider,
}

impl fmt::Debug for ManagedRedisConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedRedisConnection")
            .finish_non_exhaustive()
    }
}

impl ConnectionLike for ManagedRedisConnection {
    fn req_packed_command<'a>(
        &'a mut self,
        command: &'a ::redis::Cmd,
    ) -> ::redis::RedisFuture<'a, Value> {
        Box::pin(async move { self.provider.command(command).await })
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        pipeline: &'a ::redis::Pipeline,
        offset: usize,
        count: usize,
    ) -> ::redis::RedisFuture<'a, Vec<Value>> {
        Box::pin(async move { self.provider.pipeline(pipeline, offset, count).await })
    }

    fn get_db(&self) -> i64 {
        self.provider
            .client()
            .get_connection_info()
            .redis_settings()
            .db()
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

/// Builds a low-level Redis command until its first typed key makes it executable.
///
/// Prefix arguments support commands whose first key is not their first
/// argument, such as `XREAD` and `XGROUP`. Calling [`RedisCommandBuilder::key`]
/// consumes the builder so a command without a namespaced key cannot execute.
pub struct RedisCommandBuilder {
    name: String,
    inner: ::redis::Cmd,
}

impl fmt::Debug for RedisCommandBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisCommandBuilder")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl RedisCommandBuilder {
    fn new(name: &str) -> Result<Self> {
        validate_redis_command_name(name)?;
        Ok(Self {
            name: name.to_ascii_uppercase(),
            inner: ::redis::cmd(name),
        })
    }

    pub fn arg<T>(mut self, value: T) -> Self
    where
        T: ToRedisArgs,
    {
        self.inner.arg(value);
        self
    }

    pub fn key(mut self, key: &RedisKey) -> RedisCommand {
        self.inner.arg(key.as_str());
        RedisCommand {
            name: self.name,
            inner: self.inner,
            key_count: 1,
        }
    }
}

/// An executable low-level Redis command containing a typed, namespaced key.
///
/// Additional command values use [`RedisCommand::arg`], while additional keys
/// must use [`RedisCommand::key`] so key namespace intent remains explicit.
#[derive(Clone)]
pub struct RedisCommand {
    name: String,
    inner: ::redis::Cmd,
    key_count: usize,
}

impl fmt::Debug for RedisCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisCommand")
            .field("name", &self.name)
            .field("key_count", &self.key_count)
            .finish_non_exhaustive()
    }
}

impl RedisCommand {
    pub fn arg<T>(&mut self, value: T) -> &mut Self
    where
        T: ToRedisArgs,
    {
        self.inner.arg(value);
        self
    }

    pub fn key(&mut self, key: &RedisKey) -> &mut Self {
        self.inner.arg(key.as_str());
        self.key_count += 1;
        self
    }
}

/// A namespace-safe collection of low-level Redis commands.
#[derive(Clone)]
pub struct RedisPipeline {
    inner: ::redis::Pipeline,
}

impl fmt::Debug for RedisPipeline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisPipeline")
            .field("len", &self.inner.len())
            .field("atomic", &self.inner.is_transaction())
            .finish_non_exhaustive()
    }
}

impl Default for RedisPipeline {
    fn default() -> Self {
        Self {
            inner: ::redis::Pipeline::new(),
        }
    }
}

impl RedisPipeline {
    fn transaction() -> Self {
        let mut pipeline = ::redis::Pipeline::new();
        pipeline.atomic();
        Self { inner: pipeline }
    }

    pub fn add(&mut self, command: RedisCommand) -> &mut Self {
        self.inner.add_command(command.inner);
        self
    }

    pub fn add_ignored(&mut self, command: RedisCommand) -> &mut Self {
        self.inner.add_command(command.inner).ignore();
        self
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn is_transaction(&self) -> bool {
        self.inner.is_transaction()
    }
}

/// A Lua script invocation that receives Redis keys only through [`RedisKey`].
///
/// Script source is trusted application code: use `KEYS` for every Redis key
/// instead of hard-coding or passing keys through script arguments.
#[derive(Clone)]
pub struct RedisScript {
    source: Arc<str>,
    keys: Vec<String>,
    args: Vec<Vec<u8>>,
}

impl fmt::Debug for RedisScript {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisScript")
            .field("key_count", &self.keys.len())
            .field("arg_count", &self.args.len())
            .finish_non_exhaustive()
    }
}

impl RedisScript {
    fn new(source: impl Into<Arc<str>>, key: &RedisKey) -> Self {
        Self {
            source: source.into(),
            keys: vec![key.as_str().to_string()],
            args: Vec::new(),
        }
    }

    pub fn key(&mut self, key: &RedisKey) -> &mut Self {
        self.keys.push(key.as_str().to_string());
        self
    }

    pub fn arg<T>(&mut self, value: T) -> &mut Self
    where
        T: ToRedisArgs,
    {
        self.args.extend(value.to_redis_args());
        self
    }
}

fn validate_redis_command_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(Error::message(format!(
            "invalid Redis command name `{}`",
            name.escape_debug()
        )));
    }
    Ok(())
}

#[derive(Clone)]
pub struct RedisManager {
    provider: Option<RedisConnectionProvider>,
    namespace: Arc<str>,
    connect_timeout: Duration,
    command_timeout: Duration,
}

impl fmt::Debug for RedisManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisManager")
            .field("configured", &self.provider.is_some())
            .field("namespace", &self.namespace)
            .field("connect_timeout", &self.connect_timeout)
            .field("command_timeout", &self.command_timeout)
            .finish()
    }
}

impl RedisManager {
    pub fn from_config(config: &ConfigRepository) -> Result<Self> {
        let redis = config.redis()?;
        let app = config.app()?;
        let connect_timeout = redis.connect_timeout();
        let command_timeout = redis.command_timeout();
        let client = if redis.url.trim().is_empty() {
            None
        } else {
            Some(Arc::new(
                ::redis::Client::open(redis.url.as_str()).map_err(Error::other)?,
            ))
        };
        let provider = client.as_ref().map(|client| {
            RedisConnectionProvider::new(client.clone(), connect_timeout, command_timeout)
        });

        // Auto-derive namespace from app.name:app.environment when using the
        // default value. This ensures multi-project safety on shared Redis
        // instances without explicit configuration.
        let namespace = if redis.namespace == "foundry" {
            format!("{}:{}", slugify(&app.name), app.environment)
        } else {
            redis.namespace
        };

        Ok(Self {
            provider,
            namespace: Arc::<str>::from(namespace),
            connect_timeout,
            command_timeout,
        })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    pub fn command_timeout(&self) -> Duration {
        self.command_timeout
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

    pub fn command(&self, name: &str) -> Result<RedisCommandBuilder> {
        RedisCommandBuilder::new(name)
    }

    pub fn pipeline(&self) -> RedisPipeline {
        RedisPipeline::default()
    }

    pub fn transaction(&self) -> RedisPipeline {
        RedisPipeline::transaction()
    }

    pub fn script(&self, source: impl Into<Arc<str>>, key: &RedisKey) -> RedisScript {
        RedisScript::new(source, key)
    }

    pub async fn connection(&self) -> Result<RedisConnection> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| Error::message("redis is not configured"))?;
        let connection = provider.connection().await.map_err(Error::other)?;
        Ok(RedisConnection { connection })
    }
}

/// A reconnecting Redis command handle.
///
/// Foundry never retries a failed command because Redis may already have
/// applied a mutation before the failure became observable. Connection and
/// timeout failures invalidate the cached transport so the next command opens
/// a fresh connection.
#[derive(Debug)]
pub struct RedisConnection {
    connection: ManagedRedisConnection,
}

impl RedisConnection {
    pub async fn execute_command<T>(&mut self, command: &RedisCommand) -> Result<T>
    where
        T: FromRedisValue,
    {
        command
            .inner
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn execute_pipeline<T>(&mut self, pipeline: &RedisPipeline) -> Result<T>
    where
        T: FromRedisValue,
    {
        pipeline
            .inner
            .query_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

    pub async fn execute_script<T>(&mut self, script: &RedisScript) -> Result<T>
    where
        T: FromRedisValue,
    {
        let redis_script = ::redis::Script::new(&script.source);
        let mut invocation = redis_script.prepare_invoke();
        for key in &script.keys {
            invocation.key(key);
        }
        for arg in &script.args {
            invocation.arg(arg.as_slice());
        }
        invocation
            .invoke_async(&mut self.connection)
            .await
            .map_err(Error::other)
    }

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
    use std::future::pending;
    use std::time::{Duration, Instant};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use uuid::Uuid;

    use super::{namespaced_value, RedisManager};
    use crate::config::ConfigRepository;
    use crate::foundation::Error;

    fn config_with_redis(url: &str, namespace: &str) -> ConfigRepository {
        config_with_redis_timeouts(url, namespace, 5_000, 5_000)
    }

    fn config_with_redis_timeouts(
        url: &str,
        namespace: &str,
        connect_timeout_ms: u64,
        command_timeout_ms: u64,
    ) -> ConfigRepository {
        let defaults = toml::from_str::<toml::Value>(&format!(
            r#"
                [redis]
                url = "{url}"
                namespace = "{namespace}"
                connect_timeout_ms = {connect_timeout_ms}
                command_timeout_ms = {command_timeout_ms}
            "#
        ))
        .unwrap();
        ConfigRepository::with_env_overlay_and_defaults([defaults]).unwrap()
    }

    async fn fake_redis_connection(listener: &TcpListener) -> TcpStream {
        let (mut stream, _) = listener.accept().await.unwrap();

        // A plain RESP2 connection starts with two ignored CLIENT SETINFO
        // commands. Their replies may be written before the requests arrive;
        // TCP preserves their order for the setup pipeline.
        stream.write_all(b"+OK\r\n+OK\r\n").await.unwrap();
        stream
    }

    async fn read_until_command(stream: &mut TcpStream, command: &str) {
        let marker = format!("${}\r\n{}\r\n", command.len(), command).into_bytes();
        let mut received = Vec::new();
        let mut buffer = [0_u8; 1_024];

        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "client closed before sending {command}");
            received.extend_from_slice(&buffer[..read]);
            if received
                .windows(marker.len())
                .any(|window| window == marker)
            {
                return;
            }
        }
    }

    fn assert_redis_timeout(error: Error) {
        let Error::Other(error) = error else {
            panic!("expected wrapped Redis timeout, got {error}");
        };
        let redis_error = error
            .downcast_ref::<::redis::RedisError>()
            .expect("expected a Redis error");
        assert!(redis_error.is_timeout(), "expected timeout: {redis_error}");
    }

    async fn local_redis_manager(test_name: &str) -> Option<RedisManager> {
        let url = std::env::var("FOUNDRY_REDIS_URL")
            .or_else(|_| std::env::var("REDIS_URL"))
            .unwrap_or_else(|_| "redis://127.0.0.1:6379/15".to_string());
        let namespace = format!("foundry-redis-tests:{test_name}:{}", Uuid::now_v7());
        let manager = match RedisManager::from_config(&config_with_redis_timeouts(
            &url, &namespace, 1_000, 1_000,
        )) {
            Ok(manager) => manager,
            Err(error) => {
                eprintln!("skipping local Redis test `{test_name}`: {error}");
                return None;
            }
        };

        match manager.connection().await {
            Ok(_) => Some(manager),
            Err(error) => {
                eprintln!("skipping local Redis test `{test_name}`: {error}");
                None
            }
        }
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
    fn configured_timeouts_are_exposed() {
        let manager = RedisManager::from_config(&config_with_redis_timeouts(
            "redis://127.0.0.1:6379/9",
            "foundry-tests",
            1_750,
            2_250,
        ))
        .unwrap();

        assert_eq!(manager.connect_timeout(), Duration::from_millis(1_750));
        assert_eq!(manager.command_timeout(), Duration::from_millis(2_250));
    }

    #[test]
    fn low_level_commands_pipelines_and_scripts_keep_typed_keys_namespaced() {
        let manager = RedisManager::from_config(&config_with_redis(
            "redis://127.0.0.1:6379/9",
            "foundry-tests",
        ))
        .unwrap();
        let leaderboard = manager.key("leaderboard");
        let destination = manager.key("leaderboard:top");

        let mut add_score = manager.command("ZADD").unwrap().key(&leaderboard);
        add_score.arg(42).arg("player-1");
        let packed = String::from_utf8(add_score.inner.get_packed_command()).unwrap();
        assert!(packed.contains("foundry-tests:leaderboard"));
        assert!(!packed.contains("foundry-tests:foundry-tests"));

        let mut union = manager.command("ZUNIONSTORE").unwrap().key(&destination);
        union.arg(1).key(&leaderboard);
        let packed = String::from_utf8(union.inner.get_packed_command()).unwrap();
        assert!(packed.contains("foundry-tests:leaderboard:top"));
        assert!(packed.contains("foundry-tests:leaderboard"));

        let mut pipeline = manager.transaction();
        pipeline.add_ignored(add_score).add(union);
        let packed = String::from_utf8(pipeline.inner.get_packed_pipeline()).unwrap();
        assert!(pipeline.is_transaction());
        assert_eq!(pipeline.len(), 2);
        assert!(packed.contains("foundry-tests:leaderboard"));
        assert!(packed.contains("foundry-tests:leaderboard:top"));

        let mut script = manager.script(
            "return redis.call('ZCARD', KEYS[1]) + redis.call('ZCARD', KEYS[2])",
            &leaderboard,
        );
        script.key(&destination).arg("private-argument");
        assert_eq!(
            script.keys,
            vec![
                "foundry-tests:leaderboard".to_string(),
                "foundry-tests:leaderboard:top".to_string(),
            ]
        );
        assert_eq!(script.args, vec![b"private-argument".to_vec()]);

        assert!(!format!("{script:?}").contains("private-argument"));
        assert!(!format!("{script:?}").contains("redis.call"));
        assert!(!format!("{pipeline:?}").contains("player-1"));

        let mut stream_read = manager
            .command("XREAD")
            .unwrap()
            .arg("COUNT")
            .arg(10)
            .arg("STREAMS")
            .key(&leaderboard);
        stream_read.arg("0-0");
        let mut expected = ::redis::cmd("XREAD");
        expected
            .arg("COUNT")
            .arg(10)
            .arg("STREAMS")
            .arg("foundry-tests:leaderboard")
            .arg("0-0");
        assert_eq!(
            stream_read.inner.get_packed_command(),
            expected.get_packed_command()
        );
    }

    #[test]
    fn invalid_low_level_command_names_are_rejected() {
        let manager = RedisManager::from_config(&config_with_redis(
            "redis://127.0.0.1:6379/9",
            "foundry-tests",
        ))
        .unwrap();
        for name in ["", "GET SET", "GET\n"] {
            assert!(manager.command(name).is_err(), "accepted `{name:?}`");
        }
        assert!(manager.command("GET").is_ok());
    }

    #[test]
    fn redis_url_db_index_is_preserved() {
        let manager = RedisManager::from_config(&config_with_redis(
            "redis://127.0.0.1:6379/12",
            "foundry-tests",
        ))
        .unwrap();

        let client = manager.provider.as_ref().unwrap().client();
        assert_eq!(client.get_connection_info().redis_settings().db(), 12);
    }

    #[tokio::test]
    async fn connection_errors_when_redis_is_not_configured() {
        let manager = RedisManager::from_config(&config_with_redis("", "foundry-tests")).unwrap();

        let error = manager.connection().await.unwrap_err();
        assert_eq!(error.to_string(), "redis is not configured");
    }

    #[tokio::test]
    async fn connection_setup_is_bounded_by_the_configured_connect_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            pending::<()>().await;
        });
        let manager = RedisManager::from_config(&config_with_redis_timeouts(
            &format!("redis://{address}/"),
            "foundry-tests",
            40,
            500,
        ))
        .unwrap();

        let started = Instant::now();
        let error = tokio::time::timeout(Duration::from_secs(1), manager.connection())
            .await
            .expect("connection attempt exceeded its outer test bound")
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_redis_timeout(error);
        server.abort();
    }

    #[tokio::test]
    async fn commands_are_bounded_by_the_configured_response_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut stream = fake_redis_connection(&listener).await;
            read_until_command(&mut stream, "GET").await;
            pending::<()>().await;
        });
        let manager = RedisManager::from_config(&config_with_redis_timeouts(
            &format!("redis://{address}/"),
            "foundry-tests",
            500,
            40,
        ))
        .unwrap();
        let key = manager.key("slow");
        let mut connection = manager.connection().await.unwrap();

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            connection.get_optional::<String>(&key),
        )
        .await
        .expect("command exceeded its outer test bound")
        .unwrap_err();
        assert_redis_timeout(error);
        server.abort();
    }

    #[tokio::test]
    async fn a_dropped_cached_connection_is_replaced_for_the_next_command() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut first = fake_redis_connection(&listener).await;
            read_until_command(&mut first, "GET").await;
            drop(first);

            let mut second = fake_redis_connection(&listener).await;
            read_until_command(&mut second, "SET").await;
            second.write_all(b"+OK\r\n").await.unwrap();
        });
        let manager = RedisManager::from_config(&config_with_redis_timeouts(
            &format!("redis://{address}/"),
            "foundry-tests",
            500,
            500,
        ))
        .unwrap();
        let key = manager.key("reconnect");
        let mut connection = manager.connection().await.unwrap();

        let first = tokio::time::timeout(
            Duration::from_secs(1),
            connection.get_optional::<String>(&key),
        )
        .await
        .expect("first command exceeded its outer test bound");
        assert!(first.is_err(), "dropped connection unexpectedly succeeded");

        tokio::time::timeout(Duration::from_secs(1), connection.set(&key, "reconnected"))
            .await
            .expect("reconnect exceeded its outer test bound")
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn low_level_leaderboard_pipeline_and_script_execute_against_local_redis() {
        let Some(manager) = local_redis_manager("low-level").await else {
            return;
        };
        let leaderboard = manager.key("leaderboard");
        let counter = manager.key("counter");
        let stream = manager.key("events");
        let outside_namespace = format!("{}:outside", manager.namespace());
        let outside = manager.key_in_namespace(outside_namespace, "leaderboard");
        let mut connection = manager.connection().await.unwrap();
        connection
            .del_many(&[&leaderboard, &counter, &stream, &outside])
            .await
            .unwrap();

        let mut alice = manager.command("ZADD").unwrap().key(&leaderboard);
        alice.arg(10).arg("alice");
        let mut bob = manager.command("ZADD").unwrap().key(&leaderboard);
        bob.arg(20).arg("bob");
        assert_eq!(
            connection.execute_command::<usize>(&alice).await.unwrap(),
            1
        );
        assert_eq!(connection.execute_command::<usize>(&bob).await.unwrap(), 1);

        let mut ranking = manager.command("ZREVRANGE").unwrap().key(&leaderboard);
        ranking.arg(0).arg(-1).arg("WITHSCORES");
        assert_eq!(
            connection
                .execute_command::<Vec<String>>(&ranking)
                .await
                .unwrap(),
            vec![
                "bob".to_string(),
                "20".to_string(),
                "alice".to_string(),
                "10".to_string(),
            ]
        );

        let mut reset = manager.command("SET").unwrap().key(&counter);
        reset.arg(0);
        let mut increment = manager.command("INCRBY").unwrap().key(&counter);
        increment.arg(2);
        let mut transaction = manager.transaction();
        transaction.add_ignored(reset).add(increment);
        assert_eq!(
            connection
                .execute_pipeline::<(i64,)>(&transaction)
                .await
                .unwrap(),
            (2,)
        );

        let mut script = manager.script("return redis.call('INCRBY', KEYS[1], ARGV[1])", &counter);
        script.arg(3);
        assert_eq!(connection.execute_script::<i64>(&script).await.unwrap(), 5);

        let mut stream_group = manager
            .command("XGROUP")
            .unwrap()
            .arg("CREATE")
            .key(&stream);
        stream_group.arg("workers").arg("0").arg("MKSTREAM");
        assert_eq!(
            connection
                .execute_command::<String>(&stream_group)
                .await
                .unwrap(),
            "OK"
        );
        assert_eq!(
            connection.get_optional::<String>(&outside).await.unwrap(),
            None
        );

        connection
            .del_many(&[&leaderboard, &counter, &stream])
            .await
            .unwrap();
    }
}
