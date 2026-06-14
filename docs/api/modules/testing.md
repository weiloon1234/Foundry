# testing

Test infrastructure: TestApp, TestClient, Factory

[Back to index](../index.md)

## foundry::testing

```rust
struct FactoryBuilder
  fn new() -> Self
  fn set<T, V>(self, column: Column<M, T>, value: V) -> Self
  fn count(self, n: usize) -> Self
  async fn create<E>(&self, executor: &E) -> Result<Vec<M>>
  async fn create_one<E>(&self, executor: &E) -> Result<M>
struct FactoryValue
  fn new<T, V>(column: Column<M, T>, value: V) -> Self
struct TestApp
  fn builder() -> TestAppBuilder
  fn app(&self) -> &AppContext
  fn client(&self) -> TestClient
  async fn seed_presence( &self, channel: &ChannelId, actor_id: &str, joined_at: i64, ) -> Result<()>
  async fn history_ttl(&self, channel: &ChannelId) -> Result<Option<u64>>
struct TestClient
  fn get(&self, path: &str) -> TestRequestBuilder
  fn post(&self, path: &str) -> TestRequestBuilder
  fn put(&self, path: &str) -> TestRequestBuilder
  fn patch(&self, path: &str) -> TestRequestBuilder
  fn delete(&self, path: &str) -> TestRequestBuilder
struct TestResponse
  fn status(&self) -> StatusCode
  fn header(&self, name: &str) -> Option<&str>
  fn json<T: DeserializeOwned>(&self) -> Result<T>
  fn text(&self) -> Result<String>
  fn bytes(&self) -> &[u8] ⓘ
trait Factory: Model
  fn definition() -> Vec<FactoryValue<Self>>
fn assert_safe_to_wipe(db_url: &str) -> Result<()>
```

