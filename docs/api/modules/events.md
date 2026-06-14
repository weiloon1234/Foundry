# events

Domain event bus with typed listeners

[Back to index](../index.md)

## foundry::events

```rust
struct EventBus
  async fn dispatch<E>(&self, event: E) -> Result<()>
  async fn dispatch_with_origin<E>( &self, event: E, origin: Option<EventOrigin>, ) -> Result<()>
struct EventContext
  fn app(&self) -> &AppContext
  fn origin(&self) -> Option<&EventOrigin>
  fn actor(&self) -> Option<&Actor>
  fn ip(&self) -> Option<IpAddr>
  fn user_agent(&self) -> Option<&str>
  fn request_id(&self) -> Option<&str>
struct EventOrigin
  fn new( actor: Option<Actor>, ip: Option<IpAddr>, user_agent: Option<String>, request_id: Option<String>, ) -> Self
  fn from_request( actor: Option<Actor>, request: Option<&CurrentRequest>, ) -> Option<Self>
struct JobDispatchListener
struct WebSocketPublishListener
trait Event: Serialize
trait EventListener: Event>
  fn handle<'life0, 'life1, 'life2, 'async_trait>(
fn dispatch_job<E, J, F>(mapper: F) -> JobDispatchListener<E, J, F>
fn publish_websocket<E, F>(mapper: F) -> WebSocketPublishListener<E, F>
```

