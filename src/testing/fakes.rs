use std::any::TypeId;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::email::{EmailDriver, OutboundEmail};
use crate::events::{Event, EventDispatchSink, EventOrigin, RecordedEventDispatch};
use crate::foundation::Result;
use crate::jobs::{Job, JobDispatchSink, RecordedJobDispatch};
use crate::notifications::{
    NotificationDispatchKind, NotificationDispatchSink, RecordedNotificationDispatch,
};
use crate::support::sync::lock_unpoisoned;
use crate::support::{JobId, NotificationChannelId, QueueId};

/// Event dispatcher fake that records typed events and suppresses listeners.
#[derive(Clone, Default)]
pub struct EventFake {
    dispatches: Arc<Mutex<Vec<RecordedEventDispatch>>>,
}

impl EventFake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dispatched<E>(&self) -> Vec<E>
    where
        E: Event,
    {
        lock_unpoisoned(&self.dispatches, "event fake")
            .iter()
            .filter(|dispatch| dispatch.event_type == TypeId::of::<E>())
            .filter_map(|dispatch| dispatch.event.downcast_ref::<E>().cloned())
            .collect()
    }

    pub fn reset(&self) -> &Self {
        lock_unpoisoned(&self.dispatches, "event fake").clear();
        self
    }

    #[track_caller]
    pub fn assert_dispatched<E>(&self) -> &Self
    where
        E: Event,
    {
        let count = self.dispatched::<E>().len();
        assert!(count > 0, "expected event `{}` to be dispatched", E::ID);
        self
    }

    #[track_caller]
    pub fn assert_dispatched_where<E, F>(&self, predicate: F) -> &Self
    where
        E: Event,
        F: Fn(&E, Option<&EventOrigin>) -> bool,
    {
        let dispatches = lock_unpoisoned(&self.dispatches, "event fake");
        assert!(
            dispatches.iter().any(|dispatch| {
                dispatch.event_type == TypeId::of::<E>()
                    && dispatch
                        .event
                        .downcast_ref::<E>()
                        .is_some_and(|event| predicate(event, dispatch.origin.as_ref()))
            }),
            "no dispatched event `{}` matched the assertion",
            E::ID
        );
        self
    }

    #[track_caller]
    pub fn assert_dispatched_count<E>(&self, expected: usize) -> &Self
    where
        E: Event,
    {
        let actual = self.dispatched::<E>().len();
        assert_eq!(
            actual,
            expected,
            "expected event `{}` to be dispatched {expected} time(s), recorded {actual}",
            E::ID
        );
        self
    }

    #[track_caller]
    pub fn assert_not_dispatched<E>(&self) -> &Self
    where
        E: Event,
    {
        self.assert_dispatched_count::<E>(0)
    }

    #[track_caller]
    pub fn assert_nothing_dispatched(&self) -> &Self {
        let dispatches = lock_unpoisoned(&self.dispatches, "event fake");
        assert!(
            dispatches.is_empty(),
            "expected no events to be dispatched; recorded: {}",
            dispatches
                .iter()
                .map(|dispatch| format!("{} ({})", dispatch.event_id, dispatch.event_type_name))
                .collect::<Vec<_>>()
                .join(", ")
        );
        self
    }
}

impl EventDispatchSink for EventFake {
    fn record(&self, dispatch: RecordedEventDispatch) -> Result<()> {
        lock_unpoisoned(&self.dispatches, "event fake").push(dispatch);
        Ok(())
    }
}

/// Metadata and serialized payload captured for one queued job.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedJob {
    pub job_id: JobId,
    pub queue: QueueId,
    pub scheduled_at: i64,
    pub payload: serde_json::Value,
}

/// Job dispatcher fake that records typed jobs and suppresses queue writes.
#[derive(Clone, Default)]
pub struct JobFake {
    dispatches: Arc<Mutex<Vec<RecordedJobDispatch>>>,
}

impl JobFake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> Vec<RecordedJob> {
        lock_unpoisoned(&self.dispatches, "job fake")
            .iter()
            .map(|dispatch| RecordedJob {
                job_id: dispatch.job_id.clone(),
                queue: dispatch.queue.clone(),
                scheduled_at: dispatch.scheduled_at,
                payload: dispatch.payload.clone(),
            })
            .collect()
    }

    pub fn dispatched<J>(&self) -> Vec<J>
    where
        J: Job,
    {
        lock_unpoisoned(&self.dispatches, "job fake")
            .iter()
            .filter(|dispatch| dispatch.job_id == J::ID)
            .map(|dispatch| {
                serde_json::from_value(dispatch.payload.clone()).unwrap_or_else(|error| {
                    panic!(
                        "recorded job `{}` could not be decoded as `{}`: {error}",
                        J::ID,
                        std::any::type_name::<J>()
                    )
                })
            })
            .collect()
    }

    pub fn reset(&self) -> &Self {
        lock_unpoisoned(&self.dispatches, "job fake").clear();
        self
    }

    #[track_caller]
    pub fn assert_dispatched<J>(&self) -> &Self
    where
        J: Job,
    {
        let count = self.dispatched::<J>().len();
        assert!(count > 0, "expected job `{}` to be dispatched", J::ID);
        self
    }

    #[track_caller]
    pub fn assert_dispatched_where<J, F>(&self, predicate: F) -> &Self
    where
        J: Job,
        F: Fn(&J, &RecordedJob) -> bool,
    {
        let records = self.records();
        assert!(
            records.iter().any(|record| {
                record.job_id == J::ID
                    && serde_json::from_value::<J>(record.payload.clone())
                        .ok()
                        .is_some_and(|job| predicate(&job, record))
            }),
            "no dispatched job `{}` matched the assertion",
            J::ID
        );
        self
    }

    #[track_caller]
    pub fn assert_dispatched_count<J>(&self, expected: usize) -> &Self
    where
        J: Job,
    {
        let actual = self.dispatched::<J>().len();
        assert_eq!(
            actual,
            expected,
            "expected job `{}` to be dispatched {expected} time(s), recorded {actual}",
            J::ID
        );
        self
    }

    #[track_caller]
    pub fn assert_not_dispatched<J>(&self) -> &Self
    where
        J: Job,
    {
        self.assert_dispatched_count::<J>(0)
    }

    #[track_caller]
    pub fn assert_nothing_dispatched(&self) -> &Self {
        let dispatches = lock_unpoisoned(&self.dispatches, "job fake");
        assert!(
            dispatches.is_empty(),
            "expected no jobs to be dispatched; recorded: {}",
            dispatches
                .iter()
                .map(|dispatch| dispatch.job_id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        self
    }
}

impl JobDispatchSink for JobFake {
    fn record(&self, dispatch: RecordedJobDispatch) -> Result<()> {
        lock_unpoisoned(&self.dispatches, "job fake").push(dispatch);
        Ok(())
    }
}

/// Email driver fake that records fully resolved outbound messages.
#[derive(Clone, Default)]
pub struct MailFake {
    messages: Arc<Mutex<Vec<OutboundEmail>>>,
}

impl MailFake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> Vec<OutboundEmail> {
        lock_unpoisoned(&self.messages, "mail fake").clone()
    }

    pub fn reset(&self) -> &Self {
        lock_unpoisoned(&self.messages, "mail fake").clear();
        self
    }

    #[track_caller]
    pub fn assert_sent(&self) -> &Self {
        let count = lock_unpoisoned(&self.messages, "mail fake").len();
        assert!(count > 0, "expected an email to be sent");
        self
    }

    #[track_caller]
    pub fn assert_sent_where<F>(&self, predicate: F) -> &Self
    where
        F: Fn(&OutboundEmail) -> bool,
    {
        let messages = self.messages();
        assert!(
            messages.iter().any(predicate),
            "no sent email matched the assertion; messages: {messages:?}"
        );
        self
    }

    #[track_caller]
    pub fn assert_sent_count(&self, expected: usize) -> &Self {
        let actual = lock_unpoisoned(&self.messages, "mail fake").len();
        assert_eq!(
            actual, expected,
            "expected {expected} email(s) to be sent, recorded {actual}"
        );
        self
    }

    #[track_caller]
    pub fn assert_nothing_sent(&self) -> &Self {
        self.assert_sent_count(0)
    }
}

#[async_trait]
impl EmailDriver for MailFake {
    async fn send(&self, message: &OutboundEmail) -> Result<()> {
        lock_unpoisoned(&self.messages, "mail fake").push(message.clone());
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationDelivery {
    Immediate,
    Queued,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedNotification {
    pub notifiable_type: String,
    pub notifiable_id: String,
    pub notification_type: String,
    pub channels: Vec<NotificationChannelId>,
    pub delivery: NotificationDelivery,
}

/// Notification fake that records immediate and queued delivery attempts.
#[derive(Clone, Default)]
pub struct NotificationFake {
    notifications: Arc<Mutex<Vec<RecordedNotification>>>,
}

impl NotificationFake {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notifications(&self) -> Vec<RecordedNotification> {
        lock_unpoisoned(&self.notifications, "notification fake").clone()
    }

    pub fn reset(&self) -> &Self {
        lock_unpoisoned(&self.notifications, "notification fake").clear();
        self
    }

    #[track_caller]
    pub fn assert_sent(&self, notification_type: &str) -> &Self {
        self.assert_sent_where(|notification| notification.notification_type == notification_type)
    }

    #[track_caller]
    pub fn assert_sent_where<F>(&self, predicate: F) -> &Self
    where
        F: Fn(&RecordedNotification) -> bool,
    {
        let notifications = self.notifications();
        assert!(
            notifications.iter().any(predicate),
            "no sent notification matched the assertion; notifications: {notifications:?}"
        );
        self
    }

    #[track_caller]
    pub fn assert_sent_count(&self, expected: usize) -> &Self {
        let actual = lock_unpoisoned(&self.notifications, "notification fake").len();
        assert_eq!(
            actual, expected,
            "expected {expected} notification(s) to be sent, recorded {actual}"
        );
        self
    }

    #[track_caller]
    pub fn assert_not_sent(&self, notification_type: &str) -> &Self {
        let notifications = self.notifications();
        assert!(
            !notifications
                .iter()
                .any(|notification| notification.notification_type == notification_type),
            "notification `{notification_type}` was unexpectedly sent"
        );
        self
    }

    #[track_caller]
    pub fn assert_nothing_sent(&self) -> &Self {
        self.assert_sent_count(0)
    }
}

impl NotificationDispatchSink for NotificationFake {
    fn record(&self, dispatch: RecordedNotificationDispatch) -> Result<()> {
        lock_unpoisoned(&self.notifications, "notification fake").push(RecordedNotification {
            notifiable_type: dispatch.notifiable_type,
            notifiable_id: dispatch.notifiable_id,
            notification_type: dispatch.notification_type,
            channels: dispatch.channels,
            delivery: match dispatch.kind {
                NotificationDispatchKind::Immediate => NotificationDelivery::Immediate,
                NotificationDispatchKind::Queued => NotificationDelivery::Queued,
            },
        });
        Ok(())
    }
}
