use std::fs;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use foundry::prelude::*;

const FIRST_CHANNEL: NotificationChannelId = NotificationChannelId::new("test:first");
const FAILING_CHANNEL: NotificationChannelId = NotificationChannelId::new("test:failing");
const LAST_CHANNEL: NotificationChannelId = NotificationChannelId::new("test:last");

struct QueueProvider {
    deliveries: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl ServiceProvider for QueueProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_notification_channel(
            FIRST_CHANNEL,
            RecordingChannel {
                label: "first",
                deliveries: self.deliveries.clone(),
            },
        )?;
        registrar.register_notification_channel(FAILING_CHANNEL, FailingChannel)?;
        registrar.register_notification_channel(
            LAST_CHANNEL,
            RecordingChannel {
                label: "last",
                deliveries: self.deliveries.clone(),
            },
        )
    }
}

struct RecordingChannel {
    label: &'static str,
    deliveries: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl NotificationChannel for RecordingChannel {
    async fn send(
        &self,
        _app: &AppContext,
        _notifiable: &dyn Notifiable,
        _notification: &dyn Notification,
    ) -> Result<()> {
        self.deliveries.lock().unwrap().push(self.label);
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
        Err(Error::message("intentional channel failure"))
    }
}

struct QueueRecipient;

impl Notifiable for QueueRecipient {
    fn notification_id(&self) -> String {
        "queue-recipient".to_string()
    }
}

struct ThreeChannelNotification;

impl Notification for ThreeChannelNotification {
    fn notification_type(&self) -> &str {
        "tests.three_channel"
    }

    fn via(&self) -> Vec<NotificationChannelId> {
        vec![FIRST_CHANNEL, FAILING_CHANNEL, LAST_CHANNEL]
    }

    fn to_channel(&self, channel: &str, _notifiable: &dyn Notifiable) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "channel": channel }))
    }
}

#[tokio::test]
async fn channel_failure_dead_letters_independently_without_replaying_successes() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("00-test.toml"),
        r#"
            [jobs]
            max_retries = 1
            poll_interval_ms = 1

            [logging]
            log_dir = ""
        "#,
    )
    .unwrap();
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let kernel = App::builder()
        .load_config_dir(directory.path())
        .register_provider(QueueProvider {
            deliveries: deliveries.clone(),
        })
        .build_worker_kernel()
        .await
        .unwrap();
    let app = kernel.app().clone();

    app.notify_queued(&QueueRecipient, &ThreeChannelNotification)
        .await
        .unwrap();
    let worker = Worker::from_app(app.clone()).unwrap();
    assert!(worker.run_once().await.unwrap());
    assert!(worker.run_once().await.unwrap());
    assert!(worker.run_once().await.unwrap());
    assert!(!worker.run_once().await.unwrap());

    assert_eq!(deliveries.lock().unwrap().as_slice(), &["first", "last"]);
    let jobs = app.diagnostics().unwrap().snapshot().jobs;
    assert_eq!(jobs.enqueued_total, 3);
    assert_eq!(jobs.succeeded_total, 2);
    assert_eq!(jobs.dead_lettered_total, 1);
    assert_eq!(jobs.retried_total, 0);

    app.shutdown().await.unwrap();
}
