# testing

Test infrastructure: TestApp, clients/fakes, assertions, and model factories

[Back to index](../index.md)

## foundry::testing

```rust
enum NotificationDelivery { Immediate, Queued }
struct ClockFake
  fn new(now: DateTime, timezone: Timezone) -> Self
  fn utc(now: DateTime) -> Self
  fn now(&self) -> DateTime
  fn set(&self, now: DateTime) -> &Self
  fn advance_seconds(&self, seconds: i64) -> &Self
  fn rewind_seconds(&self, seconds: i64) -> &Self
  fn assert_now(&self, expected: DateTime) -> &Self
struct CommandIoFake
  fn new() -> Self
  fn with_input(self, value: impl Into<String>) -> Self
  fn push_input(&self, value: impl Into<String>) -> &Self
  fn stdout(&self) -> String
  fn stderr(&self) -> String
  fn clear(&self) -> &Self
  fn assert_stdout(&self, expected: &str) -> &Self
  fn assert_stdout_contains(&self, expected: &str) -> &Self
  fn assert_stderr(&self, expected: &str) -> &Self
  fn assert_stderr_contains(&self, expected: &str) -> &Self
struct DatabaseTestTransaction
  async fn begin(app: &AppContext) -> Result<Self>
  fn app(&self) -> &AppContext
  fn transaction(&self) -> &DatabaseTransaction
  async fn rollback(self) -> Result<()>
struct EventFake
  fn new() -> Self
  fn dispatched<E>(&self) -> Vec<E>
  fn reset(&self) -> &Self
  fn assert_dispatched<E>(&self) -> &Self
  fn assert_dispatched_where<E, F>(&self, predicate: F) -> &Self
  fn assert_dispatched_count<E>(&self, expected: usize) -> &Self
  fn assert_not_dispatched<E>(&self) -> &Self
  fn assert_nothing_dispatched(&self) -> &Self
struct FactoryBuilder
  fn new() -> Self
  fn set<T, V>(self, column: Column<M, T>, value: V) -> Self
  fn state<I>(self, values: I) -> Self
  fn for_parent<T, V>(self, foreign_key: Column<M, T>, parent_key: V) -> Self
  fn sequence<F, I>(self, sequence: F) -> Self
  fn count(self, n: usize) -> Self
  async fn create<E>(&self, executor: &E) -> Result<Vec<M>>
  async fn create_one<E>(&self, executor: &E) -> Result<M>
struct FactoryValue
  fn new<T, V>(column: Column<M, T>, value: V) -> Self
struct HttpClientFake
  fn new() -> Self
  fn client(&self) -> HttpClient
  fn client_builder(&self) -> HttpClientBuilder
  fn respond(&self, response: HttpResponse) -> &Self
  fn respond_json<T>( &self, status: StatusCode, value: &T, ) -> HttpClientResult<&Self>
  fn fail(&self, error: HttpClientError) -> &Self
  fn sequence<I>(&self, sequence: I) -> &Self
  fn requests(&self) -> Vec<HttpRequest>
  fn pending_responses(&self) -> usize
  fn reset(&self) -> &Self
  fn assert_sent_count(&self, expected: usize) -> &Self
  fn assert_sent<F>(&self, predicate: F) -> &Self
  fn assert_not_sent<F>(&self, predicate: F) -> &Self
  fn assert_nothing_sent(&self) -> &Self
struct JobFake
  fn new() -> Self
  fn records(&self) -> Vec<RecordedJob>
  fn dispatched<J>(&self) -> Vec<J>
  fn reset(&self) -> &Self
  fn assert_dispatched<J>(&self) -> &Self
  fn assert_dispatched_where<J, F>(&self, predicate: F) -> &Self
  fn assert_dispatched_count<J>(&self, expected: usize) -> &Self
  fn assert_not_dispatched<J>(&self) -> &Self
  fn assert_nothing_dispatched(&self) -> &Self
struct MailFake
  fn new() -> Self
  fn messages(&self) -> Vec<OutboundEmail>
  fn reset(&self) -> &Self
  fn assert_sent(&self) -> &Self
  fn assert_sent_where<F>(&self, predicate: F) -> &Self
  fn assert_sent_count(&self, expected: usize) -> &Self
  fn assert_nothing_sent(&self) -> &Self
struct NotificationFake
  fn new() -> Self
  fn notifications(&self) -> Vec<RecordedNotification>
  fn reset(&self) -> &Self
  fn assert_sent(&self, notification_type: &str) -> &Self
  fn assert_sent_where<F>(&self, predicate: F) -> &Self
  fn assert_sent_count(&self, expected: usize) -> &Self
  fn assert_not_sent(&self, notification_type: &str) -> &Self
  fn assert_nothing_sent(&self) -> &Self
struct PluginTestApp
  fn plugin_id(&self) -> &PluginId
  fn manifest(&self) -> &PluginManifest
  fn contributions(&self) -> &PluginContributions
  fn registry(&self) -> &PluginRegistry
  fn test_app(&self) -> &TestApp
  fn app(&self) -> &AppContext
  fn resolve<T>(&self) -> Result<Arc<T>>
  fn client(&self) -> TestClient
  fn into_test_app(self) -> TestApp
  async fn shutdown(self) -> Result<()>
struct PluginTestHarness
  fn new<I, P>(plugin_id: I, plugin: P) -> Self
  fn register_plugin<P>(self, plugin: P) -> Self
  fn register_plugins<I, P>(self, plugins: I) -> Self
  fn load_config_dir(self, path: impl Into<PathBuf>) -> Self
  fn configure<F>(self, configure: F) -> Self
  async fn build(self) -> Result<PluginTestApp>
struct RecordedJob
struct RecordedNotification
struct StorageFake
  fn new() -> Self
  fn driver_factory(&self) -> StorageDriverFactory
  fn files(&self) -> Vec<StoredFakeFile>
  fn reset(&self) -> &Self
  fn assert_exists(&self, path: &str) -> &Self
  fn assert_missing(&self, path: &str) -> &Self
  fn assert_content(&self, path: &str, expected: impl AsRef<[u8]>) -> &Self
  fn assert_written_count(&self, expected: usize) -> &Self
struct StoredFakeFile
struct TestApp
  fn builder() -> TestAppBuilder
  fn from_builder(builder: AppBuilder) -> TestAppBuilder
  fn app(&self) -> &AppContext
  fn client(&self) -> TestClient
  async fn begin_database_test(&self) -> Result<DatabaseTestTransaction>
  fn freeze_time(&self, now: DateTime) -> Result<ClockFake>
  async fn shutdown(self) -> Result<()>
  async fn seed_presence( &self, channel: &ChannelId, actor_id: &str, joined_at: i64, ) -> Result<()>
  async fn history_ttl(&self, channel: &ChannelId) -> Result<Option<u64>>
struct TestAppBuilder
  fn use_external_tracing_subscriber(self) -> Self
  fn load_config_dir(self, path: impl Into<PathBuf>) -> Self
  fn register_plugin<P>(self, plugin: P) -> Self
  fn register_plugins<I, P>(self, plugins: I) -> Self
  fn register_provider<P>(self, provider: P) -> Self
  fn register_routes<F>(self, registrar: F) -> Self
  fn register_middleware(self, config: MiddlewareConfig) -> Self
  fn register_websocket_routes<F>(self, registrar: F) -> Self
  fn enable_observability(self) -> Self
  fn enable_public_observability(self) -> Self
  fn enable_observability_with(self, options: ObservabilityOptions) -> Self
  fn replace_service<T>(self, value: T) -> Self
  fn replace_service_arc<T>(self, value: Arc<T>) -> Self
  fn fake_events(self, fake: EventFake) -> Self
  fn fake_jobs(self, fake: JobFake) -> Self
  fn fake_mail(self, fake: MailFake) -> Self
  fn fake_notifications(self, fake: NotificationFake) -> Self
  fn fake_http(self, fake: HttpClientFake) -> Self
  fn with_clock(self, fake: ClockFake) -> Self
  async fn build(self) -> Result<TestApp>
struct TestClient
  fn acting_as(self, actor: Actor) -> Self
  fn with_bearer_token(self, token: &str) -> Self
  fn with_session(self, session_id: &str) -> Self
  fn get(&self, path: &str) -> TestRequestBuilder
  fn post(&self, path: &str) -> TestRequestBuilder
  fn put(&self, path: &str) -> TestRequestBuilder
  fn patch(&self, path: &str) -> TestRequestBuilder
  fn delete(&self, path: &str) -> TestRequestBuilder
struct TestRequestBuilder
  fn header(self, name: &str, value: &str) -> Self
  fn bearer_auth(self, token: &str) -> Self
  fn session_auth(self, session_id: &str) -> Self
  fn acting_as(self, actor: Actor) -> Self
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
  fn assert_status(&self, expected: StatusCode) -> &Self
  fn assert_successful(&self) -> &Self
  fn assert_ok(&self) -> &Self
  fn assert_created(&self) -> &Self
  fn assert_no_content(&self) -> &Self
  fn assert_not_found(&self) -> &Self
  fn assert_unprocessable(&self) -> &Self
  fn assert_header(&self, name: &str, expected: &str) -> &Self
  fn assert_header_missing(&self, name: &str) -> &Self
  fn assert_json(&self, expected: &Value) -> &Self
  fn assert_json_path(&self, path: &str, expected: &Value) -> &Self
  fn assert_json_fragment(&self, expected: &Value) -> &Self
  fn assert_json_shape(&self, paths: &[&str]) -> &Self
  fn assert_validation_error(&self, field: &str) -> &Self
  fn assert_redirect(&self, location: &str) -> &Self
  fn assert_download(&self) -> &Self
  fn assert_download_named(&self, filename: &str) -> &Self
trait Factory: Model
  fn definition() -> Vec<FactoryValue<Self>>
  fn factory() -> FactoryBuilder<Self>
async fn assert_database_count<M, E>( executor: &E, query: ModelQuery<M>, expected: u64, ) -> Result<()>
async fn assert_database_has<M, E>( executor: &E, query: ModelQuery<M>, ) -> Result<()>
async fn assert_database_missing<M, E>( executor: &E, query: ModelQuery<M>, ) -> Result<()>
fn assert_safe_to_wipe(db_url: &str) -> Result<()>
```
