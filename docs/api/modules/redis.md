# redis

Namespaced Redis wrapper (RedisManager, RedisConnection)

[Back to index](../index.md)

## foundry::redis

```rust
struct RedisChannel
  fn suffix(&self) -> &str
  fn as_str(&self) -> &str
struct RedisConnection
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
  fn key(&self, suffix: impl AsRef<str>) -> RedisKey
  fn key_in_namespace( &self, namespace: impl AsRef<str>, suffix: impl AsRef<str>, ) -> RedisKey
  fn channel(&self, suffix: impl AsRef<str>) -> RedisChannel
  fn channel_in_namespace( &self, namespace: impl AsRef<str>, suffix: impl AsRef<str>, ) -> RedisChannel
  async fn connection(&self) -> Result<RedisConnection>
```

