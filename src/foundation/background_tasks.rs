use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::foundation::shutdown_drain::{
    drain_tasks, ShutdownDrainMessages, ShutdownDrainTarget, ShutdownDrainTask,
};
use crate::support::sync::lock_unpoisoned;

#[derive(Default)]
pub(crate) struct ManagedBackgroundTasks {
    shutting_down: AtomicBool,
    tasks: Mutex<Vec<ManagedBackgroundTask>>,
}

struct ManagedBackgroundTask {
    name: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    completed: tokio::sync::oneshot::Receiver<()>,
    abort: tokio::task::AbortHandle,
}

impl ManagedBackgroundTasks {
    pub(crate) fn register(
        &self,
        name: impl Into<String>,
        shutdown: tokio::sync::oneshot::Sender<()>,
        completed: tokio::sync::oneshot::Receiver<()>,
        abort: tokio::task::AbortHandle,
    ) {
        let task = ManagedBackgroundTask {
            name: name.into(),
            shutdown: Some(shutdown),
            completed,
            abort,
        };

        let mut tasks = lock_unpoisoned(&self.tasks, "managed background task registry");
        if self.shutting_down.load(Ordering::SeqCst) {
            tracing::warn!(
                task = %task.name,
                "managed background task registered during shutdown; aborting"
            );
            task.abort.abort();
            return;
        }

        tasks.push(task);
    }

    pub(crate) async fn shutdown(&self, timeout: Duration) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let mut tasks = {
            let mut tasks = lock_unpoisoned(&self.tasks, "managed background task registry");
            std::mem::take(&mut *tasks)
        };

        for task in &mut tasks {
            if let Some(shutdown) = task.shutdown.take() {
                let _ = shutdown.send(());
            }
        }

        drain_tasks(
            tasks,
            timeout,
            ShutdownDrainMessages {
                target: ShutdownDrainTarget::ManagedBackgroundTasks,
                timeout_disabled:
                    "background shutdown timeout disabled; aborting managed background tasks",
                waiting: "waiting for managed background tasks during shutdown",
                drained: "managed background tasks drained",
                timeout_elapsed:
                    "background shutdown timeout elapsed; aborting managed background tasks",
            },
        )
        .await;
    }
}

#[async_trait::async_trait]
impl ShutdownDrainTask for ManagedBackgroundTask {
    fn is_finished(&mut self) -> bool {
        match self.completed.try_recv() {
            Ok(()) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => true,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => false,
        }
    }

    async fn wait_finished(self) {}

    fn abort(&self) {
        tracing::warn!(task = %self.name, "aborting managed background task");
        self.abort.abort();
    }

    async fn wait_after_abort(self) {}
}
