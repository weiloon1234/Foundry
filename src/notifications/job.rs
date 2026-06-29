use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::email::EmailMessage;
use crate::foundation::Result;
use crate::jobs::{Job, JobContext};
use crate::support::{JobId, NotificationChannelId};

use super::{
    callback, store_database_notification, NotificationChannelRegistry,
    NOTIFICATION_BROADCAST_CHANNEL, NOTIFICATION_BROADCAST_EVENT, NOTIFY_BROADCAST,
    NOTIFY_DATABASE, NOTIFY_EMAIL,
};

/// Job payload that carries pre-rendered notification data for async dispatch.
///
/// Created by `notify_queued()`. Selected channel payloads are pre-rendered
/// at dispatch time so the worker doesn't need to reconstruct the notification.
#[derive(Debug, Serialize, Deserialize)]
pub struct SendNotificationJob {
    pub(crate) notifiable_id: String,
    pub(crate) notification_type: String,
    pub(crate) channels: Vec<NotificationChannelId>,
    pub(crate) email_payload: Option<EmailMessage>,
    pub(crate) database_payload: Option<serde_json::Value>,
    pub(crate) broadcast_payload: Option<serde_json::Value>,
    pub(crate) custom_payloads: Vec<(NotificationChannelId, serde_json::Value)>,
}

#[async_trait]
impl Job for SendNotificationJob {
    const ID: JobId = JobId::new("foundry:send_notification");

    async fn handle(&self, context: JobContext) -> Result<()> {
        let app = context.app();
        let registry = app.resolve::<NotificationChannelRegistry>()?;

        for channel_id in &self.channels {
            if *channel_id == NOTIFY_EMAIL {
                if let Some(ref message) = self.email_payload {
                    if let Err(error) = app.email()?.send(message.clone()).await {
                        tracing::error!(
                            channel = "email",
                            notification_type = %self.notification_type,
                            error = %error,
                            "queued notification email delivery failed"
                        );
                    }
                }
            } else if *channel_id == NOTIFY_DATABASE {
                if let Some(ref data) = self.database_payload {
                    if let Err(error) = store_database_notification(
                        app,
                        self.notifiable_id.clone(),
                        self.notification_type.clone(),
                        data.clone(),
                    )
                    .await
                    {
                        tracing::error!(
                            channel = "database",
                            notification_type = %self.notification_type,
                            error = %error,
                            "queued notification database delivery failed"
                        );
                    }
                }
            } else if *channel_id == NOTIFY_BROADCAST {
                if let Some(ref payload) = self.broadcast_payload {
                    if let Ok(ws) = app.websocket() {
                        let _ = ws
                            .publish(
                                NOTIFICATION_BROADCAST_CHANNEL,
                                NOTIFICATION_BROADCAST_EVENT,
                                Some(self.notifiable_id.as_str()),
                                payload.clone(),
                            )
                            .await;
                    }
                }
            } else {
                // Custom channel — look up from registry and send with a minimal notifiable stub
                if let Some(channel) = registry.get(channel_id) {
                    let stub = QueuedNotifiable {
                        id: self.notifiable_id.clone(),
                    };
                    let stub_notification = QueuedNotificationStub {
                        notification_type: self.notification_type.clone(),
                        channels: self.channels.clone(),
                        custom_payloads: self.custom_payloads.clone(),
                    };
                    if let Err(error) = callback::send_channel_adapter(
                        channel_id,
                        channel.as_ref(),
                        app,
                        &stub,
                        &stub_notification,
                    )
                    .await
                    {
                        tracing::error!(
                            channel = %channel_id,
                            notification_type = %self.notification_type,
                            error = %error,
                            "queued notification custom channel delivery failed"
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Minimal notifiable for queued replay.
struct QueuedNotifiable {
    id: String,
}

impl super::Notifiable for QueuedNotifiable {
    fn notification_id(&self) -> String {
        self.id.clone()
    }
}

/// Minimal notification stub for custom channel replay.
struct QueuedNotificationStub {
    notification_type: String,
    channels: Vec<NotificationChannelId>,
    custom_payloads: Vec<(NotificationChannelId, serde_json::Value)>,
}

impl super::Notification for QueuedNotificationStub {
    fn notification_type(&self) -> &str {
        &self.notification_type
    }

    fn via(&self) -> Vec<NotificationChannelId> {
        self.channels.clone()
    }

    fn to_channel(
        &self,
        channel: &str,
        _notifiable: &dyn super::Notifiable,
    ) -> Option<serde_json::Value> {
        self.custom_payloads
            .iter()
            .find(|(id, _)| id.as_ref() == channel)
            .map(|(_, v)| v.clone())
    }
}
