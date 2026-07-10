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
  fn from_builder(builder: AppBuilder) -> TestAppBuilder
  fn app(&self) -> &AppContext
  fn client(&self) -> TestClient
  async fn shutdown(self) -> Result<()>
  async fn seed_presence( &self, channel: &ChannelId, actor_id: &str, joined_at: i64, ) -> Result<()>
  async fn history_ttl(&self, channel: &ChannelId) -> Result<Option<u64>>
struct TestAppBuilder
  fn load_config_dir(self, path: impl Into<PathBuf>) -> Self
  fn register_provider<P>(self, provider: P) -> Self
  fn register_routes<F>(self, registrar: F) -> Self
  fn register_middleware(self, config: MiddlewareConfig) -> Self
  fn register_websocket_routes<F>(self, registrar: F) -> Self
  fn enable_observability(self) -> Self
  fn enable_public_observability(self) -> Self
  fn enable_observability_with(self, options: ObservabilityOptions) -> Self
  async fn build(self) -> Result<TestApp>
struct TestClient
  fn get(&self, path: &str) -> TestRequestBuilder
  fn post(&self, path: &str) -> TestRequestBuilder
  fn put(&self, path: &str) -> TestRequestBuilder
  fn patch(&self, path: &str) -> TestRequestBuilder
  fn delete(&self, path: &str) -> TestRequestBuilder
struct TestRequestBuilder
  fn header(self, name: &str, value: &str) -> Self
  fn bearer_auth(self, token: &str) -> Self
  fn body(self, body: impl Into<Vec<u8>>) -> Self
  fn text(self, body: impl Into<String>) -> Self
  fn json(self, value: &impl Serialize) -> Result<Self>
  async fn send(self) -> Result<TestResponse>
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
