use async_trait::async_trait;

use crate::foundation::{AppContext, Result};

use super::{
    callback, store_database_notification, Notifiable, Notification,
    NOTIFICATION_BROADCAST_CHANNEL, NOTIFICATION_BROADCAST_EVENT,
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
        let Some(_email) = callback::route_notification_for(notifiable, "email")? else {
            return Ok(());
        };
        let Some(message) = callback::notification_email(notification, notifiable)? else {
            return Ok(());
        };
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
        let Some(data) = callback::notification_database(notification)? else {
            return Ok(());
        };
        store_database_notification(
            app,
            callback::notifiable_id(notifiable)?,
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
        let Some(payload) = callback::notification_broadcast(notification)? else {
            return Ok(());
        };
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
