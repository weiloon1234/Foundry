use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::email::EmailMessage;
use crate::foundation::{AppContext, Error, Result};
use crate::jobs::{Job, JobContext};
use crate::support::{JobId, NotificationChannelId};

use super::{
    callback, require_builtin_notification_payload, store_database_notification,
    DatabaseNotificationScope, NotificationChannelRegistry, DEFAULT_NOTIFIABLE_TYPE,
    NOTIFICATION_BROADCAST_CHANNEL, NOTIFICATION_BROADCAST_EVENT, NOTIFY_BROADCAST,
    NOTIFY_DATABASE, NOTIFY_EMAIL,
};

/// Job payload that carries pre-rendered notification data for async dispatch.
///
/// Created by `notify_queued()`. Selected channel payloads are pre-rendered
/// at dispatch time so the worker doesn't need to reconstruct the notification.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SendNotificationJob {
    #[serde(default = "default_notifiable_type")]
    pub(crate) notifiable_type: String,
    pub(crate) notifiable_id: String,
    pub(crate) notification_type: String,
    pub(crate) channels: Vec<NotificationChannelId>,
    pub(crate) email_payload: Option<EmailMessage>,
    pub(crate) database_payload: Option<serde_json::Value>,
    pub(crate) broadcast_payload: Option<serde_json::Value>,
    pub(crate) custom_payloads: Vec<(NotificationChannelId, serde_json::Value)>,
    #[serde(default)]
    pub(crate) custom_routes: Vec<(NotificationChannelId, String)>,
}

#[async_trait]
impl Job for SendNotificationJob {
    const ID: JobId = JobId::new("foundry:send_notification");

    async fn handle(&self, context: JobContext) -> Result<()> {
        self.deliver(context.app()).await
    }
}

impl SendNotificationJob {
    pub(super) fn into_channel_jobs(self) -> Vec<Self> {
        self.channels
            .iter()
            .cloned()
            .map(|channel| self.for_channel(channel))
            .collect()
    }

    pub(super) fn selected_channel(&self) -> Option<&NotificationChannelId> {
        (self.channels.len() == 1).then_some(&self.channels[0])
    }

    fn for_channel(&self, channel: NotificationChannelId) -> Self {
        Self {
            notifiable_type: self.notifiable_type.clone(),
            notifiable_id: self.notifiable_id.clone(),
            notification_type: self.notification_type.clone(),
            channels: vec![channel.clone()],
            email_payload: if channel == NOTIFY_EMAIL {
                self.email_payload.clone()
            } else {
                None
            },
            database_payload: if channel == NOTIFY_DATABASE {
                self.database_payload.clone()
            } else {
                None
            },
            broadcast_payload: if channel == NOTIFY_BROADCAST {
                self.broadcast_payload.clone()
            } else {
                None
            },
            custom_payloads: self
                .custom_payloads
                .iter()
                .filter(|(id, _)| id == &channel)
                .cloned()
                .collect(),
            custom_routes: self
                .custom_routes
                .iter()
                .filter(|(id, _)| id == &channel)
                .cloned()
                .collect(),
        }
    }

    pub(super) async fn deliver(&self, app: &AppContext) -> Result<()> {
        let registry = app
            .resolve::<NotificationChannelRegistry>()
            .map_err(|error| {
                Error::message(format!(
                    "queued notification `{}` channel registry resolution failed: {error}",
                    self.notification_type
                ))
            })?;

        for channel_id in &self.channels {
            self.deliver_channel(app, &registry, channel_id)
                .await
                .map_err(|error| {
                    Error::message(format!(
                        "queued notification `{}` channel `{channel_id}` delivery failed: {error}",
                        self.notification_type
                    ))
                })?;
        }

        Ok(())
    }

    async fn deliver_channel(
        &self,
        app: &AppContext,
        registry: &NotificationChannelRegistry,
        channel_id: &NotificationChannelId,
    ) -> Result<()> {
        if *channel_id == NOTIFY_EMAIL {
            let message =
                require_builtin_notification_payload(self.email_payload.as_ref(), &NOTIFY_EMAIL)?;
            app.email()?.send(message.clone()).await?;
        } else if *channel_id == NOTIFY_DATABASE {
            let data = require_builtin_notification_payload(
                self.database_payload.as_ref(),
                &NOTIFY_DATABASE,
            )?;
            let scope = DatabaseNotificationScope::new(
                self.notifiable_type.clone(),
                self.notifiable_id.clone(),
            )?;
            store_database_notification(app, &scope, self.notification_type.clone(), data.clone())
                .await?;
        } else if *channel_id == NOTIFY_BROADCAST {
            let payload = require_builtin_notification_payload(
                self.broadcast_payload.as_ref(),
                &NOTIFY_BROADCAST,
            )?;
            app.websocket()?
                .publish(
                    NOTIFICATION_BROADCAST_CHANNEL,
                    NOTIFICATION_BROADCAST_EVENT,
                    Some(self.notifiable_id.as_str()),
                    payload.clone(),
                )
                .await?;
        } else {
            let channel = registry.get(channel_id).ok_or_else(|| {
                Error::message(format!(
                    "notification channel `{channel_id}` is not registered"
                ))
            })?;
            let notifiable = QueuedNotifiable {
                notifiable_type: self.notifiable_type.as_str(),
                id: self.notifiable_id.as_str(),
                routes: &self.custom_routes,
            };
            let notification = QueuedNotificationStub {
                notification_type: self.notification_type.as_str(),
                channels: &self.channels,
                custom_payloads: &self.custom_payloads,
            };
            callback::send_channel_adapter(
                channel_id,
                channel.as_ref(),
                app,
                &notifiable,
                &notification,
            )
            .await?;
        }

        Ok(())
    }
}

/// Minimal notifiable for queued replay.
struct QueuedNotifiable<'a> {
    notifiable_type: &'a str,
    id: &'a str,
    routes: &'a [(NotificationChannelId, String)],
}

impl super::Notifiable for QueuedNotifiable<'_> {
    fn notifiable_type(&self) -> &str {
        self.notifiable_type
    }

    fn notification_id(&self) -> String {
        self.id.to_string()
    }

    fn route_notification_for(&self, channel: &str) -> Option<String> {
        self.routes
            .iter()
            .find(|(id, _)| id.as_ref() == channel)
            .map(|(_, route)| route.clone())
    }
}

fn default_notifiable_type() -> String {
    DEFAULT_NOTIFIABLE_TYPE.to_string()
}

/// Minimal notification stub for custom channel replay.
struct QueuedNotificationStub<'a> {
    notification_type: &'a str,
    channels: &'a [NotificationChannelId],
    custom_payloads: &'a [(NotificationChannelId, serde_json::Value)],
}

impl super::Notification for QueuedNotificationStub<'_> {
    fn notification_type(&self) -> &str {
        self.notification_type
    }

    fn via(&self) -> Vec<NotificationChannelId> {
        self.channels.to_vec()
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
