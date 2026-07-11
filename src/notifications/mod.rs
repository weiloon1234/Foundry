pub(crate) mod callback;
mod channel;
mod database;
pub(crate) mod job;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::auth::{Actor, AuthError};
use crate::database::{DbValue, Query};
use crate::email::EmailMessage;
use crate::foundation::{AppContext, Error, Result};
use crate::support::sync::lock_unpoisoned;
use crate::support::{ChannelEventId, ChannelId, GuardId, NotificationChannelId};
use crate::websocket::{WebSocketChannelOptions, WebSocketContext, WebSocketRegistrar};

pub use channel::{
    BroadcastNotificationChannel, DatabaseNotificationChannel, EmailNotificationChannel,
    NotificationChannel,
};
pub use database::{
    DatabaseNotification, DatabaseNotificationRepository, DatabaseNotificationScope,
};
pub use job::SendNotificationJob;

pub(crate) const NOTIFICATIONS_TABLE: &str = "notifications";
/// Compatibility morph type assigned to notifications created before typed
/// notifiable scopes are configured.
pub const DEFAULT_NOTIFIABLE_TYPE: &str = "default";

pub const NOTIFICATION_BROADCAST_CHANNEL: ChannelId = ChannelId::new("notifications");
pub const NOTIFICATION_BROADCAST_EVENT: ChannelEventId = ChannelEventId::new("notification");

/// Register the built-in, server-only notification WebSocket channel.
///
/// Subscriptions require `guard` and a room equal to the authenticated actor ID. A
/// [`Notifiable::notification_id`] used with broadcast delivery must therefore return
/// the same stable value as [`Actor::id`].
pub fn register_notification_websocket_channel<G>(
    registrar: &mut WebSocketRegistrar,
    guard: G,
) -> Result<()>
where
    G: Into<GuardId>,
{
    registrar.channel_with_options(
        NOTIFICATION_BROADCAST_CHANNEL,
        |_context: WebSocketContext, _payload: serde_json::Value| async {
            Err(Error::http(
                405,
                "the notification channel does not accept client messages",
            ))
        },
        WebSocketChannelOptions::new()
            .guard(guard)
            .authorize(|context, _channel, room| async move {
                let actor = context.actor().ok_or_else(|| {
                    AuthError::unauthorized("authentication is required for notifications")
                })?;
                authorize_notification_room(actor, room.as_deref())
            })
            .outgoing_event::<_, serde_json::Value>(NOTIFICATION_BROADCAST_EVENT),
    )?;
    Ok(())
}

fn authorize_notification_room(actor: &Actor, room: Option<&str>) -> Result<()> {
    if room == Some(actor.id.as_str()) {
        Ok(())
    } else {
        Err(AuthError::forbidden("notification room is not available").into())
    }
}

pub(crate) async fn store_database_notification(
    app: &AppContext,
    scope: &DatabaseNotificationScope,
    notification_type: String,
    data: serde_json::Value,
) -> Result<()> {
    let db = app.database()?;
    Query::insert_into(NOTIFICATIONS_TABLE)
        .values([
            (
                "notifiable_type",
                DbValue::Text(scope.notifiable_type().to_string()),
            ),
            (
                "notifiable_id",
                DbValue::Text(scope.notifiable_id().to_string()),
            ),
            ("type", DbValue::Text(notification_type)),
            ("data", DbValue::Json(data)),
        ])
        .execute(db.as_ref())
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Core Traits
// ---------------------------------------------------------------------------

/// A notification that can be sent across multiple channels.
///
/// Consumer implements this for each notification type.
///
/// ```ignore
/// impl Notification for OrderShipped {
///     fn notification_type(&self) -> &str { "order_shipped" }
///     fn via(&self) -> Vec<String> { vec!["email".into(), "database".into()] }
///     fn to_email(&self, notifiable: &dyn Notifiable) -> Option<EmailMessage> {
///         let email = notifiable.route_notification_for("email")?;
///         Some(EmailMessage::new("Order shipped!").to(&email).text("Your order is on its way."))
///     }
/// }
/// ```
pub trait Notification: Send + Sync {
    /// A stable type identifier for this notification (stored in DB).
    fn notification_type(&self) -> &str;

    /// Which channels to deliver to (e.g., `[NOTIFY_EMAIL, NOTIFY_DATABASE]`).
    fn via(&self) -> Vec<NotificationChannelId>;

    /// Render as an email message.
    fn to_email(&self, _notifiable: &dyn Notifiable) -> Option<EmailMessage> {
        None
    }

    /// Render as a database record (JSON stored in `notifications.data`).
    fn to_database(&self) -> Option<serde_json::Value> {
        None
    }

    /// Render as a WebSocket broadcast payload.
    fn to_broadcast(&self) -> Option<serde_json::Value> {
        None
    }

    /// Render for a custom channel. Called when the channel name doesn't match
    /// a built-in `to_*` method.
    fn to_channel(
        &self,
        _channel: &str,
        _notifiable: &dyn Notifiable,
    ) -> Option<serde_json::Value> {
        None
    }
}

/// A model that can receive notifications.
///
/// ```ignore
/// impl Notifiable for User {
///     fn notification_id(&self) -> String { self.id.to_string() }
///     fn route_notification_for(&self, channel: &str) -> Option<String> {
///         match channel {
///             "email" => Some(self.email.clone()),
///             "sms" => self.phone.clone(),
///             _ => None,
///         }
///     }
/// }
/// ```
pub trait Notifiable: Send + Sync {
    /// Stable morph type used to isolate database notifications whose IDs can
    /// overlap across different notifiable domains.
    ///
    /// The compatibility default matches rows created before typed scopes were
    /// introduced. Override this before using more than one notifiable type.
    fn notifiable_type(&self) -> &str {
        DEFAULT_NOTIFIABLE_TYPE
    }

    /// Unique identifier for this notifiable entity (e.g., user ID).
    fn notification_id(&self) -> String;

    /// Return the routing address for a given channel (e.g., email address, phone number).
    fn route_notification_for(&self, _channel: &str) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// Channel Registry
// ---------------------------------------------------------------------------

pub(crate) type NotificationChannelRegistryHandle = Arc<Mutex<NotificationChannelRegistryBuilder>>;

#[derive(Default)]
pub(crate) struct NotificationChannelRegistryBuilder {
    channels: HashMap<NotificationChannelId, Arc<dyn NotificationChannel>>,
}

impl NotificationChannelRegistryBuilder {
    pub(crate) fn shared() -> NotificationChannelRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn contains(&self, id: &NotificationChannelId) -> bool {
        self.channels.contains_key(id)
    }

    pub(crate) fn register<I>(&mut self, id: I, channel: Arc<dyn NotificationChannel>) -> Result<()>
    where
        I: Into<NotificationChannelId>,
    {
        let id = id.into();
        if self.channels.contains_key(&id) {
            return Err(Error::message(format!(
                "notification channel `{id}` already registered"
            )));
        }
        self.channels.insert(id, channel);
        Ok(())
    }

    pub(crate) fn freeze_shared(
        handle: NotificationChannelRegistryHandle,
    ) -> NotificationChannelRegistry {
        let mut builder = lock_unpoisoned(&handle, "notification channel registry");
        NotificationChannelRegistry {
            channels: std::mem::take(&mut builder.channels),
        }
    }
}

/// Registry of notification channel adapters, frozen at boot.
pub struct NotificationChannelRegistry {
    channels: HashMap<NotificationChannelId, Arc<dyn NotificationChannel>>,
}

impl NotificationChannelRegistry {
    /// Look up a channel by ID.
    pub fn get(&self, id: &NotificationChannelId) -> Option<&Arc<dyn NotificationChannel>> {
        self.channels.get(id)
    }
}

/// Well-known built-in channel IDs.
pub const NOTIFY_EMAIL: NotificationChannelId = NotificationChannelId::new("email");
pub const NOTIFY_DATABASE: NotificationChannelId = NotificationChannelId::new("database");
pub const NOTIFY_BROADCAST: NotificationChannelId = NotificationChannelId::new("broadcast");

pub(crate) fn require_notification_route(
    route: Option<String>,
    channel: &NotificationChannelId,
) -> Result<String> {
    route
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::message(format!("notification channel `{channel}` requires a route")))
}

pub(crate) fn require_builtin_notification_payload<T>(
    payload: Option<T>,
    channel: &NotificationChannelId,
) -> Result<T> {
    let description = if channel == &NOTIFY_EMAIL {
        "an email message"
    } else {
        "a payload"
    };
    payload.ok_or_else(|| {
        Error::message(format!(
            "notification channel `{channel}` requires {description}"
        ))
    })
}

// ---------------------------------------------------------------------------
// Dispatch Functions
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotificationDispatchKind {
    Immediate,
    Queued,
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedNotificationDispatch {
    pub(crate) notifiable_type: String,
    pub(crate) notifiable_id: String,
    pub(crate) notification_type: String,
    pub(crate) channels: Vec<NotificationChannelId>,
    pub(crate) kind: NotificationDispatchKind,
}

pub(crate) trait NotificationDispatchSink: Send + Sync {
    fn record(&self, dispatch: RecordedNotificationDispatch) -> Result<()>;
}

pub(crate) struct NotificationDispatchHook {
    sink: Arc<dyn NotificationDispatchSink>,
}

impl NotificationDispatchHook {
    pub(crate) fn new(sink: Arc<dyn NotificationDispatchSink>) -> Self {
        Self { sink }
    }

    fn record(
        &self,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
        kind: NotificationDispatchKind,
    ) -> Result<()> {
        let scope = DatabaseNotificationScope::for_notifiable(notifiable)?;
        self.sink.record(RecordedNotificationDispatch {
            notifiable_type: scope.notifiable_type().to_string(),
            notifiable_id: scope.notifiable_id().to_string(),
            notification_type: callback::notification_type(notification)?,
            channels: callback::notification_channels(notification)?,
            kind,
        })
    }
}

fn record_fake_notification(
    app: &AppContext,
    notifiable: &dyn Notifiable,
    notification: &dyn Notification,
    kind: NotificationDispatchKind,
) -> Result<bool> {
    let Ok(hook) = app.resolve::<NotificationDispatchHook>() else {
        return Ok(false);
    };
    hook.record(notifiable, notification, kind)?;
    Ok(true)
}

/// Send a notification synchronously, attempting every channel in sequence.
///
/// Any failed or unregistered channels are returned together after the remaining
/// channels have been attempted.
pub async fn notify(
    app: &AppContext,
    notifiable: &dyn Notifiable,
    notification: &dyn Notification,
) -> Result<()> {
    if record_fake_notification(
        app,
        notifiable,
        notification,
        NotificationDispatchKind::Immediate,
    )? {
        return Ok(());
    }

    let registry = app.resolve::<NotificationChannelRegistry>()?;
    let channels = callback::notification_channels(notification)?;
    let notification_type = callback::notification_type(notification)?;
    let mut failures = Vec::new();

    for channel_id in channels {
        let Some(channel) = registry.get(&channel_id) else {
            tracing::error!(
                channel = %channel_id,
                notification_type = %notification_type,
                "notification channel is not registered"
            );
            failures.push(format!("channel `{channel_id}` is not registered"));
            continue;
        };

        if let Err(error) = callback::send_channel_adapter(
            &channel_id,
            channel.as_ref(),
            app,
            notifiable,
            notification,
        )
        .await
        {
            tracing::error!(
                channel = %channel_id,
                notification_type = %notification_type,
                error = %error,
                "notification channel delivery failed"
            );
            failures.push(format!("channel `{channel_id}`: {error}"));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(Error::message(format!(
            "notification `{notification_type}` delivery failed: {}",
            failures.join("; ")
        )))
    }
}

/// Pre-render all notification payloads into the legacy aggregate job shape.
///
/// Rendering, routing, and notification callback failures are returned without
/// constructing a partial job. Existing serialized aggregate jobs remain
/// supported, but new queued dispatch should use [`build_notification_jobs`] so
/// each channel has independent retry and dead-letter state.
pub fn build_notification_job(
    notifiable: &dyn Notifiable,
    notification: &dyn Notification,
) -> Result<SendNotificationJob> {
    let channels = callback::notification_channels(notification)?;
    let notifiable_scope = DatabaseNotificationScope::for_notifiable(notifiable)?;
    let email_payload = if has_notification_channel(&channels, &NOTIFY_EMAIL) {
        require_notification_route(
            callback::route_notification_for(notifiable, NOTIFY_EMAIL.as_ref())?,
            &NOTIFY_EMAIL,
        )?;
        Some(require_builtin_notification_payload(
            callback::notification_email(notification, notifiable)?,
            &NOTIFY_EMAIL,
        )?)
    } else {
        None
    };
    let database_payload = if has_notification_channel(&channels, &NOTIFY_DATABASE) {
        Some(require_builtin_notification_payload(
            callback::notification_database(notification)?,
            &NOTIFY_DATABASE,
        )?)
    } else {
        None
    };
    let broadcast_payload = if has_notification_channel(&channels, &NOTIFY_BROADCAST) {
        Some(require_builtin_notification_payload(
            callback::notification_broadcast(notification)?,
            &NOTIFY_BROADCAST,
        )?)
    } else {
        None
    };

    let mut custom_payloads = Vec::new();
    let mut custom_routes = Vec::new();
    for channel_id in &channels {
        if is_builtin_notification_channel(channel_id) {
            continue;
        }

        if let Some(route) = callback::route_notification_for(notifiable, channel_id.as_ref())? {
            custom_routes.push((channel_id.clone(), route));
        }
        if let Some(data) =
            callback::notification_channel_payload(notification, channel_id.as_ref(), notifiable)?
        {
            custom_payloads.push((channel_id.clone(), data));
        }
    }

    Ok(SendNotificationJob {
        notifiable_type: notifiable_scope.notifiable_type().to_string(),
        notifiable_id: notifiable_scope.notifiable_id().to_string(),
        notification_type: callback::notification_type(notification)?,
        channels,
        email_payload,
        database_payload,
        broadcast_payload,
        custom_payloads,
        custom_routes,
    })
}

/// Pre-render a notification into one independently retryable job per channel.
///
/// Each returned job deliberately retains the existing `SendNotificationJob`
/// ID and wire shape, allowing older workers to consume jobs produced during a
/// rolling deployment. New workers also continue to accept legacy aggregate
/// jobs that were already queued. Deploy custom channel adapters to every
/// worker before producers select them. During a mixed-version rollout, keep
/// [`Notifiable::notifiable_type`] at [`DEFAULT_NOTIFIABLE_TYPE`]; enable custom
/// types after all workers understand the new field.
pub fn build_notification_jobs(
    notifiable: &dyn Notifiable,
    notification: &dyn Notification,
) -> Result<Vec<SendNotificationJob>> {
    Ok(build_notification_job(notifiable, notification)?.into_channel_jobs())
}

fn has_notification_channel(
    channels: &[NotificationChannelId],
    target: &NotificationChannelId,
) -> bool {
    channels.iter().any(|channel| channel == target)
}

fn is_builtin_notification_channel(channel: &NotificationChannelId) -> bool {
    channel == &NOTIFY_EMAIL || channel == &NOTIFY_DATABASE || channel == &NOTIFY_BROADCAST
}

/// Dispatch a notification asynchronously via the job queue.
///
/// Selected channel payloads are pre-rendered before a `SendNotificationJob`
/// is dispatched. Returns after enqueueing without waiting for delivery.
///
/// ```ignore
/// app.notify_queued(&user, &OrderShipped { order_id: "123".into() }).await?;
/// ```
pub async fn notify_queued(
    app: &AppContext,
    notifiable: &dyn Notifiable,
    notification: &dyn Notification,
) -> Result<()> {
    if record_fake_notification(
        app,
        notifiable,
        notification,
        NotificationDispatchKind::Queued,
    )? {
        return Ok(());
    }

    let jobs = build_notification_jobs(notifiable, notification)?;
    dispatch_notification_jobs(app, jobs).await
}

pub(crate) async fn dispatch_notification_jobs(
    app: &AppContext,
    jobs: Vec<SendNotificationJob>,
) -> Result<()> {
    let dispatcher = app.jobs()?;
    let mut failures = Vec::new();
    for job in jobs {
        let channel = job
            .selected_channel()
            .map(ToString::to_string)
            .unwrap_or_else(|| "legacy-aggregate".to_string());
        if let Err(error) = dispatcher.dispatch(job).await {
            failures.push(format!("channel `{channel}`: {error}"));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(Error::message(format!(
            "queued notification dispatch failed: {}",
            failures.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde::Deserialize;
    use serde_json::json;

    use super::*;
    use crate::config::ConfigRepository;
    use crate::foundation::Container;
    use crate::validation::RuleRegistry;

    fn test_app(
        channels: Vec<(NotificationChannelId, Arc<dyn NotificationChannel>)>,
    ) -> AppContext {
        let container = Container::new();
        let handle = NotificationChannelRegistryBuilder::shared();
        {
            let mut builder = lock_unpoisoned(&handle, "notification registry");
            for (id, channel) in channels {
                builder.register(id, channel).unwrap();
            }
        }
        let registry = Arc::new(NotificationChannelRegistryBuilder::freeze_shared(handle));
        container.singleton_arc(registry).unwrap();
        AppContext::new(container, ConfigRepository::empty(), RuleRegistry::new()).unwrap()
    }

    fn queued_job(channel: NotificationChannelId) -> SendNotificationJob {
        SendNotificationJob {
            notifiable_type: DEFAULT_NOTIFIABLE_TYPE.to_string(),
            notifiable_id: "user-1".to_string(),
            notification_type: "test.notification".to_string(),
            channels: vec![channel],
            email_payload: None,
            database_payload: None,
            broadcast_payload: None,
            custom_payloads: Vec::new(),
            custom_routes: Vec::new(),
        }
    }

    #[test]
    fn notification_room_requires_the_authenticated_actor_id() {
        let actor = Actor::new("user-1", GuardId::new("api"));

        authorize_notification_room(&actor, Some("user-1")).unwrap();
        for room in [None, Some("user-2"), Some("")] {
            let error = authorize_notification_room(&actor, room).unwrap_err();
            assert_eq!(error.to_string(), "notification room is not available");
        }
    }

    #[test]
    fn notification_websocket_registration_uses_the_canonical_channel() {
        let mut registrar = WebSocketRegistrar::new();
        register_notification_websocket_channel(&mut registrar, GuardId::new("api")).unwrap();

        let error = register_notification_websocket_channel(&mut registrar, GuardId::new("api"))
            .expect_err("the canonical channel should already be registered");
        assert_eq!(
            error.to_string(),
            "websocket channel `notifications` already registered"
        );
    }

    struct TestNotifiable;

    impl Notifiable for TestNotifiable {
        fn notification_id(&self) -> String {
            "user-1".to_string()
        }

        fn route_notification_for(&self, channel: &str) -> Option<String> {
            (channel == "email").then_some("user@example.com".to_string())
        }
    }

    struct RoutedNotifiable;

    impl Notifiable for RoutedNotifiable {
        fn notification_id(&self) -> String {
            "user-2".to_string()
        }

        fn route_notification_for(&self, channel: &str) -> Option<String> {
            (channel == "sms").then_some("+60123456789".to_string())
        }
    }

    struct PanickingIdNotifiable;

    impl Notifiable for PanickingIdNotifiable {
        fn notification_id(&self) -> String {
            panic!("notifiable id exploded")
        }
    }

    struct PanickingTypeNotifiable;

    impl Notifiable for PanickingTypeNotifiable {
        fn notifiable_type(&self) -> &str {
            panic!("notifiable type exploded")
        }

        fn notification_id(&self) -> String {
            "user-1".to_string()
        }
    }

    struct PanickingRouteNotifiable;

    impl Notifiable for PanickingRouteNotifiable {
        fn notification_id(&self) -> String {
            "user-1".to_string()
        }

        fn route_notification_for(&self, _channel: &str) -> Option<String> {
            panic!("route exploded")
        }
    }

    struct TestNotification {
        channels: Vec<NotificationChannelId>,
    }

    impl TestNotification {
        fn new(channels: Vec<NotificationChannelId>) -> Self {
            Self { channels }
        }
    }

    impl Notification for TestNotification {
        fn notification_type(&self) -> &str {
            "test.notification"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            self.channels.clone()
        }

        fn to_database(&self) -> Option<serde_json::Value> {
            Some(json!({ "ok": true }))
        }

        fn to_channel(
            &self,
            channel: &str,
            _notifiable: &dyn Notifiable,
        ) -> Option<serde_json::Value> {
            Some(json!({ "channel": channel }))
        }
    }

    struct EmptyBuiltinNotification {
        channel: NotificationChannelId,
    }

    impl EmptyBuiltinNotification {
        fn new(channel: NotificationChannelId) -> Self {
            Self { channel }
        }
    }

    impl Notification for EmptyBuiltinNotification {
        fn notification_type(&self) -> &str {
            "test.empty_builtin"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![self.channel.clone()]
        }
    }

    struct EmailPayloadNotification;

    impl Notification for EmailPayloadNotification {
        fn notification_type(&self) -> &str {
            "test.email_payload"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![NOTIFY_EMAIL]
        }

        fn to_email(&self, _notifiable: &dyn Notifiable) -> Option<EmailMessage> {
            Some(EmailMessage::new("Test notification"))
        }
    }

    struct PanickingTypeNotification;

    impl Notification for PanickingTypeNotification {
        fn notification_type(&self) -> &str {
            panic!("type exploded")
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![NOTIFY_DATABASE]
        }
    }

    struct PanickingViaNotification;

    impl Notification for PanickingViaNotification {
        fn notification_type(&self) -> &str {
            "panic.via"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            panic!("via exploded")
        }
    }

    struct PanickingEmailNotification;

    impl Notification for PanickingEmailNotification {
        fn notification_type(&self) -> &str {
            "panic.email"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![NOTIFY_EMAIL]
        }

        fn to_email(&self, _notifiable: &dyn Notifiable) -> Option<EmailMessage> {
            panic!("email renderer exploded")
        }
    }

    struct PanickingDatabaseNotification;

    impl Notification for PanickingDatabaseNotification {
        fn notification_type(&self) -> &str {
            "panic.database"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![NOTIFY_DATABASE]
        }

        fn to_database(&self) -> Option<serde_json::Value> {
            panic!("database renderer exploded")
        }
    }

    struct PanickingBroadcastNotification;

    impl Notification for PanickingBroadcastNotification {
        fn notification_type(&self) -> &str {
            "panic.broadcast"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![NOTIFY_BROADCAST]
        }

        fn to_broadcast(&self) -> Option<serde_json::Value> {
            panic!("broadcast renderer exploded")
        }
    }

    struct PanickingCustomPayloadNotification;

    impl Notification for PanickingCustomPayloadNotification {
        fn notification_type(&self) -> &str {
            "panic.custom"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![NotificationChannelId::new("sms")]
        }

        fn to_channel(
            &self,
            _channel: &str,
            _notifiable: &dyn Notifiable,
        ) -> Option<serde_json::Value> {
            panic!("custom renderer exploded")
        }
    }

    struct PanickingUnselectedRenderersNotification;

    impl Notification for PanickingUnselectedRenderersNotification {
        fn notification_type(&self) -> &str {
            "panic.unselected"
        }

        fn via(&self) -> Vec<NotificationChannelId> {
            vec![NOTIFY_DATABASE]
        }

        fn to_email(&self, _notifiable: &dyn Notifiable) -> Option<EmailMessage> {
            panic!("unselected email renderer exploded")
        }

        fn to_database(&self) -> Option<serde_json::Value> {
            Some(json!({ "selected": "database" }))
        }

        fn to_broadcast(&self) -> Option<serde_json::Value> {
            panic!("unselected broadcast renderer exploded")
        }
    }

    struct PanickingChannel;

    #[async_trait]
    impl NotificationChannel for PanickingChannel {
        async fn send(
            &self,
            _app: &AppContext,
            _notifiable: &dyn Notifiable,
            _notification: &dyn Notification,
        ) -> Result<()> {
            panic!("channel exploded")
        }
    }

    struct RecordingChannel {
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl NotificationChannel for RecordingChannel {
        async fn send(
            &self,
            _app: &AppContext,
            _notifiable: &dyn Notifiable,
            _notification: &dyn Notification,
        ) -> Result<()> {
            self.log.lock().unwrap().push("sent".to_string());
            Ok(())
        }
    }

    struct FailingChannel;

    #[async_trait]
    impl NotificationChannel for FailingChannel {
        async fn send(
            &self,
            _app: &AppContext,
            _notifiable: &dyn Notifiable,
            _notification: &dyn Notification,
        ) -> Result<()> {
            Err(Error::message("adapter unavailable"))
        }
    }

    struct RoutingChannel {
        deliveries: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    }

    #[async_trait]
    impl NotificationChannel for RoutingChannel {
        async fn send(
            &self,
            _app: &AppContext,
            notifiable: &dyn Notifiable,
            notification: &dyn Notification,
        ) -> Result<()> {
            let route = notifiable
                .route_notification_for("sms")
                .ok_or_else(|| Error::message("missing SMS route"))?;
            let payload = notification
                .to_channel("sms", notifiable)
                .ok_or_else(|| Error::message("missing SMS payload"))?;
            self.deliveries.lock().unwrap().push((route, payload));
            Ok(())
        }
    }

    #[tokio::test]
    async fn notification_channel_panic_isolated_and_later_channels_continue() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let panic_id = NotificationChannelId::new("panic");
        let ok_id = NotificationChannelId::new("ok");
        let app = test_app(vec![
            (panic_id.clone(), Arc::new(PanickingChannel)),
            (
                ok_id.clone(),
                Arc::new(RecordingChannel { log: log.clone() }),
            ),
        ]);
        let notification = TestNotification::new(vec![panic_id, ok_id]);

        let error = notify(&app, &TestNotifiable, &notification)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("notification channel `panic` delivery panicked: channel exploded"));
        assert_eq!(log.lock().unwrap().as_slice(), ["sent"]);
    }

    #[tokio::test]
    async fn immediate_notification_reports_missing_channel_and_continues() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let missing_id = NotificationChannelId::new("missing");
        let ok_id = NotificationChannelId::new("ok");
        let app = test_app(vec![(
            ok_id.clone(),
            Arc::new(RecordingChannel { log: log.clone() }),
        )]);
        let notification = TestNotification::new(vec![missing_id, ok_id]);

        let error = notify(&app, &TestNotifiable, &notification)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("channel `missing` is not registered"));
        assert_eq!(log.lock().unwrap().as_slice(), ["sent"]);
    }

    #[tokio::test]
    async fn immediate_notification_reports_adapter_error_and_continues() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let fail_id = NotificationChannelId::new("fail");
        let ok_id = NotificationChannelId::new("ok");
        let app = test_app(vec![
            (fail_id.clone(), Arc::new(FailingChannel)),
            (
                ok_id.clone(),
                Arc::new(RecordingChannel { log: log.clone() }),
            ),
        ]);
        let notification = TestNotification::new(vec![fail_id, ok_id]);

        let error = notify(&app, &TestNotifiable, &notification)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("channel `fail`: adapter unavailable"));
        assert_eq!(log.lock().unwrap().as_slice(), ["sent"]);
    }

    #[tokio::test]
    async fn notification_channel_factory_panic_becomes_error() {
        let channel_id = NotificationChannelId::new("panic");
        let should_panic = true;

        let error = callback::send_notification_channel(&channel_id, move || {
            if should_panic {
                panic!("channel factory exploded");
            }
            std::future::ready(Ok(()))
        })
        .await
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("notification channel `panic` delivery panicked: channel factory exploded"));
    }

    #[tokio::test]
    async fn notification_channel_future_panic_becomes_error() {
        let channel_id = NotificationChannelId::new("panic");
        let should_panic = true;

        let error = callback::send_notification_channel(&channel_id, move || async move {
            if should_panic {
                panic!("channel future exploded");
            }
            Ok(())
        })
        .await
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("notification channel `panic` delivery panicked: channel future exploded"));
    }

    #[tokio::test]
    async fn notification_channel_error_remains_unchanged() {
        let channel_id = NotificationChannelId::new("fail");

        let error = callback::send_notification_channel(&channel_id, || {
            std::future::ready(Err(Error::message("delivery failed")))
        })
        .await
        .unwrap_err();

        assert_eq!(error.to_string(), "delivery failed");
    }

    #[test]
    fn notification_type_panic_becomes_error() {
        let error = callback::notification_type(&PanickingTypeNotification).unwrap_err();

        assert!(error
            .to_string()
            .contains("notification type callback panicked: type exploded"));
    }

    #[tokio::test]
    async fn notification_via_panic_becomes_error() {
        let app = test_app(Vec::new());

        let error = notify(&app, &TestNotifiable, &PanickingViaNotification)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("notification via callback panicked: via exploded"));
    }

    #[test]
    fn notification_email_renderer_panic_becomes_error() {
        let error =
            callback::notification_email(&PanickingEmailNotification, &TestNotifiable).unwrap_err();

        assert!(error
            .to_string()
            .contains("notification email renderer panicked: email renderer exploded"));
    }

    #[test]
    fn notification_broadcast_renderer_panic_becomes_error() {
        let error = callback::notification_broadcast(&PanickingBroadcastNotification).unwrap_err();

        assert!(error
            .to_string()
            .contains("notification broadcast renderer panicked: broadcast renderer exploded"));
    }

    #[test]
    fn notifiable_id_panic_becomes_error() {
        let error = callback::notifiable_id(&PanickingIdNotifiable).unwrap_err();

        assert!(error
            .to_string()
            .contains("notification notifiable id callback panicked: notifiable id exploded"));
    }

    #[test]
    fn notifiable_route_panic_becomes_error() {
        let error = callback::route_notification_for(&PanickingRouteNotifiable, "sms").unwrap_err();

        assert!(error
            .to_string()
            .contains("notification notifiable route callback for `sms` panicked: route exploded"));
    }

    #[tokio::test]
    async fn built_in_email_route_panic_becomes_error() {
        let app = test_app(Vec::new());

        let error = EmailNotificationChannel
            .send(
                &app,
                &PanickingRouteNotifiable,
                &TestNotification::new(vec![NOTIFY_EMAIL]),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains(
            "notification notifiable route callback for `email` panicked: route exploded"
        ));
    }

    #[tokio::test]
    async fn built_in_channel_renderer_panic_becomes_error() {
        let app = test_app(Vec::new());

        let error = DatabaseNotificationChannel
            .send(&app, &TestNotifiable, &PanickingDatabaseNotification)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("notification database renderer panicked: database renderer exploded"));
    }

    #[tokio::test]
    async fn built_in_channels_reject_missing_route_or_payload() {
        let app = test_app(Vec::new());

        let error = EmailNotificationChannel
            .send(&app, &RoutedNotifiable, &EmailPayloadNotification)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "notification channel `email` requires a route"
        );

        let error = EmailNotificationChannel
            .send(
                &app,
                &TestNotifiable,
                &EmptyBuiltinNotification::new(NOTIFY_EMAIL),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "notification channel `email` requires an email message"
        );

        let error = DatabaseNotificationChannel
            .send(
                &app,
                &TestNotifiable,
                &EmptyBuiltinNotification::new(NOTIFY_DATABASE),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "notification channel `database` requires a payload"
        );

        let error = BroadcastNotificationChannel
            .send(
                &app,
                &TestNotifiable,
                &EmptyBuiltinNotification::new(NOTIFY_BROADCAST),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "notification channel `broadcast` requires a payload"
        );
    }

    #[test]
    fn queued_builder_rejects_missing_selected_builtin_route_or_payload() {
        let error = build_notification_job(&RoutedNotifiable, &EmailPayloadNotification)
            .expect_err("selected email channel requires a route");
        assert_eq!(
            error.to_string(),
            "notification channel `email` requires a route"
        );

        for (channel, expected) in [
            (NOTIFY_EMAIL, "an email message"),
            (NOTIFY_DATABASE, "a payload"),
            (NOTIFY_BROADCAST, "a payload"),
        ] {
            let error = build_notification_job(
                &TestNotifiable,
                &EmptyBuiltinNotification::new(channel.clone()),
            )
            .expect_err("selected built-in channel requires its payload");
            assert_eq!(
                error.to_string(),
                format!("notification channel `{channel}` requires {expected}")
            );
        }
    }

    #[test]
    fn queued_builder_only_renders_selected_builtin_channels() {
        let job =
            build_notification_job(&TestNotifiable, &PanickingUnselectedRenderersNotification)
                .unwrap();

        assert_eq!(job.channels, vec![NOTIFY_DATABASE]);
        assert!(job.email_payload.is_none());
        assert_eq!(
            job.database_payload,
            Some(json!({ "selected": "database" }))
        );
        assert!(job.broadcast_payload.is_none());
    }

    #[test]
    fn queued_builder_creates_one_wire_compatible_job_per_selected_channel() {
        #[derive(Deserialize)]
        struct LegacyJobView {
            channels: Vec<NotificationChannelId>,
        }

        let sms = NotificationChannelId::new("sms");
        let jobs = build_notification_jobs(
            &RoutedNotifiable,
            &TestNotification::new(vec![NOTIFY_DATABASE, sms.clone()]),
        )
        .unwrap();

        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].channels, vec![NOTIFY_DATABASE]);
        assert!(jobs[0].database_payload.is_some());
        assert!(jobs[0].custom_payloads.is_empty());
        assert_eq!(jobs[1].channels, vec![sms.clone()]);
        assert!(jobs[1].database_payload.is_none());
        assert_eq!(jobs[1].custom_payloads[0].0, sms);

        for job in jobs {
            let serialized = serde_json::to_value(&job).unwrap();
            let legacy: LegacyJobView = serde_json::from_value(serialized.clone()).unwrap();
            assert_eq!(legacy.channels.len(), 1);
            let current: SendNotificationJob = serde_json::from_value(serialized).unwrap();
            assert_eq!(current.channels, legacy.channels);
        }
    }

    #[test]
    fn queued_builder_propagates_custom_route_panic() {
        let notification = TestNotification::new(vec![NotificationChannelId::new("sms")]);

        let error = build_notification_job(&PanickingRouteNotifiable, &notification).unwrap_err();

        assert!(error
            .to_string()
            .contains("notification notifiable route callback for `sms` panicked: route exploded"));
    }

    #[test]
    fn queued_builder_isolates_notifiable_type_panic() {
        let error = build_notification_jobs(
            &PanickingTypeNotifiable,
            &TestNotification::new(vec![NOTIFY_DATABASE]),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("notification notifiable type callback panicked: notifiable type exploded"));
    }

    #[tokio::test]
    async fn queued_custom_route_survives_serialization_and_reaches_adapter() {
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        let channel_id = NotificationChannelId::new("sms");
        let app = test_app(vec![(
            channel_id.clone(),
            Arc::new(RoutingChannel {
                deliveries: deliveries.clone(),
            }),
        )]);
        let notification = TestNotification::new(vec![channel_id.clone()]);
        let job = build_notification_job(&RoutedNotifiable, &notification).unwrap();
        let serialized = serde_json::to_string(&job).unwrap();
        let job: SendNotificationJob = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            job.custom_routes,
            vec![(channel_id, "+60123456789".to_string())]
        );
        job.deliver(&app).await.unwrap();

        assert_eq!(
            deliveries.lock().unwrap().as_slice(),
            [("+60123456789".to_string(), json!({ "channel": "sms" }))]
        );
    }

    #[test]
    fn queued_job_deserializes_payloads_created_before_custom_routes() {
        let mut serialized = serde_json::to_value(queued_job(NOTIFY_DATABASE)).unwrap();
        serialized.as_object_mut().unwrap().remove("custom_routes");
        serialized
            .as_object_mut()
            .unwrap()
            .remove("notifiable_type");

        let job: SendNotificationJob = serde_json::from_value(serialized).unwrap();

        assert!(job.custom_routes.is_empty());
        assert_eq!(job.notifiable_type, DEFAULT_NOTIFIABLE_TYPE);
    }

    #[tokio::test]
    async fn per_channel_retry_does_not_replay_a_successful_custom_channel() {
        let successful = NotificationChannelId::new("successful");
        let failing = NotificationChannelId::new("failing");
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        let app = test_app(vec![
            (
                successful.clone(),
                Arc::new(RecordingChannel {
                    log: deliveries.clone(),
                }),
            ),
            (failing.clone(), Arc::new(FailingChannel)),
        ]);
        let jobs = build_notification_jobs(
            &TestNotifiable,
            &TestNotification::new(vec![successful.clone(), failing.clone()]),
        )
        .unwrap();
        let successful_job = jobs
            .iter()
            .find(|job| job.selected_channel() == Some(&successful))
            .unwrap();
        let failing_job = jobs
            .iter()
            .find(|job| job.selected_channel() == Some(&failing))
            .unwrap();

        successful_job.deliver(&app).await.unwrap();
        for _attempt in 0..2 {
            assert!(failing_job.deliver(&app).await.is_err());
        }

        assert_eq!(deliveries.lock().unwrap().as_slice(), &["sent"]);
    }

    #[tokio::test]
    async fn queued_builtin_delivery_resolution_failures_are_returned() {
        let app = test_app(Vec::new());

        let mut email_job = queued_job(NOTIFY_EMAIL);
        email_job.email_payload = Some(EmailMessage::new("Test notification"));
        let mut database_job = queued_job(NOTIFY_DATABASE);
        database_job.database_payload = Some(json!({ "ok": true }));
        let mut broadcast_job = queued_job(NOTIFY_BROADCAST);
        broadcast_job.broadcast_payload = Some(json!({ "ok": true }));

        for (channel, job) in [
            ("email", email_job),
            ("database", database_job),
            ("broadcast", broadcast_job),
        ] {
            let error = job.deliver(&app).await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("channel `{channel}` delivery failed")),
                "unexpected {channel} error: {error}"
            );
        }
    }

    #[tokio::test]
    async fn queued_builtin_delivery_rejects_missing_selected_payload() {
        let app = test_app(Vec::new());

        for (channel, expected) in [
            (NOTIFY_EMAIL, "an email message"),
            (NOTIFY_DATABASE, "a payload"),
            (NOTIFY_BROADCAST, "a payload"),
        ] {
            let error = queued_job(channel.clone()).deliver(&app).await.unwrap_err();
            assert!(
                error.to_string().contains(&format!(
                    "channel `{channel}` delivery failed: notification channel `{channel}` requires {expected}"
                )),
                "unexpected {channel} error: {error}"
            );
        }
    }

    #[tokio::test]
    async fn queued_missing_channel_registry_is_returned() {
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();

        let error = queued_job(NOTIFY_DATABASE).deliver(&app).await.unwrap_err();

        assert!(error.to_string().contains(
            "queued notification `test.notification` channel registry resolution failed"
        ));
    }

    #[tokio::test]
    async fn queued_custom_delivery_error_is_returned() {
        let channel_id = NotificationChannelId::new("sms");
        let app = test_app(vec![(channel_id.clone(), Arc::new(FailingChannel))]);
        let notification = TestNotification::new(vec![channel_id]);
        let job = build_notification_job(&RoutedNotifiable, &notification).unwrap();

        let error = job.deliver(&app).await.unwrap_err();

        assert!(error.to_string().contains(
            "queued notification `test.notification` channel `sms` delivery failed: adapter unavailable"
        ));
    }

    #[tokio::test]
    async fn queued_custom_delivery_panic_is_returned() {
        let channel_id = NotificationChannelId::new("sms");
        let app = test_app(vec![(channel_id.clone(), Arc::new(PanickingChannel))]);
        let notification = TestNotification::new(vec![channel_id]);
        let job = build_notification_job(&RoutedNotifiable, &notification).unwrap();

        let error = job.deliver(&app).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("notification channel `sms` delivery panicked: channel exploded"));
    }

    #[tokio::test]
    async fn queued_missing_custom_channel_is_returned() {
        let app = test_app(Vec::new());
        let notification = TestNotification::new(vec![NotificationChannelId::new("unregistered")]);
        let job = build_notification_job(&TestNotifiable, &notification).unwrap();

        let error = job.deliver(&app).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("notification channel `unregistered` is not registered"));
    }

    #[tokio::test]
    async fn notify_queued_renderer_panic_becomes_error_before_dispatch() {
        let app = test_app(Vec::new());

        let error = notify_queued(&app, &TestNotifiable, &PanickingCustomPayloadNotification)
            .await
            .unwrap_err();

        assert!(error.to_string().contains(
            "notification custom channel `sms` renderer panicked: custom renderer exploded"
        ));
    }

    #[test]
    fn public_notification_job_builder_returns_renderer_panic() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            build_notification_job(&TestNotifiable, &PanickingCustomPayloadNotification)
        }));

        let error = result
            .expect("builder should isolate notification renderer panics")
            .unwrap_err();
        assert!(error.to_string().contains(
            "notification custom channel `sms` renderer panicked: custom renderer exploded"
        ));
    }
}
