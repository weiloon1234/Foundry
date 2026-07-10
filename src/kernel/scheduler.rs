use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{oneshot, watch};
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
    cron_due_in_timezone, ScheduleHandler, ScheduleHook, ScheduleKind, ScheduleOptions,
    ScheduleRegistry, ScheduledTask,
};
use crate::support::lock::LockGuard;
use crate::support::runtime::RuntimeBackend;
use crate::support::sync::lock_unpoisoned;
use crate::support::{DateTime, ScheduleId, Timezone};

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

    /// Evaluate due cron expressions in the configured application timezone.
    pub async fn tick(&self) -> Result<Vec<ScheduleId>> {
        let clock = self.app.clock();
        self.tick_at_in_timezone(clock.now(), clock.timezone())
            .await
    }

    /// Acquire scheduler leadership and evaluate cron expressions in the configured app timezone.
    pub async fn run_once(&self) -> Result<Vec<ScheduleId>> {
        let clock = self.app.clock();
        self.run_once_at_in_timezone(clock.now(), clock.timezone())
            .await
    }

    /// Acquire scheduler leadership and evaluate the supplied instant with UTC cron fields.
    ///
    /// This deterministic entry point intentionally keeps its historical UTC semantics. Use
    /// [`Self::run_once`] for application-timezone scheduling.
    pub async fn run_once_at(&self, now: DateTime) -> Result<Vec<ScheduleId>> {
        self.run_once_at_in_timezone(now, &Timezone::Utc).await
    }

    async fn run_once_at_in_timezone(
        &self,
        now: DateTime,
        timezone: &Timezone,
    ) -> Result<Vec<ScheduleId>> {
        if self.ensure_leadership().await? {
            return self.tick_at_in_timezone(now, timezone).await;
        }

        Ok(Vec::new())
    }

    /// Evaluate the supplied instant with UTC cron fields without acquiring leadership.
    ///
    /// This deterministic entry point intentionally keeps its historical UTC semantics. Use
    /// [`Self::tick`] for application-timezone scheduling.
    pub async fn tick_at(&self, now: DateTime) -> Result<Vec<ScheduleId>> {
        self.tick_at_in_timezone(now, &Timezone::Utc).await
    }

    async fn tick_at_in_timezone(
        &self,
        now: DateTime,
        timezone: &Timezone,
    ) -> Result<Vec<ScheduleId>> {
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
                ScheduleKind::Cron { expression } => {
                    cron_due_in_timezone(expression, previous, now, timezone)
                }
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
            let overlap_guard = if options.without_overlapping {
                let lock_key = format!("schedule:{task_id}");
                match self
                    .app
                    .lock()?
                    .acquire_storage_key(&lock_key, options.overlap_lock_ttl)
                    .await
                {
                    Ok(Some(guard)) => Some(guard),
                    Ok(None) => {
                        tracing::debug!(
                            target: "foundry.scheduler",
                            schedule = %task_id,
                            "Skipped (previous invocation still running)"
                        );
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "foundry.scheduler",
                            schedule = %task_id,
                            error = %error,
                            "Skipped because the overlap lock could not be acquired safely"
                        );
                        return Err(Error::message(format!(
                            "failed to acquire overlap lock for schedule `{task_id}`: {error}"
                        )));
                    }
                }
            } else {
                None
            };
            let kind_label = match &task.kind {
                ScheduleKind::Cron { .. } => "cron",
                ScheduleKind::Interval { .. } => "interval",
            };

            // Spawn each task independently — no blocking the tick loop
            let diagnostics = self.app.diagnostics().ok();
            let schedule_id = task_id.clone();
            let panic_schedule_id = schedule_id.clone();
            let (cancel, cancellation) = watch::channel(false);
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
                            ScheduleExecution {
                                task_id: schedule_id,
                                kind_label,
                                app,
                                handler,
                                options,
                                overlap_guard,
                                diagnostics,
                            },
                            cancellation,
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
            self.track_active_schedule_task(handle, cancel);

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
                    // Task handlers are spawned and isolated. Leadership and overlap-lock
                    // coordination errors are recoverable and retried on the next tick.
                    if let Err(e) = self.run_once().await {
                        tracing::warn!(
                            target: "foundry.scheduler",
                            error = %e,
                            "Scheduler tick coordination error, will retry"
                        );
                    }
                }
            }
        }

        self.drain_active_tasks().await;
        self.release_leadership().await
    }

    #[cfg(test)]
    fn track_active_task(&self, handle: JoinHandle<()>) {
        lock_unpoisoned(&self.active_tasks, "scheduler active tasks")
            .push(ScheduleTaskHandle::new(handle));
    }

    fn track_active_schedule_task(&self, handle: JoinHandle<()>, cancel: watch::Sender<bool>) {
        lock_unpoisoned(&self.active_tasks, "scheduler active tasks")
            .push(ScheduleTaskHandle::cancellable(handle, cancel));
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

    async fn release_leadership(&self) -> Result<()> {
        if !self.leader_active.load(Ordering::Acquire) {
            return Ok(());
        }

        self.backend
            .release_scheduler_leadership(&self.owner_id)
            .await?;
        self.leader_active.store(false, Ordering::Release);
        if let Ok(diagnostics) = self.app.diagnostics() {
            diagnostics.set_scheduler_leader_active(false);
        }
        tracing::info!(
            target: "foundry.scheduler",
            owner = %self.owner_id,
            "Scheduler leadership released"
        );
        Ok(())
    }
}

struct ScheduleExecution {
    task_id: ScheduleId,
    kind_label: &'static str,
    app: AppContext,
    handler: ScheduleHandler,
    options: ScheduleOptions,
    overlap_guard: Option<LockGuard>,
    diagnostics: Option<Arc<RuntimeDiagnostics>>,
}

async fn run_spawned_schedule_task(
    execution: ScheduleExecution,
    mut cancellation: watch::Receiver<bool>,
) {
    let ScheduleExecution {
        task_id,
        kind_label,
        app,
        handler,
        options,
        overlap_guard,
        diagnostics,
    } = execution;
    let mut overlap_lease = overlap_guard
        .map(|guard| ScheduleOverlapLease::start(guard, task_id.clone(), options.overlap_lock_ttl));

    let stop_reason = {
        let lifecycle = run_schedule_lifecycle(
            &task_id,
            kind_label,
            &app,
            &handler,
            &options,
            diagnostics.as_ref(),
        );
        tokio::pin!(lifecycle);

        if let Some(lease) = overlap_lease.as_mut() {
            tokio::select! {
                biased;
                _ = cancellation.changed() => ScheduleStopReason::Cancelled,
                _ = &mut lease.lost => ScheduleStopReason::OverlapLeaseLost,
                _ = &mut lifecycle => ScheduleStopReason::Completed,
            }
        } else {
            tokio::select! {
                biased;
                _ = cancellation.changed() => ScheduleStopReason::Cancelled,
                _ = &mut lifecycle => ScheduleStopReason::Completed,
            }
        }
    };

    match stop_reason {
        ScheduleStopReason::Completed => {}
        ScheduleStopReason::Cancelled => tracing::debug!(
            target: "foundry.scheduler",
            schedule = %task_id,
            "Schedule cancelled during shutdown"
        ),
        ScheduleStopReason::OverlapLeaseLost => tracing::error!(
            target: "foundry.scheduler",
            schedule = %task_id,
            "Schedule cancelled because its overlap lock lease was lost"
        ),
    }

    if let Some(lease) = overlap_lease {
        lease.release(&task_id).await;
    }
}

fn overlap_heartbeat_interval(ttl: Duration) -> Duration {
    (ttl / 3).max(Duration::from_millis(100))
}

async fn run_schedule_lifecycle(
    task_id: &ScheduleId,
    kind_label: &'static str,
    app: &AppContext,
    handler: &ScheduleHandler,
    options: &ScheduleOptions,
    diagnostics: Option<&Arc<RuntimeDiagnostics>>,
) {
    if let Some(ref before) = options.before_hook {
        run_schedule_hook(task_id, app, "before", before).await;
    }

    let result = run_schedule_handler(app, handler).await;

    match &result {
        Ok(()) => {
            tracing::info!(
                target: "foundry.scheduler",
                schedule = %task_id,
                kind = kind_label,
                "Schedule executed"
            );
            if let Some(diagnostics) = diagnostics {
                diagnostics.record_schedule_executed();
            }

            if let Some(ref after) = options.after_hook {
                run_schedule_hook(task_id, app, "after", after).await;
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
                run_schedule_hook(task_id, app, "on_failure", on_failure).await;
            }
        }
    }
}

enum ScheduleStopReason {
    Completed,
    Cancelled,
    OverlapLeaseLost,
}

struct ScheduleOverlapLease {
    guard: Option<Arc<LockGuard>>,
    stop: Option<oneshot::Sender<()>>,
    lost: oneshot::Receiver<()>,
    heartbeat: Option<JoinHandle<()>>,
}

impl ScheduleOverlapLease {
    fn start(guard: LockGuard, task_id: ScheduleId, ttl: Duration) -> Self {
        let guard = Arc::new(guard);
        let heartbeat_guard = guard.clone();
        let (stop, mut stop_rx) = oneshot::channel();
        let (lost_tx, lost) = oneshot::channel();
        let heartbeat_interval = overlap_heartbeat_interval(ttl);
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => return,
                    _ = tokio::time::sleep(heartbeat_interval) => {}
                }

                match tokio::time::timeout(heartbeat_interval, heartbeat_guard.extend(ttl)).await {
                    Ok(Ok(true)) => continue,
                    Ok(Ok(false)) => tracing::warn!(
                        target: "foundry.scheduler",
                        schedule = %task_id,
                        "Schedule overlap lock heartbeat lost ownership"
                    ),
                    Ok(Err(error)) => tracing::warn!(
                        target: "foundry.scheduler",
                        schedule = %task_id,
                        error = %error,
                        "Schedule overlap lock heartbeat failed"
                    ),
                    Err(_) => tracing::warn!(
                        target: "foundry.scheduler",
                        schedule = %task_id,
                        timeout_ms = heartbeat_interval.as_millis(),
                        "Schedule overlap lock heartbeat timed out"
                    ),
                }
                break;
            }
            let _ = lost_tx.send(());
        });

        Self {
            guard: Some(guard),
            stop: Some(stop),
            lost,
            heartbeat: Some(heartbeat),
        }
    }

    async fn release(mut self, task_id: &ScheduleId) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(heartbeat) = self.heartbeat.take() {
            if let Err(error) = heartbeat.await {
                tracing::warn!(
                    target: "foundry.scheduler",
                    schedule = %task_id,
                    error = %error,
                    "Schedule overlap lock heartbeat task failed"
                );
            }
        }

        let Some(guard) = self.guard.take() else {
            return;
        };
        let Ok(guard) = Arc::try_unwrap(guard) else {
            tracing::warn!(
                target: "foundry.scheduler",
                schedule = %task_id,
                "Schedule overlap lock could not be released synchronously"
            );
            return;
        };
        match guard.release().await {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                target: "foundry.scheduler",
                schedule = %task_id,
                "Schedule overlap lock ownership was lost before release"
            ),
            Err(error) => tracing::warn!(
                target: "foundry.scheduler",
                schedule = %task_id,
                error = %error,
                "Failed to release schedule overlap lock; the lease will expire"
            ),
        }
    }
}

impl Drop for ScheduleOverlapLease {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
    }
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
        if !self.leader_active.load(Ordering::Acquire) {
            return;
        }

        let backend = self.backend.clone();
        let owner_id = self.owner_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = backend.release_scheduler_leadership(&owner_id).await {
                    tracing::warn!(
                        target: "foundry.scheduler",
                        owner = %owner_id,
                        error = %error,
                        "Failed to release scheduler leadership from drop fallback"
                    );
                }
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

struct ScheduleTaskHandle {
    task: JoinHandle<()>,
    cancel: Option<watch::Sender<bool>>,
}

impl ScheduleTaskHandle {
    #[cfg(test)]
    fn new(task: JoinHandle<()>) -> Self {
        Self { task, cancel: None }
    }

    fn cancellable(task: JoinHandle<()>, cancel: watch::Sender<bool>) -> Self {
        Self {
            task,
            cancel: Some(cancel),
        }
    }
}

#[async_trait::async_trait]
impl ShutdownDrainTask for ScheduleTaskHandle {
    fn is_finished(&mut self) -> bool {
        self.task.is_finished()
    }

    async fn wait_finished(self) {
        if let Err(error) = self.task.await {
            tracing::warn!(
                target: "foundry.scheduler",
                error = %error,
                "Schedule task finished with join error"
            );
        }
    }

    fn abort(&self) {
        if let Some(cancel) = &self.cancel {
            let _ = cancel.send(true);
        } else {
            self.task.abort();
        }
    }

    async fn wait_after_abort(self) {
        let _ = self.task.await;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use tempfile::tempdir;
    use tokio::sync::Notify;
    use uuid::Uuid;

    use crate::foundation::Error;
    use crate::logging::ExecutionContext;
    use crate::scheduler::{CronExpression, ScheduleOptions};
    use crate::support::runtime::RuntimeBackend;
    use crate::support::{DateTime, ScheduleId};

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

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
    async fn clock_driven_ticks_use_app_timezone_while_explicit_ticks_use_utc() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("app.toml"),
            r#"
                [app]
                timezone = "Asia/Kuala_Lumpur"
            "#,
        )
        .unwrap();

        let kernel = crate::App::builder()
            .load_config_dir(directory.path())
            .register_schedule(|registry| {
                registry.daily_at(
                    ScheduleId::new("scheduler.app-timezone"),
                    "08:00",
                    |_| async { Ok(()) },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();
        let clock = kernel.app().clock();

        let local_due = kernel
            .tick_at_in_timezone(schedule_time("2026-04-08T00:00:00Z"), clock.timezone())
            .await
            .unwrap();
        let explicit_not_due = kernel
            .tick_at(schedule_time("2026-04-08T00:00:00Z"))
            .await
            .unwrap();
        let explicit_due = kernel
            .tick_at(schedule_time("2026-04-08T08:00:00Z"))
            .await
            .unwrap();

        assert_eq!(local_due, vec![ScheduleId::new("scheduler.app-timezone")]);
        assert!(explicit_not_due.is_empty());
        assert_eq!(
            explicit_due,
            vec![ScheduleId::new("scheduler.app-timezone")]
        );
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
    async fn scheduler_run_awaits_leadership_release() {
        let dir = tempdir().unwrap();
        let namespace = format!("scheduler-release-{}", Uuid::now_v7());
        fs::write(
            dir.path().join("scheduler.toml"),
            format!(
                r#"
                [redis]
                namespace = "{namespace}"

                [scheduler]
                leader_lease_ttl_ms = 60000
                "#
            ),
        )
        .unwrap();

        let kernel_one = crate::App::builder()
            .load_config_dir(dir.path())
            .build_scheduler_kernel()
            .await
            .unwrap();
        let kernel_two = crate::App::builder()
            .load_config_dir(dir.path())
            .build_scheduler_kernel()
            .await
            .unwrap();
        let first_app = kernel_one.app().clone();

        assert!(kernel_one.ensure_leadership().await.unwrap());
        assert!(
            first_app
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .leader_active
        );

        let backend = first_app.resolve::<RuntimeBackend>().unwrap();
        let runtime = match backend.as_ref() {
            RuntimeBackend::Memory(runtime) => runtime.clone(),
            RuntimeBackend::Redis(_) => unreachable!("test uses memory runtime"),
        };
        let leadership = runtime.scheduler_leader.lock().await;
        let mut run = tokio::spawn(kernel_one.run_until(async {}));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut run)
                .await
                .is_err(),
            "run_until must await leadership release"
        );
        drop(leadership);
        run.await.unwrap().unwrap();

        assert!(
            !first_app
                .diagnostics()
                .unwrap()
                .snapshot()
                .scheduler
                .leader_active
        );
        assert!(kernel_two.ensure_leadership().await.unwrap());
        kernel_two.release_leadership().await.unwrap();
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
    async fn overlap_lock_heartbeat_keeps_long_schedule_owned() {
        let entered = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let handled = Arc::new(AtomicUsize::new(0));
        let schedule_id = ScheduleId::new("scheduler.overlap.heartbeat");
        let registered_id = schedule_id.clone();
        let schedule_entered = entered.clone();
        let schedule_finish = finish.clone();
        let schedule_handled = handled.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let entered = schedule_entered.clone();
                let finish = schedule_finish.clone();
                let handled = schedule_handled.clone();
                registry.cron_with_options(
                    registered_id.clone(),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().without_overlapping_for(Duration::from_secs(1)),
                    move |_| {
                        let entered = entered.clone();
                        let finish = finish.clone();
                        let handled = handled.clone();
                        async move {
                            handled.fetch_add(1, Ordering::SeqCst);
                            entered.notify_one();
                            finish.notified().await;
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
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(1_250)).await;
        let contender = kernel
            .app()
            .lock()
            .unwrap()
            .acquire_storage_key(&format!("schedule:{schedule_id}"), Duration::from_secs(1))
            .await
            .unwrap();
        assert!(
            contender.is_none(),
            "heartbeat must retain the lock after its original lease duration"
        );
        assert_eq!(handled.load(Ordering::SeqCst), 1);

        finish.notify_one();
        kernel.drain_active_tasks().await;
        let released = kernel
            .app()
            .lock()
            .unwrap()
            .acquire_storage_key(&format!("schedule:{schedule_id}"), Duration::from_secs(1))
            .await
            .unwrap()
            .expect("schedule completion should release the overlap lock");
        assert!(released.release().await.unwrap());
    }

    #[tokio::test]
    async fn overlap_lock_loss_cancels_protected_schedule() {
        let entered = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let schedule_id = ScheduleId::new("scheduler.overlap.lost");
        let registered_id = schedule_id.clone();
        let schedule_entered = entered.clone();
        let schedule_dropped = dropped.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let entered = schedule_entered.clone();
                let dropped = schedule_dropped.clone();
                registry.cron_with_options(
                    registered_id.clone(),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().without_overlapping_for(Duration::from_secs(1)),
                    move |_| {
                        let entered = entered.clone();
                        let dropped = dropped.clone();
                        async move {
                            let _drop_flag = DropFlag(dropped);
                            entered.notify_one();
                            std::future::pending::<()>().await;
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();
        let backend = kernel.app().resolve::<RuntimeBackend>().unwrap();
        let lock_key = format!("schedule:{schedule_id}");

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .unwrap();
        backend
            .set_value(&lock_key, "replacement-owner", 60)
            .await
            .unwrap();

        wait_for_flag(&dropped).await;
        kernel.drain_active_tasks().await;
        assert_eq!(
            backend.get_value(&lock_key).await.unwrap().as_deref(),
            Some("replacement-owner")
        );
        backend.del_key(&lock_key).await.unwrap();
    }

    #[tokio::test]
    async fn scheduler_shutdown_awaits_owner_safe_overlap_release() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("scheduler.toml"),
            r#"
            [scheduler]
            shutdown_timeout_ms = 0
            "#,
        )
        .unwrap();
        let entered = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let schedule_id = ScheduleId::new("scheduler.overlap.shutdown");
        let registered_id = schedule_id.clone();
        let schedule_entered = entered.clone();
        let schedule_dropped = dropped.clone();
        let kernel = crate::App::builder()
            .load_config_dir(dir.path())
            .register_schedule(move |registry| {
                let entered = schedule_entered.clone();
                let dropped = schedule_dropped.clone();
                registry.cron_with_options(
                    registered_id.clone(),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().without_overlapping(),
                    move |_| {
                        let entered = entered.clone();
                        let dropped = dropped.clone();
                        async move {
                            let _drop_flag = DropFlag(dropped);
                            entered.notify_one();
                            std::future::pending::<()>().await;
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();
        let backend = kernel.app().resolve::<RuntimeBackend>().unwrap();
        let runtime = match backend.as_ref() {
            RuntimeBackend::Memory(runtime) => runtime.clone(),
            RuntimeBackend::Redis(_) => unreachable!("test uses memory runtime"),
        };
        let lock_key = format!("schedule:{schedule_id}");

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .unwrap();

        let stored_locks = runtime.unique_keys.lock().await;
        let mut drain = Box::pin(kernel.drain_active_tasks());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut drain)
                .await
                .is_err(),
            "shutdown must remain pending until the overlap release completes"
        );
        drop(stored_locks);
        drain.await;

        assert!(dropped.load(Ordering::SeqCst));
        assert!(!backend.key_exists(&lock_key).await.unwrap());
    }

    #[tokio::test]
    async fn overlap_skip_is_not_reported_as_started() {
        let handled = Arc::new(AtomicUsize::new(0));
        let schedule_id = ScheduleId::new("scheduler.overlap.return-value");
        let registered_id = schedule_id.clone();
        let schedule_handled = handled.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let handled = schedule_handled.clone();
                registry.cron_with_options(
                    registered_id.clone(),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().without_overlapping(),
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
        let held = kernel
            .app()
            .lock()
            .unwrap()
            .acquire_storage_key(&format!("schedule:{schedule_id}"), Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();

        let started = kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();

        assert!(started.is_empty());
        assert_eq!(handled.load(Ordering::SeqCst), 0);
        assert!(held.release().await.unwrap());
    }

    #[tokio::test]
    async fn stale_schedule_owner_does_not_delete_replacement_lock() {
        let entered = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let schedule_id = ScheduleId::new("scheduler.overlap.stale-owner");
        let registered_id = schedule_id.clone();
        let schedule_entered = entered.clone();
        let schedule_finish = finish.clone();
        let kernel = crate::App::builder()
            .register_schedule(move |registry| {
                let entered = schedule_entered.clone();
                let finish = schedule_finish.clone();
                registry.cron_with_options(
                    registered_id.clone(),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().without_overlapping(),
                    move |_| {
                        let entered = entered.clone();
                        let finish = finish.clone();
                        async move {
                            entered.notify_one();
                            finish.notified().await;
                            Ok(())
                        }
                    },
                )?;
                Ok(())
            })
            .build_scheduler_kernel()
            .await
            .unwrap();
        let backend = kernel.app().resolve::<RuntimeBackend>().unwrap();
        let lock_key = format!("schedule:{schedule_id}");

        kernel
            .tick_at(schedule_time("2026-04-08T12:00:00Z"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .unwrap();
        backend
            .set_value(&lock_key, "replacement-owner", 60)
            .await
            .unwrap();

        finish.notify_one();
        kernel.drain_active_tasks().await;

        assert_eq!(
            backend.get_value(&lock_key).await.unwrap().as_deref(),
            Some("replacement-owner")
        );
        backend.del_key(&lock_key).await.unwrap();
    }

    #[tokio::test]
    async fn overlap_lock_backend_error_skips_schedule() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let close_connection = tokio::spawn(async move {
            let (connection, _) = listener.accept().await.unwrap();
            drop(connection);
        });
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("redis.toml"),
            format!(
                r#"
                [redis]
                url = "redis://{address}/"
                namespace = "scheduler-failing-lock-{}"
                "#,
                Uuid::now_v7()
            ),
        )
        .unwrap();

        let handled = Arc::new(AtomicUsize::new(0));
        let schedule_handled = handled.clone();
        let kernel = crate::App::builder()
            .load_config_dir(dir.path())
            .register_schedule(move |registry| {
                let handled = schedule_handled.clone();
                registry.cron_with_options(
                    ScheduleId::new("scheduler.overlap.backend-error"),
                    CronExpression::every_minute()?,
                    ScheduleOptions::new().without_overlapping(),
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
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            kernel.tick_at(schedule_time("2026-04-08T12:00:00Z")),
        )
        .await
        .unwrap()
        .unwrap_err();
        close_connection.await.unwrap();

        assert!(error.to_string().contains("failed to acquire overlap lock"));
        assert_eq!(handled.load(Ordering::SeqCst), 0);
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
