# cache

In-memory and Redis-backed caching (CacheManager)

[Back to index](../index.md)

## foundry::cache

```rust
struct CacheManager
  fn tags<I, S>(&self, tags: I) -> TaggedCache<'_>
  async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>
  async fn put<T: Serialize>( &self, key: &str, value: &T, ttl: Duration, ) -> Result<()>
  async fn remember<T, F, Fut>( &self, key: &str, ttl: Duration, f: F, ) -> Result<T>
  async fn forget(&self, key: &str) -> Result<bool>
  async fn flush(&self) -> Result<()>
struct MemoryCacheStore
  fn new(max_entries: usize) -> Self
struct RedisCacheStore
  fn new(redis: Arc<RedisManager>, prefix: String) -> Self
struct TaggedCache
  fn tag_names(&self) -> &[String]
  async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>
  async fn put<T: Serialize>( &self, key: &str, value: &T, ttl: Duration, ) -> Result<()>
  async fn remember<T, F, Fut>( &self, key: &str, ttl: Duration, callback: F, ) -> Result<T>
  async fn forget(&self, key: &str) -> Result<bool>
  async fn flush(&self) -> Result<()>
trait CacheStore
  fn get_raw<'life0, 'life1, 'async_trait>(
  fn put_raw<'life0, 'life1, 'life2, 'async_trait>(
  fn forget<'life0, 'life1, 'async_trait>(
  fn flush<'life0, 'async_trait>(
  fn get_control_raw<'life0, 'life1, 'async_trait>(
  fn put_control_raw<'life0, 'life1, 'life2, 'async_trait>(
```

## Notes

- Cache keys are validated before backend access; Redis nil/missing keys are distinct from backend failures.
- `remember()` uses local single-flight by default and can coordinate across workers with an opt-in distributed lock.
- `cache.error_mode = "fail_open"` logs backend I/O failures and continues, while validation, serialization, and callback errors remain strict.
