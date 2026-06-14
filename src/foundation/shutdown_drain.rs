use std::time::Duration;

const SHUTDOWN_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) struct ShutdownDrainMessages {
    pub(crate) target: ShutdownDrainTarget,
    pub(crate) timeout_disabled: &'static str,
    pub(crate) waiting: &'static str,
    pub(crate) drained: &'static str,
    pub(crate) timeout_elapsed: &'static str,
}

pub(crate) enum ShutdownDrainTarget {
    ManagedBackgroundTasks,
    Scheduler,
    Worker,
}

macro_rules! drain_info {
    ($target:expr, $($arg:tt)+) => {
        match $target {
            ShutdownDrainTarget::ManagedBackgroundTasks => {
                tracing::info!(target: "foundry::foundation::background_tasks", $($arg)+)
            }
            ShutdownDrainTarget::Scheduler => {
                tracing::info!(target: "foundry.scheduler", $($arg)+)
            }
            ShutdownDrainTarget::Worker => {
                tracing::info!(target: "foundry.worker", $($arg)+)
            }
        }
    };
}

macro_rules! drain_warn {
    ($target:expr, $($arg:tt)+) => {
        match $target {
            ShutdownDrainTarget::ManagedBackgroundTasks => {
                tracing::warn!(target: "foundry::foundation::background_tasks", $($arg)+)
            }
            ShutdownDrainTarget::Scheduler => {
                tracing::warn!(target: "foundry.scheduler", $($arg)+)
            }
            ShutdownDrainTarget::Worker => {
                tracing::warn!(target: "foundry.worker", $($arg)+)
            }
        }
    };
}

#[async_trait::async_trait]
pub(crate) trait ShutdownDrainTask: Send + Sized {
    fn is_finished(&mut self) -> bool;
    async fn wait_finished(self);
    fn abort(&self);
    async fn wait_after_abort(self);
}

pub(crate) async fn drain_tasks<T>(
    mut tasks: Vec<T>,
    timeout: Duration,
    messages: ShutdownDrainMessages,
) where
    T: ShutdownDrainTask,
{
    if tasks.is_empty() {
        return;
    }

    if timeout.is_zero() {
        drain_warn!(
            messages.target,
            active = tasks.len(),
            "{}",
            messages.timeout_disabled
        );
        abort_tasks(tasks).await;
        return;
    }

    let task_count = tasks.len();
    drain_info!(
        messages.target,
        active = task_count,
        timeout_ms = timeout.as_millis(),
        "{}",
        messages.waiting
    );

    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    loop {
        reap_finished_tasks(&mut tasks).await;
        if tasks.is_empty() {
            drain_info!(messages.target, active = task_count, "{}", messages.drained);
            return;
        }

        tokio::select! {
            biased;
            _ = &mut deadline => {
                drain_warn!(
                    messages.target,
                    active = tasks.len(),
                    timeout_ms = timeout.as_millis(),
                    "{}",
                    messages.timeout_elapsed
                );
                abort_tasks(tasks).await;
                return;
            }
            _ = tokio::time::sleep(SHUTDOWN_DRAIN_POLL_INTERVAL) => {}
        }
    }
}

async fn reap_finished_tasks<T>(tasks: &mut Vec<T>)
where
    T: ShutdownDrainTask,
{
    let mut index = 0;
    while index < tasks.len() {
        if tasks[index].is_finished() {
            let task = tasks.swap_remove(index);
            task.wait_finished().await;
        } else {
            index += 1;
        }
    }
}

async fn abort_tasks<T>(tasks: Vec<T>)
where
    T: ShutdownDrainTask,
{
    for task in &tasks {
        task.abort();
    }

    for task in tasks {
        task.wait_after_abort().await;
    }
}
