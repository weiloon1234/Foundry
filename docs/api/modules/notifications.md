# notifications

Multi-channel notifications: email, database, broadcast

[Back to index](../index.md)

## foundry::notifications

```rust
pub const DEFAULT_NOTIFIABLE_TYPE: &str;
pub const NOTIFICATION_BROADCAST_CHANNEL: ChannelId;
pub const NOTIFICATION_BROADCAST_EVENT: ChannelEventId;
pub const NOTIFY_BROADCAST: NotificationChannelId;
pub const NOTIFY_DATABASE: NotificationChannelId;
pub const NOTIFY_EMAIL: NotificationChannelId;
struct BroadcastNotificationChannel
struct DatabaseNotification
  fn is_read(&self) -> bool
  fn is_unread(&self) -> bool
struct DatabaseNotificationChannel
struct DatabaseNotificationRepository
  fn new( notifiable_type: impl Into<String>, notifiable_id: impl Into<String>, ) -> Result<Self>
  fn from_scope(scope: DatabaseNotificationScope) -> Self
  fn for_notifiable(notifiable: &dyn Notifiable) -> Result<Self>
  fn for_actor(actor: &Actor) -> Result<Self>
  fn for_actor_as( actor: &Actor, notifiable_type: impl Into<String>, ) -> Result<Self>
  fn scope(&self) -> &DatabaseNotificationScope
  async fn list(&self, app: &AppContext) -> Result<Vec<DatabaseNotification>>
  async fn list_with<E>( &self, executor: &E, ) -> Result<Vec<DatabaseNotification>>
  async fn paginate( &self, app: &AppContext, pagination: Pagination, ) -> Result<Paginated<DatabaseNotification>>
  async fn paginate_with<E>( &self, executor: &E, pagination: Pagination, ) -> Result<Paginated<DatabaseNotification>>
  async fn unread( &self, app: &AppContext, ) -> Result<Vec<DatabaseNotification>>
  async fn unread_with<E>( &self, executor: &E, ) -> Result<Vec<DatabaseNotification>>
  async fn read(&self, app: &AppContext) -> Result<Vec<DatabaseNotification>>
  async fn read_with<E>( &self, executor: &E, ) -> Result<Vec<DatabaseNotification>>
  async fn unread_count(&self, app: &AppContext) -> Result<u64>
  async fn unread_count_with<E>(&self, executor: &E) -> Result<u64>
  async fn mark_read( &self, app: &AppContext, id: ModelId<DatabaseNotification>, ) -> Result<bool>
  async fn mark_read_with<E>( &self, executor: &E, id: ModelId<DatabaseNotification>, ) -> Result<bool>
  async fn mark_all_read(&self, app: &AppContext) -> Result<u64>
  async fn mark_all_read_with<E>(&self, executor: &E) -> Result<u64>
  async fn delete( &self, app: &AppContext, id: ModelId<DatabaseNotification>, ) -> Result<bool>
  async fn delete_with<E>( &self, executor: &E, id: ModelId<DatabaseNotification>, ) -> Result<bool>
struct DatabaseNotificationScope
  fn new( notifiable_type: impl Into<String>, notifiable_id: impl Into<String>, ) -> Result<Self>
  fn for_notifiable(notifiable: &dyn Notifiable) -> Result<Self>
  fn for_actor(actor: &Actor) -> Result<Self>
  fn for_actor_as( actor: &Actor, notifiable_type: impl Into<String>, ) -> Result<Self>
  fn notifiable_type(&self) -> &str
  fn notifiable_id(&self) -> &str
struct EmailNotificationChannel
struct NotificationChannelRegistry
  fn get( &self, id: &NotificationChannelId, ) -> Option<&Arc<dyn NotificationChannel>>
struct SendNotificationJob
trait Notifiable
  fn notification_id(&self) -> String
  fn notifiable_type(&self) -> &str
  fn route_notification_for(&self, _channel: &str) -> Option<String>
trait Notification
  fn notification_type(&self) -> &str
  fn via(&self) -> Vec<NotificationChannelId>
  fn to_email(&self, _notifiable: &dyn Notifiable) -> Option<EmailMessage>
  fn to_database(&self) -> Option<Value>
  fn to_broadcast(&self) -> Option<Value>
  fn to_channel(
trait NotificationChannel
  fn send<'life0, 'life1, 'life2, 'life3, 'async_trait>(
fn build_notification_job( notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<SendNotificationJob>
fn build_notification_jobs( notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<Vec<SendNotificationJob>>
async fn notify( app: &AppContext, notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<()>
async fn notify_queued( app: &AppContext, notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<()>
fn register_notification_websocket_channel<G>( registrar: &mut WebSocketRegistrar, guard: G, ) -> Result<()>
```
