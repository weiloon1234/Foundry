use async_trait::async_trait;

use crate::foundation::{AppContext, Result};

use super::{
    callback, require_builtin_notification_payload, require_notification_route,
    store_database_notification, DatabaseNotificationScope, Notifiable, Notification,
    NOTIFICATION_BROADCAST_CHANNEL, NOTIFICATION_BROADCAST_EVENT, NOTIFY_BROADCAST,
    NOTIFY_DATABASE, NOTIFY_EMAIL,
};

/// Adapter trait for notification delivery channels.
///
/// Framework provides built-in channels (email, database, broadcast).
/// Projects can register custom channels via `register_notification_channel()`.
/// Returning an error fails immediate delivery and makes queued jobs eligible
/// for their configured retry and dead-letter policy.
#[async_trait]
pub trait NotificationChannel: Send + Sync + 'static {
    async fn send(
        &self,
        app: &AppContext,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
    ) -> Result<()>;
}

/// Built-in email notification channel.
pub struct EmailNotificationChannel;

#[async_trait]
impl NotificationChannel for EmailNotificationChannel {
    async fn send(
        &self,
        app: &AppContext,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
    ) -> Result<()> {
        let _email = require_notification_route(
            callback::route_notification_for(notifiable, NOTIFY_EMAIL.as_ref())?,
            &NOTIFY_EMAIL,
        )?;
        let message = require_builtin_notification_payload(
            callback::notification_email(notification, notifiable)?,
            &NOTIFY_EMAIL,
        )?;
        app.email()?.send(message).await
    }
}

/// Built-in database notification channel.
/// Stores notifications in the `notifications` table.
pub struct DatabaseNotificationChannel;

#[async_trait]
impl NotificationChannel for DatabaseNotificationChannel {
    async fn send(
        &self,
        app: &AppContext,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
    ) -> Result<()> {
        let data = require_builtin_notification_payload(
            callback::notification_database(notification)?,
            &NOTIFY_DATABASE,
        )?;
        let scope = DatabaseNotificationScope::for_notifiable(notifiable)?;
        store_database_notification(
            app,
            &scope,
            callback::notification_type(notification)?,
            data,
        )
        .await?;
        Ok(())
    }
}

/// Built-in WebSocket broadcast notification channel.
pub struct BroadcastNotificationChannel;

#[async_trait]
impl NotificationChannel for BroadcastNotificationChannel {
    async fn send(
        &self,
        app: &AppContext,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
    ) -> Result<()> {
        let payload = require_builtin_notification_payload(
            callback::notification_broadcast(notification)?,
            &NOTIFY_BROADCAST,
        )?;
        let ws = app.websocket()?;
        let room = callback::notifiable_id(notifiable)?;
        ws.publish(
            NOTIFICATION_BROADCAST_CHANNEL,
            NOTIFICATION_BROADCAST_EVENT,
            Some(room.as_str()),
            payload,
        )
        .await
    }
}
