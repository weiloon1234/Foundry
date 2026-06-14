use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use foundry::prelude::*;
use tempfile::tempdir;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        pub const AUDIT_JOB: JobId = JobId::new("audit.job");
        pub const INTERVAL_SCHEDULE: ScheduleId = ScheduleId::new("interval.dispatch");
        pub const CRON_SCHEDULE: ScheduleId = ScheduleId::new("cron.dispatch");
    }

    pub mod domain {
        use super::*;

        #[derive(Debug, Serialize, Deserialize)]
        pub struct AuditJob {
            pub marker: String,
            pub sleep_ms: u64,
        }

        #[async_trait]
        impl Job for AuditJob {
            const ID: JobId = ids::AUDIT_JOB;
            const QUEUE: Option<QueueId> = Some(QueueId::new("default"));

            async fn handle(&self, context: JobContext) -> Result<()> {
                let log = context.app().resolve::<Mutex<Vec<String>>>()?;
                log.lock()
                    .unwrap()
                    .push(format!("start:{}:{}", self.marker, context.attempt()));
                drop(log);

                if self.sleep_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
                }

                let log = context.app().resolve::<Mutex<Vec<String>>>()?;
                log.lock()
                    .unwrap()
                    .push(format!("done:{}:{}", self.marker, context.attempt()));
                Ok(())
            }

            fn backoff(&self, _attempt: u32) -> Duration {
                Duration::from_millis(10)
            }
        }
    }

    pub mod providers {
        use super::*;

        #[derive(Clone)]
        pub struct AppServiceProvider {
            pub log: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.singleton_arc(self.log.clone())?;
                registrar.register_job::<domain::AuditJob>()?;
                Ok(())
            }
        }
    }

    pub mod schedules {
        use super::*;

        pub fn register_interval(registry: &mut ScheduleRegistry) -> Result<()> {
            registry.interval(
                ids::INTERVAL_SCHEDULE,
                Duration::from_millis(50),
                |invocation| async move {
                    invocation
                        .app()
                        .jobs()?
                        .dispatch(domain::AuditJob {
                            marker: "scheduled".to_string(),
                            sleep_ms: 0,
                        })
                        .await
                },
            )?;
            Ok(())
        }

        pub fn register_cron(registry: &mut ScheduleRegistry) -> Result<()> {
            registry.cron(
                ids::CRON_SCHEDULE,
                CronExpression::parse("*/1 * * * * *")?,
                |invocation| async move {
                    invocation
                        .app()
                        .jobs()?
                        .dispatch(domain::AuditJob {
                            marker: "cron".to_string(),
                            sleep_ms: 0,
                        })
                        .await
                },
            )?;
            Ok(())
        }
    }
}

fn write_runtime_config(dir: &Path, namespace: &str) {
    fs::write(
        dir.join("00-runtime.toml"),
        format!(
            r#"
            [redis]
            namespace = "{namespace}"

            [jobs]
            queue = "default"
            max_retries = 3
            poll_interval_ms = 10
            lease_ttl_ms = 60
            requeue_batch_size = 8

            [scheduler]
            tick_interval_ms = 20
            leader_lease_ttl_ms = 80
        "#
        ),
    )
    .unwrap();
}

fn build_app(config_dir: &Path, log: Arc<Mutex<Vec<String>>>) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::AppServiceProvider { log })
}

async fn wait_for_log(log: &Arc<Mutex<Vec<String>>>, prefix: &str) {
    for _ in 0..80 {
        if log
            .lock()
            .unwrap()
            .iter()
            .any(|entry| entry.starts_with(prefix))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("log entry with prefix `{prefix}` not observed");
}

async fn wait_for_worker_job(app: &AppContext) {
    for _ in 0..80 {
        if Worker::from_app(app.clone())
            .unwrap()
            .run_once()
            .await
            .unwrap()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("worker did not observe a runnable job");
}

#[tokio::test]
async fn run_worker_async_processes_jobs_through_worker_kernel() {
    let config_dir = tempdir().unwrap();
    write_runtime_config(config_dir.path(), "distributed-worker-run");
    let log = Arc::new(Mutex::new(Vec::new()));

    let worker = build_app(config_dir.path(), log.clone())
        .build_worker_kernel()
        .await
        .unwrap();
    let app = worker.app().clone();
    app.jobs()
        .unwrap()
        .dispatch(app::domain::AuditJob {
            marker: "direct".to_string(),
            sleep_ms: 0,
        })
        .await
        .unwrap();

    let worker_task = tokio::spawn(async move { worker.run().await.unwrap() });
    wait_for_log(&log, "done:direct").await;

    let snapshot = app.diagnostics().unwrap().snapshot();
    assert_eq!(snapshot.jobs.enqueued_total, 1);
    assert!(snapshot.jobs.leased_total >= 1);
    assert!(snapshot.jobs.started_total >= 1);
    assert!(snapshot.jobs.succeeded_total >= 1);

    worker_task.abort();
}

#[tokio::test]
async fn aborting_worker_run_does_not_duplicate_in_flight_jobs() {
    let config_dir = tempdir().unwrap();
    write_runtime_config(config_dir.path(), "distributed-worker-recovery");
    let shared_log = Arc::new(Mutex::new(Vec::new()));

    let worker_one = build_app(config_dir.path(), shared_log.clone())
        .build_worker_kernel()
        .await
        .unwrap();
    let app_one = worker_one.app().clone();
    app_one
        .jobs()
        .unwrap()
        .dispatch(app::domain::AuditJob {
            marker: "recover".to_string(),
            sleep_ms: 200,
        })
        .await
        .unwrap();

    let worker_one_task = tokio::spawn(async move { worker_one.run().await.unwrap() });
    wait_for_log(&shared_log, "start:recover").await;
    worker_one_task.abort();

    let worker_two = build_app(config_dir.path(), shared_log.clone())
        .build_worker_kernel()
        .await
        .unwrap();
    let app_two = worker_two.app().clone();
    let worker_two_task = tokio::spawn(async move { worker_two.run().await.unwrap() });

    wait_for_log(&shared_log, "done:recover").await;
    tokio::time::sleep(Duration::from_millis(120)).await;

    let entries = shared_log.lock().unwrap().clone();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.starts_with("done:recover"))
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.starts_with("start:recover"))
            .count(),
        2
    );

    assert!(!Worker::from_app(app_two.clone())
        .unwrap()
        .run_once()
        .await
        .unwrap());

    let snapshot = app_two.diagnostics().unwrap().snapshot();
    assert_eq!(
        app_one
            .diagnostics()
            .unwrap()
            .snapshot()
            .jobs
            .succeeded_total,
        0
    );
    assert!(snapshot.jobs.succeeded_total >= 1);

    worker_two_task.abort();
}

#[tokio::test]
async fn run_scheduler_async_enqueues_jobs_that_run_worker_async_processes() {
    let config_dir = tempdir().unwrap();
    write_runtime_config(config_dir.path(), "distributed-scheduler-worker");
    let log = Arc::new(Mutex::new(Vec::new()));

    let worker_task = tokio::spawn({
        let builder = build_app(config_dir.path(), log.clone());
        async move { builder.run_worker_async().await.unwrap() }
    });
    let scheduler_task = tokio::spawn({
        let builder = build_app(config_dir.path(), log.clone())
            .register_schedule(app::schedules::register_interval);
        async move { builder.run_scheduler_async().await.unwrap() }
    });

    wait_for_log(&log, "done:scheduled").await;

    worker_task.abort();
    scheduler_task.abort();
}

#[tokio::test]
async fn only_one_scheduler_kernel_executes_when_sharing_a_backend() {
    let config_dir = tempdir().unwrap();
    write_runtime_config(config_dir.path(), "distributed-scheduler-leadership");
    let log = Arc::new(Mutex::new(Vec::new()));

    let scheduler_one = build_app(config_dir.path(), log.clone())
        .register_schedule(app::schedules::register_cron)
        .build_scheduler_kernel()
        .await
        .unwrap();
    let scheduler_two = build_app(config_dir.path(), log.clone())
        .register_schedule(app::schedules::register_cron)
        .build_scheduler_kernel()
        .await
        .unwrap();

    let now = DateTime::parse("2026-04-09T12:00:00Z").unwrap();
    let executed_one = scheduler_one.run_once_at(now).await.unwrap();
    let executed_two = scheduler_two.run_once_at(now).await.unwrap();

    assert_eq!(executed_one, vec![app::ids::CRON_SCHEDULE]);
    assert!(executed_two.is_empty());

    wait_for_worker_job(scheduler_one.app()).await;
    wait_for_log(&log, "done:cron").await;

    let snapshot_one = scheduler_one.app().diagnostics().unwrap().snapshot();
    let snapshot_two = scheduler_two.app().diagnostics().unwrap().snapshot();
    assert_eq!(snapshot_one.scheduler.leadership_acquired_total, 1);
    assert!(snapshot_one.scheduler.leader_active);
    assert_eq!(snapshot_two.scheduler.leadership_acquired_total, 0);
}
