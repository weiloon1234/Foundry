use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::foundation::shutdown_drain::{
    drain_tasks, ShutdownDrainMessages, ShutdownDrainTarget, ShutdownDrainTask,
};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{
    catch_async_panic, catch_future_panic, panic_payload_message, RuntimeDiagnostics,
    SchedulerLeadershipState,
};
use crate::scheduler::{
    cron_due, ScheduleHandler, ScheduleHook, ScheduleKind, ScheduleOptions, ScheduleRegistry,
    ScheduledTask,
};
use crate::support::runtime::RuntimeBackend;
use crate::support::sync::lock_unpoisoned;
use crate::support::{DateTime, ScheduleId};

pub struct SchedulerKernel {
    app: AppContext,
    tasks: Vec<ScheduledTask>,
    backend: RuntimeBackend,
    tick_interval: Duration,
    leader_lease_ttl: Duration,
    shutdown_timeout: Duration,
    owner_id: String,
    leader_active: AtomicBool,
    last_tick: Mutex<Option<DateTime>>,
    last_interval_run: Mutex<HashMap<ScheduleId, DateTime>>,
    active_tasks: Mutex<Vec<ScheduleTaskHandle>>,
}

impl SchedulerKernel {
    pub fn new(app: AppContext, registry: ScheduleRegistry) -> Result<Self> {
        let backend = app.resolve::<RuntimeBackend>()?.as_ref().clone();
        let config = app.config().scheduler()?;
        Ok(Self {
            app,
            tasks: registry.tasks(),
            backend,
            tick_interval: Duration::from_millis(config.tick_interval_ms.max(1)),
            leader_lease_ttl: Duration::from_millis(config.leader_lease_ttl_ms.max(1)),
            shutdown_timeout: Duration::from_millis(config.shutdown_timeout_ms),
            owner_id: next_owner_id(),
            leader_active: AtomicBool::new(false),
            last_tick: Mutex::new(None),
            last_interval_run: Mutex::new(HashMap::new()),
            active_tasks: Mutex::new(Vec::new()),
        })
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub async fn tick(&self) -> Result<Vec<ScheduleId>> {
        self.tick_at(self.app.clock().now()).await
    }

    pub async fn run_once(&self) -> Result<Vec<ScheduleId>> {
        self.run_once_at(self.app.clock().now()).await
    }

    pub async fn run_once_at(&self, now: DateTime) -> Result<Vec<ScheduleId>> {
        if self.ensure_leadership().await? {
            return self.tick_at(now).await;
        }

        Ok(Vec::new())
    }

    pub async fn tick_at(&self, now: DateTime) -> Result<Vec<ScheduleId>> {
        self.prune_finished_tasks().await;

        if let Ok(diagnostics) = self.app.diagnostics() {
            diagnostics.record_scheduler_tick();
        }
        let previous = {
            let mut last_tick = lock_unpoisoned(&self.last_tick, "scheduler tick");
            let previous = last_tick.unwrap_or_else(|| now.sub_seconds(1));
            *last_tick = Some(now);
            previous
        };

        // Check current environment for per-task environment filtering
        let current_env = self
            .app
            .config()
            .app()
            .map(|c| c.environment.to_string())
            .unwrap_or_else(|_| "development".to_string());

        let mut executed = Vec::new();
        for task in &self.tasks {
            let is_due = match &task.kind {
                ScheduleKind::Cron { expression } => cron_due(expression, previous, now),
                ScheduleKind::Interval { every } => {
                    interval_due(&self.last_interval_run, &task.id, *every, now)
                }
            };

            if !is_due {
                continue;
            }

            // Environment filter
            if !task.options.environments.is_empty()
                && !task.options.environments.iter().any(|e| e == &current_env)
            {
                continue;
            }

            let task_id = task.id.clone();
            let app = self.app.clone();
            let handler = task.handler.clone();
            let options = task.options.clone();
            let backend = self.backend.clone();
            let kind_label = match &task.kind {
                ScheduleKind::Cron { .. } => "cron",
                ScheduleKind::Interval { .. } => "interval",
            };

            // Spawn each task independently — no blocking the tick loop
            let diagnostics = self.app.diagnostics().ok();
            let schedule_id = task_id.clone();
            let panic_schedule_id = schedule_id.clone();
            let handle = tokio::spawn(async move {
                let execution_id = schedule_id.to_string();
                let trace_context = crate::logging::TraceContext::generated().with_parent(Some(
                    crate::logging::TraceParent::new("scheduler", execution_id.clone()),
                ));
                let result = catch_future_panic(crate::logging::scope_current_trace(
                    trace_context,
                    crate::logging::scope_current_execution(
                        crate::logging::ExecutionContext::Scheduler { id: execution_id },
                        run_spawned_schedule_task(
                            schedule_id,
                            kind_label,
                            app,
                            handler,
                            options,
                            backend,
                            diagnostics,
                        ),
                    ),
                ))
                .await;

                if let Err(panic) = result {
                    tracing::error!(
                        target: "foundry.scheduler",
                        schedule = %panic_schedule_id,
                        panic = %panic_payload_message(panic),
                        "Schedule task panicked outside scheduler execution boundary"
                    );
                }
            });
            self.track_active_task(handle);

            executed.push(task_id);
        }

        Ok(executed)
    }

    pub async fn run(self) -> Result<()> {
        self.run_until(super::shutdown::shutdown_signal()).await
    }

    async fn run_until<S>(self, shutdown: S) -> Result<()>
    where
        S: Future<Output = ()>,
    {
        let mut interval = tokio::time::interval(self.tick_interval);
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => {
                    tracing::info!(target: "foundry.scheduler", "scheduler shutdown requested");
                    break;
                }
                _ = interval.tick() => {
                    // Error from run_once is only from leadership — not from tasks
                    // (tasks are spawned and isolated). Leadership errors are recoverable.
                    if let Err(e) = self.run_once().await {
                        tracing::warn!(
                            target: "foundry.scheduler",
                            error = %e,
                            "Scheduler tick error (leadership), will retry"
                        );
                    }
                }
            }
        }

        self.drain_active_tasks().await;
        Ok(())
    }

    fn track_active_task(&self, handle: JoinHandle<()>) {
        lock_unpoisoned(&self.active_tasks, "scheduler active tasks")
            .push(ScheduleTaskHandle(handle));
    }

    async fn prune_finished_tasks(&self) {
        let mut finished = Vec::new();
        {
            let mut active_tasks = lock_unpoisoned(&self.active_tasks, "scheduler active tasks");
            let mut index = 0;
            while index < active_tasks.len() {
                if active_tasks[index].is_finished() {
                    finished.push(active_tasks.swap_remove(index));
                } else {
                    index += 1;
                }
            }
        }

        for handle in finished {
            handle.wait_finished().await;
        }
    }

    async fn drain_active_tasks(&self) {
        let active_tasks = {
            let mut active_tasks = lock_unpoisoned(&self.active_tasks, "scheduler active tasks");
            std::mem::take(&mut *active_tasks)
        };

        drain_tasks(
            active_tasks,
            self.shutdown_timeout,
            ShutdownDrainMessages {
                target: ShutdownDrainTarget::Scheduler,
                timeout_disabled:
                    "scheduler shutdown timeout disabled; aborting active schedule tasks",
                waiting: "waiting for active schedule tasks during shutdown",
                drained: "active schedule tasks drained",
                timeout_elapsed:
                    "scheduler shutdown timeout elapsed; aborting active schedule tasks",
            },
        )
        .await;
    }

    async fn ensure_leadership(&self) -> Result<bool> {
        // Acquire/Release: this flag gates job-execution authority, so reads
        // must observe the writes that accompanied the leadership change.
        let leader_active = self.leader_active.load(Ordering::Acquire);
        if leader_active {
            if self
                .backend
                .renew_scheduler_leadership(&self.owner_id, self.leader_lease_ttl)
                .await?
            {
                self.leader_active.store(true, Ordering::Release);
                if let Ok(diagnostics) = self.app.diagnostics() {
                    diagnostics.set_scheduler_leader_active(true);
                }
                return Ok(true);
            }

            self.leader_active.store(false, Ordering::Release);
            tracing::warn!(
                target: "foundry.scheduler",
                state = "lost",
                owner = %self.owner_id,
                "Scheduler leadership lost"
            );
            if let Ok(diagnostics) = self.app.diagnostics() {
                diagnostics.record_scheduler_leadership(SchedulerLeadershipState::Lost);
            }
            return Ok(false);
        }

        if self
            .backend
            .try_acquire_scheduler_leadership(&self.owner_id, self.leader_lease_ttl)
            .await?
        {
            self.leader_active.store(true, Ordering::Release);
            tracing::info!(
                target: "foundry.scheduler",
                state = "acquired",
                owner = %self.owner_id,
                "Scheduler leadership acquired"
            );
            if let Ok(diagnostics) = self.app.diagnostics() {
                diagnostics.record_scheduler_leadership(SchedulerLeadershipState::Acquired);
            }
            return Ok(true);
        }

        self.leader_active.store(false, Ordering::Release);
        if let Ok(diagnostics) = self.app.diagnostics() {
            diagnostics.set_scheduler_leader_active(false);
        }
        Ok(false)
    }
}

async fn run_spawned_schedule_task(
    task_id: ScheduleId,
    kind_label: &'static str,
    app: AppContext,
    handler: ScheduleHandler,
    options: ScheduleOptions,
    backend: RuntimeBackend,
    diagnostics: Option<Arc<RuntimeDiagnostics>>,
) {
    let _lock_guard = if options.without_overlapping {
        let lock_key = format!("schedule:{task_id}");
        match backend.set_nx_value(&lock_key, "1", 3600).await {
            Ok(true) => Some(ScheduleLockGuard {
                backend: backend.clone(),
                key: lock_key,
            }),
            Ok(false) => {
                tracing::debug!(
                    target: "foundry.scheduler",
                    schedule = %task_id,
                    "Skipped (previous invocation still running)"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    target: "foundry.scheduler",
                    schedule = %task_id,
                    error = %error,
                    "Failed to acquire overlap lock, running anyway"
                );
                None
            }
        }
    } else {
        None
    };

    if let Some(ref before) = options.before_hook {
        run_schedule_hook(&task_id, &app, "before", before).await;
    }

    let result = run_schedule_handler(&app, &handler).await;

    match &result {
        Ok(()) => {
            tracing::info!(
                target: "foundry.scheduler",
                schedule = %task_id,
                kind = kind_label,
                "Schedule executed"
            );
            if let Some(ref diagnostics) = diagnostics {
                diagnostics.record_schedule_executed();
            }

            if let Some(ref after) = options.after_hook {
                run_schedule_hook(&task_id, &app, "after", after).await;
            }
        }
        Err(error) => {
            tracing::error!(
                target: "foundry.scheduler",
                schedule = %task_id,
                kind = kind_label,
                error = %error,
                "Schedule failed"
            );

            if let Some(ref on_failure) = options.on_failure {
                run_schedule_hook(&task_id, &app, "on_failure", on_failure).await;
            }
        }
    }

    drop(_lock_guard);
}

async fn run_schedule_handler(app: &AppContext, handler: &ScheduleHandler) -> Result<()> {
    match catch_async_panic(|| handler(app.clone())).await {
        Ok(result) => result,
        Err(panic) => Err(Error::message(format!(
            "schedule panicked: {}",
            panic_payload_message(panic)
        ))),
    }
}

async fn run_schedule_hook(
    task_id: &ScheduleId,
    app: &AppContext,
    hook: &'static str,
    callback: &ScheduleHook,
) {
    match catch_async_panic(|| callback(app.clone())).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(
                target: "foundry.scheduler",
                schedule = %task_id,
                hook = hook,
                error = %error,
                "Schedule hook failed"
            );
        }
        Err(panic) => {
            tracing::warn!(
                target: "foundry.scheduler",
                schedule = %task_id,
                hook = hook,
                panic = %panic_payload_message(panic),
                "Schedule hook panicked"
            );
        }
    }
}

impl Drop for SchedulerKernel {
    fn drop(&mut self) {
        let backend = self.backend.clone();
        let owner_id = self.owner_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = backend.release_scheduler_leadership(&owner_id).await;
            });
        } else {
            tracing::warn!(
                target: "foundry.scheduler",
                owner = %owner_id,
                "scheduler dropped outside a tokio runtime; leadership lease will only lapse via TTL"
            );
        }
    }
}

fn interval_due(
    state: &Mutex<HashMap<ScheduleId, DateTime>>,
    id: &ScheduleId,
    every: Duration,
    now: DateTime,
) -> bool {
    let mut state = lock_unpoisoned(state, "scheduler interval");
    match state.get(id).cloned() {
        Some(last_run) => {
            if (now.as_chrono() - last_run.as_chrono())
                .to_std()
                .map(|elapsed| elapsed >= every)
                .unwrap_or(false)
            {
                state.insert(id.clone(), now);
                true
            } else {
                false
            }
        }
        None => {
            state.insert(id.clone(), now);
            false
        }
    }
}

fn next_owner_id() -> String {
    static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
    format!(
        "scheduler-{:x}-{:x}",
        DateTime::now().timestamp_micros(),
        NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
    )
}

struct ScheduleTaskHandle(JoinHandle<()>);

#[async_trait::async_trait]
impl ShutdownDrainTask for ScheduleTaskHandle {
    fn is_finished(&mut self) -> bool {
        self.0.is_finished()
    }

    async fn wait_finished(self) {
        if let Err(error) = self.0.await {
            tracing::warn!(
                target: "foundry.scheduler",
                error = %error,
                "Schedule task finished with join error"
            );
        }
    }

    fn abort(&self) {
        self.0.abort();
    }

    async fn wait_after_abort(self) {
        let _ = self.0.await;
    }
}

/// Drop guard that releases a schedule overlap lock, even on panic.
struct ScheduleLockGuard {
    backend: RuntimeBackend,
    key: String,
}

impl Drop for ScheduleLockGuard {
    fn drop(&mut self) {
        let backend = self.backend.clone();
        let key = std::mem::take(&mut self.key);
        if !key.is_empty() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = backend.del_key(&key).await;
                });
            } else {
                tracing::warn!(
                    target: "foundry.scheduler",
                    key = %key,
                    "schedule lock guard dropped outside a tokio runtime; overlap lock will only lapse via TTL"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use tempfile::tempdir;

    use crate::foundation::Error;
    use crate::logging::ExecutionContext;
    use crate::scheduler::{CronExpression, ScheduleOptions};
    use crate::support::runtime::RuntimeBackend;
    use crate::support::{DateTime, ScheduleId};

    fn panic_result(message: &'static str) -> crate::Result<()> {
        panic!("{message}")
    }

    async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        for _ in 0..100 {
            if counter.load(Ordering::SeqCst) >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "counter did not reach {expected}; latest={}",
            counter.load(Ordering::SeqCst)
        );
    }

    async fn wait_for_flag(flag: &AtomicBool) {
        for _ in 0..100 {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("flag was not set");
    }

    async fn wait_for_missing_key(backend: &RuntimeBackend, key: &str) {
        for _ in 0..100 {
            if !backend.key_exists(key).await.unwrap() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("key `{key}` was not released");
    }

    fn schedule_time(value: &str) -> DateTime {
        DateTime::parse(value).unwrap()
    }

    #[tokio::test]
    async fn scheduler_run_exits_when_shutdown_future_completes() {
        let kernel = crate::App::builder()
            .build_scheduler_kernel()
            .await
            .unwrap();

        kernel.run_until(async {}).await.unwrap();
    }

    #[tokio::test]
    async fn scheduler_shutdown_waits_for_active_schedule_tasks() {
        let completed = Arc::new(AtomicBool::new(false));
        let kernel = crate::App::builder()
            .build_scheduler_kernel()
            .await
            .unwrap();

        let task_completed = completed.clone();
        kernel.track_active_task(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            task_completed.store(true, Ordering::SeqCst);
        }));

        kernel.drain_active_tasks().await;

        assert!(completed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn scheduler_shutdown_aborts_active_schedule_tasks_after_timeout() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("scheduler.toml"),
            r#"
            [scheduler]
            shutdown_timeout_ms = 1
            "#,
        )
        .unwrap();

        let completed = Arc::new(AtomicBool::new(false));
        let kernel = crate::App::builder()
            .load_config_dir(dir.path())
            .build_scheduler_kernel()
            .await
            .unwrap();

        let task_completed = completed.clone();
        kernel.track_active_task(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            task_completed.store(true, Ordering::SeqCst);
        }));

        tokio::time::timeout(Duration::from_millis(100), kernel.drain_active_tasks())
            .await
            .unwrap();

        assert!(!completed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn panicking_schedule_handler_runs_failure_path_without_success_diagnostics() {
        let failure_count = Arc::new(AtomicUsize::new(0));
        let schedule_failure_count = failure_count.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let failure_count = schedule_failure_count.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.panic.failure"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().on_failure(move |_| {
                        let failure_count = failure_count.clone();
                        async move {
                            failure_count.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    }),
                    |_| async { panic_result("schedule explode") },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        let executed = kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        assert_eq!(executed, vec![ScheduleId::new("scheduler.panic.failure")]);
        wait_for_count(&failure_count, 1).await;
        kernel.prune_finished_tasks().await;
        assert_eq!(
            kernel
                .app()
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .executed_schedules_total,
            0
        );
    }

    #[tokio::test]
    async fn panicking_schedule_handler_factory_runs_failure_path() {
        let failure_count = Arc::new(AtomicUsize::new(0));
        let schedule_failure_count = failure_count.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let failure_count = schedule_failure_count.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.factory-panic.failure"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().on_failure(move |_| {
                        let failure_count = failure_count.clone();
                        async move {
                            failure_count.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    }),
                    |_| -> std::future::Ready<crate::Result<()>> {
                        panic!("schedule factory explode")
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        let executed = kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        assert_eq!(
            executed,
            vec![ScheduleId::new("scheduler.factory-panic.failure")]
        );
        wait_for_count(&failure_count, 1).await;
        kernel.prune_finished_tasks().await;
        assert_eq!(
            kernel
                .app()
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .executed_schedules_total,
            0
        );
    }

    #[tokio::test]
    async fn panicking_schedule_does_not_stop_other_due_schedules() {
        let failure_count = Arc::new(AtomicUsize::new(0));
        let success_count = Arc::new(AtomicUsize::new(0));
        let schedule_failure_count = failure_count.clone();
        let schedule_success_count = success_count.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let failure_count = schedule_failure_count.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.panic.first"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().on_failure(move |_| {
                        let failure_count = failure_count.clone();
                        async move {
                            failure_count.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    }),
                    |_| async { panic_result("first schedule explode") },
                )?;

                let success_count = schedule_success_count.clone();
                registry.cron(
                    ScheduleId::new("scheduler.success.second"),
                    CronExpression::every_minute()?,
                    move |_| {
                        let success_count = success_count.clone();
                        async move {
                            success_count.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        let executed = kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        assert_eq!(
            executed,
            vec![
                ScheduleId::new("scheduler.panic.first"),
                ScheduleId::new("scheduler.success.second")
            ]
        );
        wait_for_count(&failure_count, 1).await;
        wait_for_count(&success_count, 1).await;
        kernel.prune_finished_tasks().await;
        assert_eq!(
            kernel
                .app()
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .executed_schedules_total,
            1
        );
    }

    #[tokio::test]
    async fn before_hook_panic_isolated_and_handler_still_runs() {
        let handled = Arc::new(AtomicUsize::new(0));
        let schedule_handled = handled.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let handled = schedule_handled.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.before.panic"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().before(|_| async { panic_result("before explode") }),
                    move |_| {
                        let handled = handled.clone();
                        async move {
                            handled.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        wait_for_count(&handled, 1).await;
        kernel.prune_finished_tasks().await;
        assert_eq!(
            kernel
                .app()
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .executed_schedules_total,
            1
        );
    }

    #[tokio::test]
    async fn before_hook_factory_panic_isolated_and_handler_still_runs() {
        let handled = Arc::new(AtomicUsize::new(0));
        let schedule_handled = handled.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let handled = schedule_handled.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.before.factory-panic"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().before(|_| -> std::future::Ready<crate::Result<()>> {
                        panic!("before factory explode")
                    }),
                    move |_| {
                        let handled = handled.clone();
                        async move {
                            handled.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        wait_for_count(&handled, 1).await;
        kernel.prune_finished_tasks().await;
        assert_eq!(
            kernel
                .app()
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .executed_schedules_total,
            1
        );
    }

    #[tokio::test]
    async fn after_hook_panic_isolated_after_success_diagnostics() {
        let handled = Arc::new(AtomicUsize::new(0));
        let after_entered = Arc::new(AtomicUsize::new(0));
        let schedule_handled = handled.clone();
        let schedule_after_entered = after_entered.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let handled = schedule_handled.clone();
                let after_entered = schedule_after_entered.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.after.panic"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().after(move |_| {
                        let after_entered = after_entered.clone();
                        async move {
                            after_entered.fetch_add(1, Ordering::SeqCst);
                            panic_result("after explode")
                        }
                    }),
                    move |_| {
                        let handled = handled.clone();
                        async move {
                            handled.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        wait_for_count(&handled, 1).await;
        wait_for_count(&after_entered, 1).await;
        kernel.prune_finished_tasks().await;
        assert_eq!(
            kernel
                .app()
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .executed_schedules_total,
            1
        );
    }

    #[tokio::test]
    async fn on_failure_hook_panic_isolated_after_handler_failure() {
        let failure_hook_entered = Arc::new(AtomicUsize::new(0));
        let schedule_failure_hook_entered = failure_hook_entered.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let failure_hook_entered = schedule_failure_hook_entered.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.failure-hook.panic"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().on_failure(move |_| {
                        let failure_hook_entered = failure_hook_entered.clone();
                        async move {
                            failure_hook_entered.fetch_add(1, Ordering::SeqCst);
                            panic_result("failure hook explode")
                        }
                    }),
                    |_| async { Err(Error::message("handler failed")) },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        wait_for_count(&failure_hook_entered, 1).await;
        kernel.prune_finished_tasks().await;
        assert_eq!(
            kernel
                .app()
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .executed_schedules_total,
            0
        );
    }

    #[tokio::test]
    async fn overlap_lock_releases_after_schedule_handler_panic() {
        let failure_count = Arc::new(AtomicUsize::new(0));
        let schedule_failure_count = failure_count.clone();
        let schedule_id = ScheduleId::new("scheduler.overlap.panic");
        let lock_key = format!("schedule:{schedule_id}");
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let failure_count = schedule_failure_count.clone();
                registry.cron_with_options(
                    schedule_id.clone(),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new()
                        .without_overlapping()
                        .on_failure(move |_| {
                            let failure_count = failure_count.clone();
                            async move {
                                failure_count.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            }
                        }),
                    |_| async { panic_result("overlap panic") },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();
        let backend = kernel.app().resolve::<RuntimeBackend>().unwrap();

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();
        wait_for_count(&failure_count, 1).await;
        wait_for_missing_key(&backend, &lock_key).await;

        kernel
            .tick_at(schedule_time("2026-04-08T12:01:00Z"))
            .await
            .unwrap();
        wait_for_count(&failure_count, 2).await;
        kernel.prune_finished_tasks().await;
    }

    #[tokio::test]
    async fn scheduler_execution_context_is_scoped_for_schedule_lifecycle() {
        let saw_context = Arc::new(AtomicBool::new(false));
        let saw_trace = Arc::new(AtomicBool::new(false));
        let schedule_saw_context = saw_context.clone();
        let schedule_saw_trace = saw_trace.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let saw_context = schedule_saw_context.clone();
                let saw_trace = schedule_saw_trace.clone();
                registry.cron(
                    ScheduleId::new("scheduler.context.visible"),
                    CronExpression::every_minute()?,
                    move |_| {
                        let saw_context = saw_context.clone();
                        let saw_trace = saw_trace.clone();
                        async move {
                            if matches!(
                                crate::logging::current_execution(),
                                Some(ExecutionContext::Scheduler { id })
                                    if id == "scheduler.context.visible"
                            ) {
                                saw_context.store(true, Ordering::SeqCst);
                            }
                            if crate::logging::current_trace_id()
                                .as_deref()
                                .is_some_and(|trace_id| trace_id.starts_with("foundry-trace-"))
                            {
                                saw_trace.store(true, Ordering::SeqCst);
                            }
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        wait_for_flag(&saw_context).await;
        wait_for_flag(&saw_trace).await;
        kernel.prune_finished_tasks().await;
    }
}
