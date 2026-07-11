# redis

Namespaced Redis wrapper (RedisManager, RedisConnection)

[Back to index](../index.md)

## foundry::redis

```rust
struct RedisChannel
  fn suffix(&self) -> &str
  fn as_str(&self) -> &str
struct RedisCommand
  fn arg<T>(&mut self, value: T) -> &mut Self
  fn key(&mut self, key: &RedisKey) -> &mut Self
struct RedisCommandBuilder
  fn arg<T>(self, value: T) -> Self
  fn key(self, key: &RedisKey) -> RedisCommand
struct RedisConnection
  async fn execute_command<T>(&mut self, command: &RedisCommand) -> Result<T>
  async fn execute_pipeline<T>( &mut self, pipeline: &RedisPipeline, ) -> Result<T>
  async fn execute_script<T>(&mut self, script: &RedisScript) -> Result<T>
  async fn get<T>(&mut self, key: &RedisKey) -> Result<T>
  async fn get_optional<T>(&mut self, key: &RedisKey) -> Result<Option<T>>
  async fn set<V>(&mut self, key: &RedisKey, value: V) -> Result<()>
  async fn set_ex<V>( &mut self, key: &RedisKey, value: V, seconds: u64, ) -> Result<()>
  async fn del(&mut self, key: &RedisKey) -> Result<usize>
  async fn del_many(&mut self, keys: &[&RedisKey]) -> Result<usize>
  async fn exists(&mut self, key: &RedisKey) -> Result<bool>
  async fn expire(&mut self, key: &RedisKey, seconds: u64) -> Result<bool>
  async fn incr(&mut self, key: &RedisKey) -> Result<i64>
  async fn publish<V>( &mut self, channel: &RedisChannel, value: V, ) -> Result<usize>
  async fn hget<T, F>(&mut self, key: &RedisKey, field: F) -> Result<T>
  async fn hset<F, V>( &mut self, key: &RedisKey, field: F, value: V, ) -> Result<usize>
  async fn sadd<V>(&mut self, key: &RedisKey, value: V) -> Result<usize>
  async fn srem<V>(&mut self, key: &RedisKey, value: V) -> Result<usize>
  async fn smembers<T>(&mut self, key: &RedisKey) -> Result<Vec<T>>
struct RedisKey
  fn suffix(&self) -> &str
  fn as_str(&self) -> &str
struct RedisManager
  fn from_config(config: &ConfigRepository) -> Result<Self>
  fn namespace(&self) -> &str
  fn connect_timeout(&self) -> Duration
  fn command_timeout(&self) -> Duration
  fn key(&self, suffix: impl AsRef<str>) -> RedisKey
  fn key_in_namespace( &self, namespace: impl AsRef<str>, suffix: impl AsRef<str>, ) -> RedisKey
  fn channel(&self, suffix: impl AsRef<str>) -> RedisChannel
  fn channel_in_namespace( &self, namespace: impl AsRef<str>, suffix: impl AsRef<str>, ) -> RedisChannel
  fn command(&self, name: &str) -> Result<RedisCommandBuilder>
  fn pipeline(&self) -> RedisPipeline
  fn transaction(&self) -> RedisPipeline
  fn script(&self, source: impl Into<Arc<str>>, key: &RedisKey) -> RedisScript
  async fn connection(&self) -> Result<RedisConnection>
struct RedisPipeline
  fn add(&mut self, command: RedisCommand) -> &mut Self
  fn add_ignored(&mut self, command: RedisCommand) -> &mut Self
  fn len(&self) -> usize
  fn is_empty(&self) -> bool
  fn is_transaction(&self) -> bool
struct RedisScript
  fn key(&mut self, key: &RedisKey) -> &mut Self
  fn arg<T>(&mut self, value: T) -> &mut Self
```
