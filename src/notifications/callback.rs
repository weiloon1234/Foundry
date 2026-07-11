use std::future::Future;

use crate::email::EmailMessage;
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_async_panic, catch_sync_panic, panic_payload_message};
use crate::support::NotificationChannelId;

use super::{Notifiable, Notification, NotificationChannel};

pub(crate) fn notification_type(notification: &dyn Notification) -> Result<String> {
    catch_notification_callback("type callback", || {
        notification.notification_type().to_string()
    })
}

pub(crate) fn notification_channels(
    notification: &dyn Notification,
) -> Result<Vec<NotificationChannelId>> {
    catch_notification_callback("via callback", || notification.via())
}

pub(crate) fn notification_email(
    notification: &dyn Notification,
    notifiable: &dyn Notifiable,
) -> Result<Option<EmailMessage>> {
    catch_notification_callback("email renderer", || notification.to_email(notifiable))
}

pub(crate) fn notification_database(
    notification: &dyn Notification,
) -> Result<Option<serde_json::Value>> {
    catch_notification_callback("database renderer", || notification.to_database())
}

pub(crate) fn notification_broadcast(
    notification: &dyn Notification,
) -> Result<Option<serde_json::Value>> {
    catch_notification_callback("broadcast renderer", || notification.to_broadcast())
}

pub(crate) fn notification_channel_payload(
    notification: &dyn Notification,
    channel: &str,
    notifiable: &dyn Notifiable,
) -> Result<Option<serde_json::Value>> {
    let subject = format!("custom channel `{channel}` renderer");
    catch_notification_callback(&subject, || notification.to_channel(channel, notifiable))
}

pub(crate) fn notifiable_id(notifiable: &dyn Notifiable) -> Result<String> {
    catch_notification_callback("notifiable id callback", || notifiable.notification_id())
}

pub(crate) fn notifiable_type(notifiable: &dyn Notifiable) -> Result<String> {
    catch_notification_callback("notifiable type callback", || {
        notifiable.notifiable_type().to_string()
    })
}

pub(crate) fn route_notification_for(
    notifiable: &dyn Notifiable,
    channel: &str,
) -> Result<Option<String>> {
    let subject = format!("notifiable route callback for `{channel}`");
    catch_notification_callback(&subject, || notifiable.route_notification_for(channel))
}

pub(crate) async fn send_notification_channel<F, Fut>(
    channel_id: &NotificationChannelId,
    send: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let subject = format!("channel `{channel_id}` delivery");
    match catch_async_panic(send).await {
        Ok(result) => result,
        Err(panic) => Err(notification_panic_error(&subject, panic)),
    }
}

pub(crate) async fn send_channel_adapter(
    channel_id: &NotificationChannelId,
    channel: &dyn NotificationChannel,
    app: &AppContext,
    notifiable: &dyn Notifiable,
    notification: &dyn Notification,
) -> Result<()> {
    send_notification_channel(channel_id, || channel.send(app, notifiable, notification)).await
}

fn catch_notification_callback<T, F>(subject: &str, callback: F) -> Result<T>
where
    F: FnOnce() -> T,
{
    catch_sync_panic(callback).map_err(|panic| notification_panic_error(subject, panic))
}

fn notification_panic_error(subject: &str, panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.notification",
        subject = subject,
        panic = %message,
        "notification callback panicked"
    );
    Error::message(format!("notification {subject} panicked: {message}"))
}
