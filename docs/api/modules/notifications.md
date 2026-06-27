# notifications

Multi-channel notifications: email, database, broadcast

[Back to index](../index.md)

## Notes

- `types:export` writes `NotificationManifest.ts` with registered notification
  payload DTOs, built-in channel ids, canonical broadcast channel/event
  constants, and helpers such as `isTypedNotificationBroadcastPayload()` so
  frontend clients can avoid copying notification routing strings.

## foundry::notifications

```rust
pub const NOTIFICATION_BROADCAST_CHANNEL: ChannelId;
pub const NOTIFICATION_BROADCAST_EVENT: ChannelEventId;
pub const NOTIFY_BROADCAST: NotificationChannelId;
pub const NOTIFY_DATABASE: NotificationChannelId;
pub const NOTIFY_EMAIL: NotificationChannelId;
struct BroadcastNotificationChannel
struct DatabaseNotificationChannel
struct EmailNotificationChannel
struct NotificationChannelRegistry
  fn get( &self, id: &NotificationChannelId, ) -> Option<&Arc<dyn NotificationChannel>>
struct SendNotificationJob
trait Notifiable
  fn notification_id(&self) -> String
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
fn build_notification_job( notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> SendNotificationJob
async fn notify( app: &AppContext, notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<()>
async fn notify_queued( app: &AppContext, notifiable: &dyn Notifiable, notification: &dyn Notification, ) -> Result<()>
```
