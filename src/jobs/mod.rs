mod backend;

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::task::JoinHandle;

use crate::config::JobsConfig;
use crate::database::{DbType, DbValue, Query, Sql};
use crate::foundation::shutdown_drain::{
    drain_tasks, ShutdownDrainMessages, ShutdownDrainTarget, ShutdownDrainTask,
};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{
    catch_async_panic, catch_future_panic, catch_sync_panic, panic_payload_message,
    JobOutcome as RecordedJobOutcome, RuntimeDiagnostics,
};
use crate::support::runtime::RuntimeBackend;
use crate::support::{sync::lock_unpoisoned, DateTime, JobId, QueueId};

use self::backend::{ClaimedJobLease, JobToEnqueue, SuccessfulJobEffects};

const INVALID_JOB_ENVELOPE_ID: JobId = JobId::new("foundry.invalid_job_envelope");

// ---------------------------------------------------------------------------
// Job middleware
// ---------------------------------------------------------------------------

#[async_trait]
pub trait JobMiddleware: Send + Sync + 'static {
    async fn before(&self, _job_id: &JobId, _context: &JobContext) -> Result<()> {
        Ok(())
    }
    async fn after(&self, _job_id: &JobId, _context: &JobContext) -> Result<()> {
        Ok(())
    }
    async fn failed(&self, _job_id: &JobId, _context: &JobContext, _error: &str) -> Result<()> {
        Ok(())
    }

    async fn on_dead_lettered(&self, _context: &JobDeadLetterContext) -> Result<()> {
        Ok(())
    }
}

pub(crate) type JobMiddlewareRegistryHandle = Arc<Mutex<JobMiddlewareRegistryBuilder>>;

#[derive(Default)]
pub(crate) struct JobMiddlewareRegistryBuilder {
    middlewares: Vec<Arc<dyn JobMiddleware>>,
}

impl JobMiddlewareRegistryBuilder {
    pub(crate) fn shared() -> JobMiddlewareRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register(&mut self, middleware: Arc<dyn JobMiddleware>) {
        self.middlewares.push(middleware);
    }

    pub(crate) fn freeze_shared(handle: JobMiddlewareRegistryHandle) -> JobMiddlewareRegistry {
        let mut builder = lock_unpoisoned(&handle, "job middleware registry");
        JobMiddlewareRegistry {
            middlewares: std::mem::take(&mut builder.middlewares),
        }
    }
}

pub struct JobMiddlewareRegistry {
    middlewares: Vec<Arc<dyn JobMiddleware>>,
}

impl JobMiddlewareRegistry {
    async fn run_before(&self, job_id: &JobId, context: &JobContext) {
        for mw in &self.middlewares {
            match catch_async_panic(|| mw.before(job_id, context)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %job_id,
                        error = %error,
                        "job middleware before hook failed"
                    );
                }
                Err(panic) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %job_id,
                        panic = %panic_payload_message(panic),
                        "job middleware before hook panicked"
                    );
                }
            }
        }
    }

    async fn run_after(&self, job_id: &JobId, context: &JobContext) {
        for mw in &self.middlewares {
            match catch_async_panic(|| mw.after(job_id, context)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %job_id,
                        error = %error,
                        "job middleware after hook failed"
                    );
                }
                Err(panic) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %job_id,
                        panic = %panic_payload_message(panic),
                        "job middleware after hook panicked"
                    );
                }
            }
        }
    }

    async fn run_failed(&self, job_id: &JobId, context: &JobContext, err: &str) {
        for mw in &self.middlewares {
            match catch_async_panic(|| mw.failed(job_id, context, err)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %job_id,
                        error = %error,
                        "job middleware failed hook failed"
                    );
                }
                Err(panic) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %job_id,
                        panic = %panic_payload_message(panic),
                        "job middleware failed hook panicked"
                    );
                }
            }
        }
    }

    async fn run_dead_lettered(&self, context: &JobDeadLetterContext) {
        for mw in &self.middlewares {
            match catch_async_panic(|| mw.on_dead_lettered(context)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %context.class,
                        job_id = %context.id,
                        error = %error,
                        "job middleware dead-letter hook failed"
                    );
                }
                Err(panic) => {
                    tracing::warn!(
                        target: "foundry.worker",
                        job = %context.class,
                        job_id = %context.id,
                        panic = %panic_payload_message(panic),
                        "job middleware dead-letter hook panicked"
                    );
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct JobContext {
    app: AppContext,
    queue: QueueId,
    attempt: u32,
    trace: Option<crate::logging::TraceContext>,
}

impl JobContext {
    fn new(app: AppContext, queue: QueueId, attempt: u32) -> Self {
        Self::new_with_trace(app, queue, attempt, crate::logging::current_trace_context())
    }

    fn new_with_trace(
        app: AppContext,
        queue: QueueId,
        attempt: u32,
        trace: Option<crate::logging::TraceContext>,
    ) -> Self {
        Self {
            app,
            queue,
            attempt,
            trace,
        }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn queue(&self) -> &QueueId {
        &self.queue
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.trace.as_ref().map(|trace| trace.trace_id.as_str())
    }

    pub fn request_id(&self) -> Option<&str> {
        self.trace
            .as_ref()
            .and_then(|trace| trace.request_id.as_deref())
    }
}

#[derive(Clone)]
pub struct JobDeadLetterContext {
    pub class: String,
    pub id: String,
    pub attempts: u32,
    pub last_error: String,
    pub payload: serde_json::Value,
    pub app: AppContext,
}

#[async_trait]
pub trait Job: Serialize + DeserializeOwned + Send + Sync + Debug + 'static {
    const ID: JobId;
    const QUEUE: Option<QueueId> = None;

    async fn handle(&self, context: JobContext) -> Result<()>;

    /// Maximum number of *total attempts* before the job is dead-lettered
    /// (like Laravel's `tries`): `Some(1)` dead-letters on the first failure
    /// with no retry, `Some(3)` allows two retries after the initial attempt.
    /// `None` falls back to the `jobs.max_retries` config value.
    fn max_retries(&self) -> Option<u32> {
        None
    }

    fn backoff(&self, attempt: u32) -> Duration {
        match attempt {
            1 => Duration::from_secs(5),
            2 => Duration::from_secs(30),
            3 => Duration::from_secs(60),
            4 => Duration::from_secs(300),
            _ => Duration::from_secs(600),
        }
    }

    /// Maximum execution time for this job. Override for long-running jobs.
    /// Default uses the global `timeout_seconds` config (300s / 5 minutes).
    fn timeout(&self) -> Option<Duration> {
        None // None = use global config default
    }

    /// Optional rate limit for this job type.
    /// Returns `(max_per_window, window_duration)`. When the limit is
    /// exceeded the job is requeued with a short delay instead of being
    /// counted as a retry attempt.
    fn rate_limit(&self) -> Option<(u32, Duration)> {
        None
    }

    /// If set, prevents duplicate dispatch of this job type within the
    /// returned duration. A second dispatch with the same unique key
    /// inside the window is silently dropped.
    fn unique_for(&self) -> Option<Duration> {
        None
    }

    /// Key used for the uniqueness check. Defaults to the job ID when
    /// `None` is returned. Override to include payload-specific fields
    /// (e.g. a user ID) so that *different* payloads are treated as
    /// distinct jobs.
    fn unique_key(&self) -> Option<String> {
        None
    }
}

#[derive(Clone)]
pub struct JobDispatcher {
    runtime: Arc<JobRuntime>,
    diagnostics: Arc<RuntimeDiagnostics>,
    test_sink: Option<Arc<dyn JobDispatchSink>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedJobDispatch {
    pub(crate) job_id: JobId,
    pub(crate) queue: QueueId,
    pub(crate) scheduled_at: i64,
    pub(crate) payload: serde_json::Value,
}

pub(crate) trait JobDispatchSink: Send + Sync {
    fn record(&self, dispatch: RecordedJobDispatch) -> Result<()>;
}

struct UniqueJobReservation {
    key: String,
    owner: String,
    job_id: JobId,
    unique_key: String,
}

impl UniqueJobReservation {
    async fn rollback(&self, backend: &RuntimeBackend) {
        match backend.del_if_value(&self.key, &self.owner).await {
            Ok(true) => {
                tracing::debug!(
                    target: "foundry.worker",
                    job = %self.job_id,
                    unique_key = %self.unique_key,
                    "Rolled back unique job reservation after dispatch failure"
                );
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    target: "foundry.worker",
                    job = %self.job_id,
                    unique_key = %self.unique_key,
                    error = %error,
                    "Failed to roll back unique job reservation after dispatch failure"
                );
            }
        }
    }
}

impl JobDispatcher {
    pub(crate) fn new(runtime: Arc<JobRuntime>, diagnostics: Arc<RuntimeDiagnostics>) -> Self {
        Self {
            runtime,
            diagnostics,
            test_sink: None,
        }
    }

    pub(crate) fn with_test_sink(&self, sink: Arc<dyn JobDispatchSink>) -> Self {
        Self {
            runtime: self.runtime.clone(),
            diagnostics: self.diagnostics.clone(),
            test_sink: Some(sink),
        }
    }

    pub async fn dispatch<J>(&self, job: J) -> Result<()>
    where
        J: Job,
    {
        self.dispatch_at_millis(job, Utc::now().timestamp_millis())
            .await
    }

    /// Dispatch a job immediately on an explicit queue.
    ///
    /// Long-running workers must include dynamic queues in
    /// `jobs.queue_priorities`; framework-owned queues such as `email.queue`
    /// are registered automatically.
    pub async fn dispatch_on<J, Q>(&self, job: J, queue: Q) -> Result<()>
    where
        J: Job,
        Q: Into<QueueId>,
    {
        self.dispatch_at_millis_on(job, Utc::now().timestamp_millis(), Some(queue.into()))
            .await
    }

    /// Dispatch a job at an absolute Foundry timestamp.
    pub async fn dispatch_at<J>(&self, job: J, run_at: DateTime) -> Result<()>
    where
        J: Job,
    {
        self.dispatch_at_millis(job, run_at.timestamp_millis())
            .await
    }

    /// Dispatch a job at an absolute Foundry timestamp on an explicit queue.
    pub async fn dispatch_at_on<J, Q>(&self, job: J, run_at: DateTime, queue: Q) -> Result<()>
    where
        J: Job,
        Q: Into<QueueId>,
    {
        self.dispatch_at_millis_on(job, run_at.timestamp_millis(), Some(queue.into()))
            .await
    }

    /// Dispatch a job after the supplied delay.
    pub async fn dispatch_after<J>(&self, job: J, delay: Duration) -> Result<()>
    where
        J: Job,
    {
        let run_at_millis = checked_dispatch_time_after(Utc::now().timestamp_millis(), delay)?;
        self.dispatch_at_millis(job, run_at_millis).await
    }

    /// Dispatch a job after the supplied delay on an explicit queue.
    pub async fn dispatch_after_on<J, Q>(&self, job: J, delay: Duration, queue: Q) -> Result<()>
    where
        J: Job,
        Q: Into<QueueId>,
    {
        let run_at_millis = checked_dispatch_time_after(Utc::now().timestamp_millis(), delay)?;
        self.dispatch_at_millis_on(job, run_at_millis, Some(queue.into()))
            .await
    }

    /// Dispatch a job at a raw Unix epoch timestamp in milliseconds.
    pub async fn dispatch_later<J>(&self, job: J, run_at_millis: i64) -> Result<()>
    where
        J: Job,
    {
        self.dispatch_at_millis(job, run_at_millis).await
    }

    /// Dispatch a job at a raw Unix epoch timestamp on an explicit queue.
    pub async fn dispatch_later_on<J, Q>(&self, job: J, run_at_millis: i64, queue: Q) -> Result<()>
    where
        J: Job,
        Q: Into<QueueId>,
    {
        self.dispatch_at_millis_on(job, run_at_millis, Some(queue.into()))
            .await
    }

    async fn dispatch_at_millis<J>(&self, job: J, run_at_millis: i64) -> Result<()>
    where
        J: Job,
    {
        self.dispatch_at_millis_on(job, run_at_millis, None).await
    }

    async fn dispatch_at_millis_on<J>(
        &self,
        job: J,
        run_at_millis: i64,
        queue_override: Option<QueueId>,
    ) -> Result<()>
    where
        J: Job,
    {
        if queue_override
            .as_ref()
            .is_some_and(|queue| queue.as_str().trim().is_empty())
        {
            return Err(Error::message("job queue name cannot be empty"));
        }

        if let Some(sink) = &self.test_sink {
            let queue = queue_override.unwrap_or_else(|| {
                J::QUEUE
                    .clone()
                    .unwrap_or_else(|| self.runtime.config.queue.clone())
            });
            sink.record(RecordedJobDispatch {
                job_id: J::ID.clone(),
                queue,
                scheduled_at: run_at_millis,
                payload: serde_json::to_value(&job).map_err(Error::other)?,
            })?;
            return Ok(());
        }

        let mut unique_reservation = None;

        // Unique job check: skip dispatch if a duplicate exists within the window
        if let Some(unique_duration) = job.unique_for() {
            let unique_suffix = job.unique_key().unwrap_or_else(|| J::ID.to_string());
            let unique_redis_key = format!("jobs:unique:{}:{}", J::ID, unique_suffix);
            let unique_owner = next_delivery_token();
            let ttl_secs = unique_duration.as_secs().max(1);
            let is_new = self
                .runtime
                .backend
                .set_nx_value(&unique_redis_key, &unique_owner, ttl_secs)
                .await?;
            if !is_new {
                tracing::debug!(
                    target: "foundry.worker",
                    job = %J::ID,
                    unique_key = %unique_suffix,
                    "Skipping duplicate job dispatch (unique constraint)"
                );
                return Ok(());
            }

            unique_reservation = Some(UniqueJobReservation {
                key: unique_redis_key,
                owner: unique_owner,
                job_id: J::ID,
                unique_key: unique_suffix,
            });
        }

        let dispatch_result = async {
            let queue = queue_override.unwrap_or_else(|| {
                J::QUEUE
                    .clone()
                    .unwrap_or_else(|| self.runtime.config.queue.clone())
            });
            let trace = crate::logging::trace_context_for_child(
                crate::logging::current_execution_trace_parent(),
            );
            let envelope = JobEnvelope {
                job: J::ID,
                queue: queue.clone(),
                attempts: 0,
                scheduled_at: run_at_millis,
                payload: serde_json::to_value(job).map_err(Error::other)?,
                trace: Some(trace),
                batch_id: None,
                chain_remaining: None,
            };
            let payload = serde_json::to_string(&envelope).map_err(Error::other)?;
            let token = next_delivery_token();

            if run_at_millis > Utc::now().timestamp_millis() {
                self.runtime
                    .backend
                    .schedule_job(&queue, &token, &payload, run_at_millis)
                    .await?;
            } else {
                self.runtime
                    .backend
                    .enqueue_job(&queue, &token, &payload)
                    .await?;
            }

            self.diagnostics
                .record_job_outcome(RecordedJobOutcome::Enqueued);

            Ok(())
        }
        .await;

        if let Err(error) = dispatch_result {
            if let Some(reservation) = &unique_reservation {
                reservation.rollback(&self.runtime.backend).await;
            }
            return Err(error);
        }

        Ok(())
    }

    /// Start building a batch of jobs that execute concurrently with an
    /// optional completion callback.
    pub fn batch(&self, name: &str) -> JobBatchBuilder {
        JobBatchBuilder {
            dispatcher: self.clone(),
            name: name.to_string(),
            jobs: Vec::new(),
            on_complete: None,
        }
    }

    /// Start building a chain of jobs that execute sequentially.
    pub fn chain(&self) -> JobChainBuilder {
        JobChainBuilder {
            dispatcher: self.clone(),
            jobs: Vec::new(),
        }
    }
}

fn checked_dispatch_time_after(now_millis: i64, delay: Duration) -> Result<i64> {
    let delay_millis = i64::try_from(delay.as_millis())
        .map_err(|_| Error::message("job dispatch delay exceeds the supported timestamp range"))?;
    now_millis
        .checked_add(delay_millis)
        .ok_or_else(|| Error::message("job dispatch delay exceeds the supported timestamp range"))
}

// ---------------------------------------------------------------------------
// Job batching
// ---------------------------------------------------------------------------

/// Builder for dispatching a group of jobs with an optional completion callback.
pub struct JobBatchBuilder {
    dispatcher: JobDispatcher,
    name: String,
    jobs: Vec<(JobId, QueueId, serde_json::Value)>,
    on_complete: Option<(JobId, QueueId, serde_json::Value)>,
}

impl JobBatchBuilder {
    /// Add a job to the batch.
    #[allow(clippy::should_implement_trait)]
    pub fn add<J: Job>(mut self, job: J) -> Result<Self> {
        let queue = J::QUEUE
            .clone()
            .unwrap_or_else(|| self.dispatcher.runtime.config.queue.clone());
        let payload = serde_json::to_value(&job).map_err(Error::other)?;
        self.jobs.push((J::ID, queue, payload));
        Ok(self)
    }

    /// Set a callback job that fires when all batch jobs complete successfully.
    pub fn on_complete<J: Job>(mut self, job: J) -> Result<Self> {
        let queue = J::QUEUE
            .clone()
            .unwrap_or_else(|| self.dispatcher.runtime.config.queue.clone());
        let payload = serde_json::to_value(&job).map_err(Error::other)?;
        self.on_complete = Some((J::ID, queue, payload));
        Ok(self)
    }

    /// Dispatch all batch jobs. Returns the batch ID.
    pub async fn dispatch(self) -> Result<String> {
        if self.jobs.is_empty() {
            return Err(Error::message("cannot dispatch an empty batch"));
        }

        let batch_id = format!("batch-{}-{}", self.name, next_delivery_token());
        let trace = crate::logging::trace_context_for_child(
            crate::logging::current_execution_trace_parent(),
        );
        let on_complete_payload = match &self.on_complete {
            Some((job_id, queue, payload)) => {
                let envelope = JobEnvelope {
                    job: job_id.clone(),
                    queue: queue.clone(),
                    attempts: 0,
                    scheduled_at: 0,
                    payload: payload.clone(),
                    trace: Some(trace.clone()),
                    batch_id: None,
                    chain_remaining: None,
                };
                Some(serde_json::to_string(&envelope).map_err(Error::other)?)
            }
            None => None,
        };
        let on_complete_queue = self.on_complete.as_ref().map(|(_, q, _)| q.to_string());

        let now = Utc::now().timestamp_millis();
        let mut jobs = Vec::with_capacity(self.jobs.len());
        for (job_id, queue, payload) in self.jobs {
            let envelope = JobEnvelope {
                job: job_id,
                queue: queue.clone(),
                attempts: 0,
                scheduled_at: now,
                payload,
                trace: Some(trace.clone()),
                batch_id: Some(batch_id.clone()),
                chain_remaining: None,
            };
            let serialized = serde_json::to_string(&envelope).map_err(Error::other)?;
            let token = next_delivery_token();
            jobs.push(JobToEnqueue {
                queue,
                token,
                payload: serialized,
            });
        }

        let enqueued = self
            .dispatcher
            .runtime
            .backend
            .dispatch_batch(
                &batch_id,
                on_complete_payload.as_deref(),
                on_complete_queue.as_deref(),
                jobs,
            )
            .await?;

        for _ in 0..enqueued {
            self.dispatcher
                .diagnostics
                .record_job_outcome(RecordedJobOutcome::Enqueued);
        }

        tracing::info!(
            target: "foundry.worker",
            batch_id = %batch_id,
            total = enqueued,
            "Batch dispatched"
        );

        Ok(batch_id)
    }
}

// ---------------------------------------------------------------------------
// Job chaining
// ---------------------------------------------------------------------------

/// Builder for dispatching a sequence of jobs that run one after another.
pub struct JobChainBuilder {
    dispatcher: JobDispatcher,
    jobs: Vec<ChainedJob>,
}

impl JobChainBuilder {
    /// Add a job to the end of the chain.
    #[allow(clippy::should_implement_trait)]
    pub fn add<J: Job>(mut self, job: J) -> Result<Self> {
        let queue = J::QUEUE
            .clone()
            .unwrap_or_else(|| self.dispatcher.runtime.config.queue.clone());
        let payload = serde_json::to_value(&job).map_err(Error::other)?;
        self.jobs.push(ChainedJob {
            job: J::ID,
            queue,
            payload,
        });
        Ok(self)
    }

    /// Dispatch the chain. Only the first job is enqueued immediately;
    /// subsequent jobs are stored in the envelope and dispatched on success.
    pub async fn dispatch(mut self) -> Result<()> {
        if self.jobs.is_empty() {
            return Err(Error::message("cannot dispatch an empty chain"));
        }

        let first = self.jobs.remove(0);
        let remaining = if self.jobs.is_empty() {
            None
        } else {
            Some(self.jobs)
        };

        let now = Utc::now().timestamp_millis();
        let trace = crate::logging::trace_context_for_child(
            crate::logging::current_execution_trace_parent(),
        );
        let envelope = JobEnvelope {
            job: first.job,
            queue: first.queue.clone(),
            attempts: 0,
            scheduled_at: now,
            payload: first.payload,
            trace: Some(trace),
            batch_id: None,
            chain_remaining: remaining,
        };
        let serialized = serde_json::to_string(&envelope).map_err(Error::other)?;
        let token = next_delivery_token();
        self.dispatcher
            .runtime
            .backend
            .enqueue_job(&first.queue, &token, &serialized)
            .await?;
        self.dispatcher
            .diagnostics
            .record_job_outcome(RecordedJobOutcome::Enqueued);

        Ok(())
    }
}

pub struct Worker {
    app: AppContext,
    runtime: Arc<JobRuntime>,
    diagnostics: Arc<RuntimeDiagnostics>,
    history_prune: Arc<Mutex<JobHistoryPruneState>>,
    auth_prune: Arc<Mutex<AuthCredentialPruneState>>,
    upload_temp_prune: Arc<Mutex<UploadTempPruneState>>,
    attachment_orphan_prune: Arc<Mutex<AttachmentOrphanPruneState>>,
}

#[derive(Default)]
struct JobHistoryPruneState {
    last_attempt: Option<Instant>,
}

#[derive(Default)]
struct AuthCredentialPruneState {
    tokens_last_attempt: Option<Instant>,
    password_resets_last_attempt: Option<Instant>,
    email_verification_last_attempt: Option<Instant>,
}

#[derive(Default)]
struct UploadTempPruneState {
    last_attempt: Option<Instant>,
}

#[derive(Default)]
struct AttachmentOrphanPruneState {
    last_attempt: Option<Instant>,
}

#[derive(Clone, Copy)]
enum AuthCredentialPruneKind {
    Tokens,
    PasswordResets,
    EmailVerification,
}

impl Worker {
    pub fn from_app(app: AppContext) -> Result<Self> {
        let runtime = app.job_runtime()?;
        let diagnostics = app.diagnostics()?;
        Ok(Self {
            app,
            runtime,
            diagnostics,
            history_prune: Arc::new(Mutex::new(JobHistoryPruneState::default())),
            auth_prune: Arc::new(Mutex::new(AuthCredentialPruneState::default())),
            upload_temp_prune: Arc::new(Mutex::new(UploadTempPruneState::default())),
            attachment_orphan_prune: Arc::new(Mutex::new(AttachmentOrphanPruneState::default())),
        })
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    /// Run the worker. Spawns a tokio task per claimed job (goroutine-style).
    /// When `max_concurrent_jobs` is set (> 0), a semaphore bounds concurrency.
    /// When 0 (default), jobs spawn without limit — true goroutine behavior.
    pub async fn run(self) -> Result<()> {
        self.run_until(crate::kernel::shutdown::shutdown_signal())
            .await
    }

    pub(crate) async fn run_until<S>(self, shutdown: S) -> Result<()>
    where
        S: Future<Output = ()> + Send + 'static,
    {
        // 0 = unlimited (use a large semaphore that never blocks in practice)
        let max_concurrent = if self.runtime.config.max_concurrent_jobs == 0 {
            u32::MAX >> 1 // ~1 billion — effectively unlimited
        } else {
            self.runtime.config.max_concurrent_jobs as u32
        };
        let shutdown_timeout = self.runtime.shutdown_timeout();
        let worker = Arc::new(self);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent as usize));
        let active_jobs = Arc::new(ActiveWorkerJobs::new(shutdown_timeout));

        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let shutdown_handle = {
            let tx = shutdown_tx.clone();
            tokio::spawn(async move {
                shutdown.await;
                let _ = tx.send(true);
            })
        };
        let mut shutdown_rx = shutdown_tx.subscribe();

        tracing::info!(
            target: "foundry.worker",
            max_concurrent = max_concurrent,
            "worker started"
        );

        // Separate maintenance task — runs on its own timer, not on every claim
        let maintenance_worker = worker.clone();
        let mut maintenance_shutdown = shutdown_tx.subscribe();
        let maintenance_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(maintenance_worker.runtime.poll_interval());
            loop {
                tokio::select! {
                    biased;
                    _ = maintenance_shutdown.changed() => break,
                    _ = interval.tick() => {
                        let now_millis = Utc::now().timestamp_millis();
                        let _ = maintenance_worker.runtime.promote_due_jobs(now_millis).await;
                        let requeued = maintenance_worker.runtime.requeue_expired_jobs(now_millis).await.unwrap_or(0);
                        for _ in 0..requeued {
                            maintenance_worker.diagnostics.record_job_outcome(RecordedJobOutcome::ExpiredLeaseRequeued);
                        }
                        maintenance_worker.prune_job_history_if_due().await;
                        maintenance_worker.prune_auth_credentials_if_due().await;
                        maintenance_worker.prune_upload_temp_files_if_due().await;
                        maintenance_worker.prune_attachment_orphans_if_due().await;
                    }
                }
            }
        });

        loop {
            active_jobs.prune_finished().await;

            if *shutdown_rx.borrow() {
                maintenance_handle.abort();
                let _ = maintenance_handle.await;
                active_jobs.drain().await;
                tracing::info!(target: "foundry.worker", "worker stopped");
                break;
            }

            // Acquire permit before claiming — bounds concurrency
            let permit = tokio::select! {
                biased;
                _ = shutdown_rx.changed() => continue,
                permit = semaphore.clone().acquire_owned() => match permit {
                    Ok(p) => p,
                    Err(_) => break,
                }
            };

            let claim = tokio::select! {
                biased;
                _ = shutdown_rx.changed() => {
                    drop(permit);
                    continue;
                }
                claim = worker.runtime.claim_job() => claim,
            };

            match claim {
                Ok(Some(lease)) => {
                    worker
                        .diagnostics
                        .record_job_outcome(RecordedJobOutcome::Leased);
                    let w = worker.clone();
                    let handle = tokio::spawn(async move {
                        let _ = w.process_claimed_job(lease).await;
                        drop(permit);
                    });
                    active_jobs.track(handle);
                }
                Ok(None) => {
                    drop(permit);
                    Self::sleep_or_shutdown(&mut shutdown_rx, worker.runtime.poll_interval()).await;
                }
                Err(error) => {
                    drop(permit);
                    tracing::error!(target: "foundry.worker", error = %error, "claim failed");
                    Self::sleep_or_shutdown(&mut shutdown_rx, worker.runtime.poll_interval()).await;
                }
            }
        }

        shutdown_handle.abort();
        let _ = shutdown_handle.await;

        Ok(())
    }

    async fn sleep_or_shutdown(
        shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
        duration: Duration,
    ) {
        if *shutdown_rx.borrow() {
            return;
        }

        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {}
            _ = tokio::time::sleep(duration) => {}
        }
    }

    async fn prune_job_history_if_due(&self) {
        if self.runtime.config.history_retention_days == 0
            || self.runtime.config.history_prune_batch_size == 0
            || !self.job_history_prune_due()
        {
            return;
        }

        match self.prune_job_history().await {
            Ok(deleted) if deleted > 0 => {
                tracing::info!(
                    target: "foundry.worker",
                    deleted,
                    retention_days = self.runtime.config.history_retention_days,
                    "pruned job history"
                );
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "failed to prune job history"
                );
            }
        }
    }

    fn job_history_prune_due(&self) -> bool {
        let interval = Duration::from_millis(self.runtime.config.history_prune_interval_ms.max(1));
        let now = Instant::now();
        let mut state = lock_unpoisoned(&self.history_prune, "job history prune state");
        if state
            .last_attempt
            .is_some_and(|last_attempt| now.duration_since(last_attempt) < interval)
        {
            return false;
        }
        state.last_attempt = Some(now);
        true
    }

    async fn prune_job_history(&self) -> Result<u64> {
        let Ok(lock) = self.app.lock() else {
            return Ok(0);
        };
        let Some(_guard) = lock
            .acquire("jobs:history_prune", Duration::from_secs(60))
            .await?
        else {
            return Ok(0);
        };

        let db = self.app.database()?;
        if !db.is_configured() {
            return Ok(0);
        }

        db.raw_execute(
            "DELETE FROM job_history WHERE id IN (SELECT id FROM job_history WHERE created_at < NOW() - ($1::double precision * INTERVAL '1 day') ORDER BY created_at ASC LIMIT $2)",
            &[
                DbValue::Int64(self.runtime.config.history_retention_days as i64),
                DbValue::Int64(self.runtime.config.history_prune_batch_size as i64),
            ],
        )
        .await
    }

    async fn prune_auth_credentials_if_due(&self) {
        let auth = match self.app.config().auth() {
            Ok(auth) => auth,
            Err(error) => {
                tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "failed to load auth config for credential pruning"
                );
                return;
            }
        };

        if auth.tokens.prune_retention_days > 0
            && auth.tokens.prune_batch_size > 0
            && self.auth_credential_prune_due(
                AuthCredentialPruneKind::Tokens,
                auth.tokens.prune_interval_ms,
            )
        {
            match self
                .prune_personal_access_tokens(
                    auth.tokens.prune_retention_days,
                    auth.tokens.prune_batch_size,
                )
                .await
            {
                Ok(deleted) if deleted > 0 => tracing::info!(
                    target: "foundry.worker",
                    deleted,
                    retention_days = auth.tokens.prune_retention_days,
                    "pruned personal access tokens"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "failed to prune personal access tokens"
                ),
            }
        }

        if auth.password_resets.expiry_minutes > 0
            && auth.password_resets.prune_batch_size > 0
            && self.auth_credential_prune_due(
                AuthCredentialPruneKind::PasswordResets,
                auth.password_resets.prune_interval_ms,
            )
        {
            match self
                .prune_password_reset_tokens(auth.password_resets.prune_batch_size)
                .await
            {
                Ok(deleted) if deleted > 0 => tracing::info!(
                    target: "foundry.worker",
                    deleted,
                    expiry_minutes = auth.password_resets.expiry_minutes,
                    "pruned password reset tokens"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "failed to prune password reset tokens"
                ),
            }
        }

        if auth.email_verification.expiry_minutes > 0
            && auth.email_verification.prune_batch_size > 0
            && self.auth_credential_prune_due(
                AuthCredentialPruneKind::EmailVerification,
                auth.email_verification.prune_interval_ms,
            )
        {
            match self
                .prune_email_verification_tokens(auth.email_verification.prune_batch_size)
                .await
            {
                Ok(deleted) if deleted > 0 => tracing::info!(
                    target: "foundry.worker",
                    deleted,
                    expiry_minutes = auth.email_verification.expiry_minutes,
                    "pruned email verification tokens"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "failed to prune email verification tokens"
                ),
            }
        }
    }

    fn auth_credential_prune_due(&self, kind: AuthCredentialPruneKind, interval_ms: u64) -> bool {
        let interval = Duration::from_millis(interval_ms.max(1));
        let now = Instant::now();
        let mut state = lock_unpoisoned(&self.auth_prune, "auth credential prune state");
        let last_attempt = match kind {
            AuthCredentialPruneKind::Tokens => &mut state.tokens_last_attempt,
            AuthCredentialPruneKind::PasswordResets => &mut state.password_resets_last_attempt,
            AuthCredentialPruneKind::EmailVerification => {
                &mut state.email_verification_last_attempt
            }
        };
        if last_attempt.is_some_and(|last| now.duration_since(last) < interval) {
            return false;
        }
        *last_attempt = Some(now);
        true
    }

    async fn prune_personal_access_tokens(
        &self,
        retention_days: u64,
        batch_size: u64,
    ) -> Result<u64> {
        let Ok(lock) = self.app.lock() else {
            return Ok(0);
        };
        let Some(_guard) = lock
            .acquire("auth:tokens_prune", Duration::from_secs(60))
            .await?
        else {
            return Ok(0);
        };

        self.app
            .tokens()?
            .prune_limited(retention_days, batch_size)
            .await
    }

    async fn prune_password_reset_tokens(&self, batch_size: u64) -> Result<u64> {
        let Ok(lock) = self.app.lock() else {
            return Ok(0);
        };
        let Some(_guard) = lock
            .acquire("auth:password_resets_prune", Duration::from_secs(60))
            .await?
        else {
            return Ok(0);
        };

        self.app
            .password_resets()?
            .prune_expired_limited(batch_size)
            .await
    }

    async fn prune_email_verification_tokens(&self, batch_size: u64) -> Result<u64> {
        let Ok(lock) = self.app.lock() else {
            return Ok(0);
        };
        let Some(_guard) = lock
            .acquire("auth:email_verification_prune", Duration::from_secs(60))
            .await?
        else {
            return Ok(0);
        };

        self.app
            .email_verification()?
            .prune_expired_limited(batch_size)
            .await
    }

    async fn prune_upload_temp_files_if_due(&self) {
        let storage = match self.app.config().storage() {
            Ok(storage) => storage,
            Err(error) => {
                tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "failed to load storage config for upload temp pruning"
                );
                return;
            }
        };

        if storage.upload_temp_retention_seconds == 0
            || storage.upload_temp_prune_batch_size == 0
            || !self.upload_temp_prune_due(storage.upload_temp_prune_interval_ms)
        {
            return;
        }

        match crate::storage::upload::prune_stale_upload_temp_files(
            storage.upload_temp_retention_seconds,
            storage.upload_temp_prune_batch_size,
        )
        .await
        {
            Ok(deleted) if deleted > 0 => tracing::info!(
                target: "foundry.worker",
                deleted,
                retention_seconds = storage.upload_temp_retention_seconds,
                "pruned upload temp files"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                target: "foundry.worker",
                error = %error,
                "failed to prune upload temp files"
            ),
        }
    }

    fn upload_temp_prune_due(&self, interval_ms: u64) -> bool {
        let interval = Duration::from_millis(interval_ms.max(1));
        let now = Instant::now();
        let mut state = lock_unpoisoned(&self.upload_temp_prune, "upload temp prune state");
        if state
            .last_attempt
            .is_some_and(|last| now.duration_since(last) < interval)
        {
            return false;
        }
        state.last_attempt = Some(now);
        true
    }

    async fn prune_attachment_orphans_if_due(&self) {
        let storage = match self.app.config().storage() {
            Ok(storage) => storage,
            Err(error) => {
                tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "failed to load storage config for attachment orphan audit"
                );
                return;
            }
        };

        if !storage.attachment_orphan_audit_enabled
            || storage.attachment_orphan_prune_batch_size == 0
            || !self.attachment_orphan_prune_due(storage.attachment_orphan_prune_interval_ms)
        {
            return;
        }

        let options = crate::attachments::AttachmentOrphanOptions {
            disk: None,
            prefix: storage.attachment_orphan_prefix.clone(),
            limit: storage.attachment_orphan_prune_batch_size as usize,
            older_than_seconds: storage.attachment_orphan_retention_seconds,
            delete: storage.attachment_orphan_delete_enabled,
        };

        match crate::attachments::audit_attachment_orphans_with_lock(&self.app, options).await {
            Ok(Some(report)) if report.candidate_count > 0 || report.deleted_count > 0 => {
                tracing::info!(
                    target: "foundry.worker",
                    candidates = report.candidate_count,
                    deleted = report.deleted_count,
                    delete_enabled = storage.attachment_orphan_delete_enabled,
                    "attachment orphan maintenance complete"
                );
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(error) => tracing::warn!(
                target: "foundry.worker",
                error = %error,
                "failed to audit attachment orphans"
            ),
        }
    }

    fn attachment_orphan_prune_due(&self, interval_ms: u64) -> bool {
        let interval = Duration::from_millis(interval_ms.max(1));
        let now = Instant::now();
        let mut state = lock_unpoisoned(
            &self.attachment_orphan_prune,
            "attachment orphan prune state",
        );
        if state
            .last_attempt
            .is_some_and(|last| now.duration_since(last) < interval)
        {
            return false;
        }
        state.last_attempt = Some(now);
        true
    }

    pub async fn run_once(&self) -> Result<bool> {
        let now_millis = Utc::now().timestamp_millis();
        let promoted = self.runtime.promote_due_jobs(now_millis).await?;
        let requeued = self.runtime.requeue_expired_jobs(now_millis).await?;
        self.prune_job_history_if_due().await;
        self.prune_auth_credentials_if_due().await;
        self.prune_upload_temp_files_if_due().await;
        self.prune_attachment_orphans_if_due().await;
        for _ in 0..requeued {
            self.diagnostics
                .record_job_outcome(RecordedJobOutcome::ExpiredLeaseRequeued);
        }

        if let Some(lease) = self.runtime.claim_job().await? {
            self.diagnostics
                .record_job_outcome(RecordedJobOutcome::Leased);
            self.process_claimed_job(lease).await?;
            return Ok(true);
        }

        Ok(promoted > 0 || requeued > 0)
    }

    async fn process_claimed_job(&self, lease: ClaimedJobLease) -> Result<()> {
        let queue = lease.queue.clone();
        let token = lease.token.clone();
        let heartbeat = self.spawn_lease_heartbeat(queue.clone(), token.clone());
        let lost_signal = heartbeat.lease_lost_signal();
        let finalization = heartbeat.finalization();
        // Cancel the job future the moment the lease can no longer be proven:
        // letting it run past lease expiry means another worker can claim the
        // redelivered job and execute it concurrently.
        let result = tokio::select! {
            result = self.process_claimed_job_with_active_lease(lease, finalization) => Some(result),
            _ = lease_lost(lost_signal) => None,
        };
        heartbeat.shutdown().await;
        match result {
            None => {
                tracing::warn!(
                    target: "foundry.worker",
                    queue = %queue,
                    token = %token,
                    "Job execution cancelled after lease loss; the job will be redelivered"
                );
                Ok(())
            }
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => {
                error.log_recovery();
                let ClaimedJobInfraError { error, .. } = *error;
                Err(error)
            }
        }
    }

    async fn process_claimed_job_with_active_lease(
        &self,
        lease: ClaimedJobLease,
        finalization: LeaseFinalization,
    ) -> ClaimedJobProcessingResult<()> {
        self.diagnostics
            .record_job_outcome(RecordedJobOutcome::Started);

        let started_at = Utc::now().timestamp_millis();
        let middleware = self.app.resolve::<JobMiddlewareRegistry>().ok();
        let envelope: JobEnvelope = match serde_json::from_str(&lease.payload) {
            Ok(envelope) => envelope,
            Err(error) => {
                let trace_context = crate::logging::TraceContext::generated().with_parent(Some(
                    crate::logging::TraceParent::new("job", lease.token.clone()),
                ));
                let poison_envelope = JobEnvelope {
                    job: INVALID_JOB_ENVELOPE_ID,
                    queue: lease.queue.clone(),
                    attempts: 0,
                    scheduled_at: started_at,
                    payload: serde_json::Value::String(lease.payload.clone()),
                    trace: Some(trace_context.clone()),
                    batch_id: None,
                    chain_remaining: None,
                };
                let job_context = JobContext::new_with_trace(
                    self.app.clone(),
                    lease.queue.clone(),
                    1,
                    Some(trace_context),
                );
                self.dead_letter_claimed_job(DeadLetterClaimedJob {
                    lease: &lease,
                    envelope: poison_envelope,
                    error: format!("job envelope could not be deserialized: {error}"),
                    attempts: 1,
                    started_at,
                    middleware: middleware.as_deref(),
                    job_context: Some(&job_context),
                    finalization: &finalization,
                })
                .await
                .map_err(|error| {
                    ClaimedJobInfraError::new(
                        ClaimedJobPhase::PoisonDeadLetter,
                        ClaimedJobContext::new(
                            Some(INVALID_JOB_ENVELOPE_ID.to_string()),
                            lease.queue.to_string(),
                            lease.token.clone(),
                            Some(1),
                        ),
                        error,
                    )
                })?;
                return Ok(());
            }
        };
        let trace_context = envelope
            .trace
            .clone()
            .unwrap_or_else(crate::logging::TraceContext::generated)
            .with_parent(Some(crate::logging::TraceParent::new(
                "job",
                lease.token.clone(),
            )));
        let envelope = JobEnvelope {
            trace: Some(trace_context.clone()),
            ..envelope
        };
        let context = ClaimedJobContext::from_envelope(&lease, &envelope, envelope.attempts + 1);
        let Some(registration) = self.runtime.registry.jobs.get(&envelope.job) else {
            let attempts = envelope.attempts + 1;
            let context = ClaimedJobContext::from_envelope(&lease, &envelope, attempts);
            let job_context = JobContext::new_with_trace(
                self.app.clone(),
                envelope.queue.clone(),
                attempts,
                Some(trace_context.clone()),
            );
            let error = format!("job `{}` is not registered", envelope.job);
            self.dead_letter_claimed_job(DeadLetterClaimedJob {
                lease: &lease,
                envelope,
                error,
                attempts,
                started_at,
                middleware: middleware.as_deref(),
                job_context: Some(&job_context),
                finalization: &finalization,
            })
            .await
            .map_err(|error| {
                ClaimedJobInfraError::new(ClaimedJobPhase::UnknownJobDeadLetter, context, error)
            })?;
            return Ok(());
        };

        // Rate limit check: requeue without incrementing attempts if over limit
        if let Some((max_per_window, window)) = registration.handler.check_rate_limit(&envelope) {
            let window_secs = window.as_secs().max(1);
            let window_bucket = Utc::now().timestamp() / window_secs as i64;
            let rate_key = format!("jobs:rate:{}:{}", envelope.job, window_bucket);
            let current_count = claimed_job_result(
                ClaimedJobPhase::RateLimitCheck,
                &context,
                self.runtime
                    .backend
                    .incr_with_ttl(&rate_key, window_secs)
                    .await,
            )?;
            if current_count > max_per_window as u64 {
                // Over the rate limit — requeue with the same attempt count
                // and a short delay so it retries soon without counting as a failure.
                let delay_ms = 1000; // 1 second delay before retry
                let requeue_at = Utc::now().timestamp_millis() + delay_ms;
                let requeue_envelope = JobEnvelope {
                    scheduled_at: requeue_at,
                    ..envelope
                };
                let context = ClaimedJobContext::from_envelope(
                    &lease,
                    &requeue_envelope,
                    requeue_envelope.attempts + 1,
                );
                let payload = claimed_job_result(
                    ClaimedJobPhase::RateLimitPayload,
                    &context,
                    serde_json::to_string(&requeue_envelope).map_err(Error::other),
                )?;
                let requeue_token = next_delivery_token();
                if !finalization.begin() {
                    return Ok(());
                }
                if !claimed_job_result(
                    ClaimedJobPhase::RateLimitRequeue,
                    &context,
                    self.runtime
                        .retry_job(
                            &lease.queue,
                            &lease.token,
                            &requeue_token,
                            &payload,
                            requeue_at,
                        )
                        .await,
                )? {
                    tracing::warn!(
                        target: "foundry.worker",
                        queue = %lease.queue,
                        token = %lease.token,
                        "Lost job lease before rate-limit requeue"
                    );
                    return Ok(());
                }
                tracing::debug!(
                    target: "foundry.worker",
                    job = %requeue_envelope.job,
                    count = current_count,
                    limit = max_per_window,
                    "Job rate-limited, requeued with delay"
                );
                return Ok(());
            }
        }

        let job_context = JobContext::new_with_trace(
            self.app.clone(),
            envelope.queue.clone(),
            envelope.attempts + 1,
            Some(trace_context.clone()),
        );

        // Before hooks
        if let Some(ref mw) = middleware {
            crate::logging::scope_current_trace(
                trace_context.clone(),
                mw.run_before(&envelope.job, &job_context),
            )
            .await;
        }

        let default_timeout = Duration::from_secs(self.runtime.config.timeout_seconds.max(1));
        let execution = crate::logging::scope_current_trace(
            trace_context.clone(),
            crate::logging::scope_current_execution(
                crate::logging::ExecutionContext::Job {
                    class: envelope.job.to_string(),
                    id: lease.token.clone(),
                },
                registration.handler.execute(
                    &self.app,
                    &envelope,
                    self.runtime.config.max_retries,
                    default_timeout,
                ),
            ),
        )
        .await;
        let execution = claimed_job_result(ClaimedJobPhase::ExecuteJob, &context, execution)?;

        match execution {
            JobExecutionOutcome::Success => {
                if let Some(ref mw) = middleware {
                    crate::logging::scope_current_trace(
                        trace_context.clone(),
                        mw.run_after(&envelope.job, &job_context),
                    )
                    .await;
                }
                let chain_effect = claimed_job_result(
                    ClaimedJobPhase::BuildChainContinuation,
                    &context,
                    Self::build_chain_continuation(
                        envelope.chain_remaining.clone(),
                        Some(trace_context.clone()),
                    ),
                )?;
                if !finalization.begin() {
                    return Ok(());
                }
                let success = claimed_job_result(
                    ClaimedJobPhase::SuccessFinalization,
                    &context,
                    self.runtime
                        .complete_successful_job(
                            &lease.queue,
                            &lease.token,
                            SuccessfulJobEffects {
                                chain: chain_effect,
                                batch_id: envelope.batch_id.clone(),
                                batch_callback_token: envelope
                                    .batch_id
                                    .as_ref()
                                    .map(|_| next_delivery_token()),
                            },
                        )
                        .await,
                )?;
                if !success.lease_released {
                    tracing::warn!(
                        target: "foundry.worker",
                        queue = %lease.queue,
                        token = %lease.token,
                        "Lost job lease before success finalization"
                    );
                    return Ok(());
                }
                tracing::info!(
                    target: "foundry.worker",
                    job = %envelope.job,
                    queue = %envelope.queue,
                    attempt = envelope.attempts + 1,
                    "Job succeeded"
                );
                self.diagnostics
                    .record_job_outcome(RecordedJobOutcome::Succeeded);

                let duration_ms = Utc::now().timestamp_millis() - started_at;
                self.record_job_history(JobHistoryEntry {
                    job_id: &envelope.job,
                    queue: &envelope.queue,
                    status: JobHistoryStatus::Succeeded,
                    attempt: envelope.attempts + 1,
                    error: None,
                    started_at,
                    duration_ms,
                    payload: job_history_trace_payload(&envelope),
                })
                .await;

                if let Some(ref batch_id) = envelope.batch_id {
                    if let Some(batch) = success.batch {
                        tracing::debug!(
                            target: "foundry.worker",
                            batch_id = %batch_id,
                            completed = batch.completed,
                            total = batch.total,
                            "Batch progress"
                        );
                        if batch.completed >= batch.total {
                            if batch.on_complete_enqueued {
                                self.diagnostics
                                    .record_job_outcome(RecordedJobOutcome::Enqueued);
                                tracing::info!(
                                    target: "foundry.worker",
                                    batch_id = %batch_id,
                                    "Batch completed, on_complete job dispatched"
                                );
                            } else {
                                tracing::info!(
                                    target: "foundry.worker",
                                    batch_id = %batch_id,
                                    "Batch completed"
                                );
                            }
                        }
                    } else {
                        tracing::warn!(
                            target: "foundry.worker",
                            batch_id = %batch_id,
                            "Batch metadata missing during success finalization"
                        );
                    }
                }

                if success.chain_enqueued {
                    self.diagnostics
                        .record_job_outcome(RecordedJobOutcome::Enqueued);
                    tracing::info!(
                        target: "foundry.worker",
                        job = %envelope.job,
                        "Chain continuation dispatched"
                    );
                }

                Ok(())
            }
            JobExecutionOutcome::Retry {
                run_at_millis,
                attempts,
                error,
            } => {
                if let Some(ref mw) = middleware {
                    crate::logging::scope_current_trace(
                        trace_context.clone(),
                        mw.run_failed(&envelope.job, &job_context, &error),
                    )
                    .await;
                }
                let retry_job_id = envelope.job.clone();
                let retry_queue = envelope.queue.clone();
                let retry_envelope = JobEnvelope {
                    attempts,
                    scheduled_at: run_at_millis,
                    ..envelope
                };
                let retry_context =
                    ClaimedJobContext::from_envelope(&lease, &retry_envelope, attempts);
                let payload = claimed_job_result(
                    ClaimedJobPhase::RetryPayload,
                    &retry_context,
                    serde_json::to_string(&retry_envelope).map_err(Error::other),
                )?;
                let retry_token = next_delivery_token();
                if !finalization.begin() {
                    return Ok(());
                }
                if !claimed_job_result(
                    ClaimedJobPhase::RetrySchedule,
                    &retry_context,
                    self.runtime
                        .retry_job(
                            &lease.queue,
                            &lease.token,
                            &retry_token,
                            &payload,
                            run_at_millis,
                        )
                        .await,
                )? {
                    tracing::warn!(
                        target: "foundry.worker",
                        queue = %lease.queue,
                        token = %lease.token,
                        "Lost job lease before retry scheduling"
                    );
                    return Ok(());
                }
                self.diagnostics
                    .record_job_outcome(RecordedJobOutcome::Retried);

                let duration_ms = Utc::now().timestamp_millis() - started_at;
                self.record_job_history(JobHistoryEntry {
                    job_id: &retry_job_id,
                    queue: &retry_queue,
                    status: JobHistoryStatus::Retried,
                    attempt: attempts,
                    error: Some(&error),
                    started_at,
                    duration_ms,
                    payload: job_history_trace_payload(&retry_envelope),
                })
                .await;

                Ok(())
            }
            JobExecutionOutcome::DeadLetter { error, attempts } => {
                if let Some(ref mw) = middleware {
                    crate::logging::scope_current_trace(
                        trace_context.clone(),
                        mw.run_failed(&envelope.job, &job_context, &error),
                    )
                    .await;
                }
                let job_name = envelope.job.clone();
                let queue_name = envelope.queue.clone();
                let payload_json = envelope.payload.clone();
                let dead_letter = FailedJobEnvelope {
                    failed_at: Utc::now().timestamp_millis(),
                    error: error.clone(),
                    envelope: JobEnvelope {
                        attempts,
                        ..envelope
                    },
                };
                let dead_letter_context = ClaimedJobContext::new(
                    Some(job_name.to_string()),
                    queue_name.to_string(),
                    lease.token.clone(),
                    Some(attempts),
                );
                let payload = claimed_job_result(
                    ClaimedJobPhase::DeadLetterPayload,
                    &dead_letter_context,
                    serde_json::to_string(&dead_letter).map_err(Error::other),
                )?;
                if !finalization.begin() {
                    return Ok(());
                }
                if !claimed_job_result(
                    ClaimedJobPhase::DeadLetterTransition,
                    &dead_letter_context,
                    self.runtime
                        .dead_letter_job(&lease.queue, &lease.token, &payload)
                        .await,
                )? {
                    tracing::warn!(
                        target: "foundry.worker",
                        queue = %lease.queue,
                        token = %lease.token,
                        "Lost job lease before dead-letter transition"
                    );
                    return Ok(());
                }
                tracing::error!(
                    target: "foundry.worker",
                    job = %job_name,
                    queue = %queue_name,
                    attempts = attempts,
                    error = %error,
                    "Job dead-lettered"
                );
                self.diagnostics
                    .record_job_outcome(RecordedJobOutcome::DeadLettered);

                let duration_ms = Utc::now().timestamp_millis() - started_at;
                self.record_job_history(JobHistoryEntry {
                    job_id: &job_name,
                    queue: &queue_name,
                    status: JobHistoryStatus::DeadLettered,
                    attempt: attempts,
                    error: Some(&error),
                    started_at,
                    duration_ms,
                    payload: job_history_trace_payload(&dead_letter.envelope),
                })
                .await;

                if let Some(ref mw) = middleware {
                    crate::logging::scope_current_trace(
                        trace_context,
                        mw.run_dead_lettered(&JobDeadLetterContext {
                            class: job_name.to_string(),
                            id: lease.token.clone(),
                            attempts,
                            last_error: error.clone(),
                            payload: payload_json,
                            app: self.app.clone(),
                        }),
                    )
                    .await;
                }

                Ok(())
            }
        }
    }

    async fn dead_letter_claimed_job(&self, job: DeadLetterClaimedJob<'_>) -> Result<()> {
        let DeadLetterClaimedJob {
            lease,
            envelope,
            error,
            attempts,
            started_at,
            middleware,
            job_context,
            finalization,
        } = job;

        if let (Some(middleware), Some(job_context)) = (middleware, job_context) {
            let failed = middleware.run_failed(&envelope.job, job_context, &error);
            if let Some(trace_context) = envelope.trace.clone() {
                crate::logging::scope_current_trace(trace_context, failed).await;
            } else {
                failed.await;
            }
        }

        let job_name = envelope.job.clone();
        let queue_name = envelope.queue.clone();
        let payload_json = envelope.payload.clone();
        let dead_letter = FailedJobEnvelope {
            failed_at: Utc::now().timestamp_millis(),
            error: error.clone(),
            envelope: JobEnvelope {
                attempts,
                ..envelope
            },
        };
        let payload = serde_json::to_string(&dead_letter).map_err(Error::other)?;
        if !finalization.begin() {
            return Ok(());
        }
        if !self
            .runtime
            .dead_letter_job(&lease.queue, &lease.token, &payload)
            .await?
        {
            tracing::warn!(
                target: "foundry.worker",
                queue = %lease.queue,
                token = %lease.token,
                "Lost job lease before poison dead-letter transition"
            );
            return Ok(());
        }

        tracing::error!(
            target: "foundry.worker",
            job = %job_name,
            queue = %queue_name,
            attempts = attempts,
            error = %error,
            "Job dead-lettered"
        );
        self.diagnostics
            .record_job_outcome(RecordedJobOutcome::DeadLettered);

        let duration_ms = Utc::now().timestamp_millis() - started_at;
        self.record_job_history(JobHistoryEntry {
            job_id: &job_name,
            queue: &queue_name,
            status: JobHistoryStatus::DeadLettered,
            attempt: attempts,
            error: Some(&error),
            started_at,
            duration_ms,
            payload: job_history_trace_payload(&dead_letter.envelope),
        })
        .await;

        if let Some(middleware) = middleware {
            let dead_letter_context = JobDeadLetterContext {
                class: job_name.to_string(),
                id: lease.token.clone(),
                attempts,
                last_error: error,
                payload: payload_json,
                app: self.app.clone(),
            };
            let dead_lettered = middleware.run_dead_lettered(&dead_letter_context);
            if let Some(trace_context) = dead_letter.envelope.trace {
                crate::logging::scope_current_trace(trace_context, dead_lettered).await;
            } else {
                dead_lettered.await;
            }
        }

        Ok(())
    }

    fn spawn_lease_heartbeat(&self, queue: QueueId, token: String) -> LeaseHeartbeat {
        LeaseHeartbeat::spawn(self.runtime.clone(), queue, token)
    }

    fn build_chain_continuation(
        remaining: Option<Vec<ChainedJob>>,
        trace: Option<crate::logging::TraceContext>,
    ) -> Result<Option<JobToEnqueue>> {
        let Some(mut remaining) = remaining else {
            return Ok(None);
        };
        if remaining.is_empty() {
            return Ok(None);
        }

        let next = remaining.remove(0);
        let chain_remaining = if remaining.is_empty() {
            None
        } else {
            Some(remaining)
        };

        let now = Utc::now().timestamp_millis();
        let envelope = JobEnvelope {
            job: next.job.clone(),
            queue: next.queue.clone(),
            attempts: 0,
            scheduled_at: now,
            payload: next.payload,
            trace,
            batch_id: None,
            chain_remaining,
        };
        let serialized = serde_json::to_string(&envelope).map_err(Error::other)?;
        let token = next_delivery_token();
        Ok(Some(JobToEnqueue {
            queue: next.queue,
            token,
            payload: serialized,
        }))
    }
}

struct ActiveWorkerJobs {
    tasks: Mutex<Vec<WorkerJobTask>>,
    shutdown_timeout: Duration,
}

impl ActiveWorkerJobs {
    fn new(shutdown_timeout: Duration) -> Self {
        Self {
            tasks: Mutex::new(Vec::new()),
            shutdown_timeout,
        }
    }

    fn track(&self, handle: JoinHandle<()>) {
        lock_unpoisoned(&self.tasks, "worker active jobs").push(WorkerJobTask::new(handle));
    }

    async fn prune_finished(&self) {
        let mut finished = Vec::new();
        {
            let mut tasks = lock_unpoisoned(&self.tasks, "worker active jobs");
            let mut index = 0;
            while index < tasks.len() {
                if tasks[index].is_finished() {
                    finished.push(tasks.swap_remove(index));
                } else {
                    index += 1;
                }
            }
        }

        for task in finished {
            task.wait_finished().await;
        }
    }

    async fn drain(&self) {
        let tasks = {
            let mut tasks = lock_unpoisoned(&self.tasks, "worker active jobs");
            std::mem::take(&mut *tasks)
        };

        drain_tasks(
            tasks,
            self.shutdown_timeout,
            ShutdownDrainMessages {
                target: ShutdownDrainTarget::Worker,
                timeout_disabled: "worker shutdown timeout disabled; aborting active jobs",
                waiting: "waiting for active jobs during worker shutdown",
                drained: "active worker jobs drained",
                timeout_elapsed: "worker shutdown timeout elapsed; aborting active jobs",
            },
        )
        .await;
    }
}

struct WorkerJobTask {
    handle: Option<JoinHandle<()>>,
}

impl WorkerJobTask {
    fn new(handle: JoinHandle<()>) -> Self {
        Self {
            handle: Some(handle),
        }
    }
}

#[async_trait]
impl ShutdownDrainTask for WorkerJobTask {
    fn is_finished(&mut self) -> bool {
        self.handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true)
    }

    async fn wait_finished(mut self) {
        if let Some(handle) = self.handle.take() {
            if let Err(error) = handle.await {
                tracing::warn!(
                    target: "foundry.worker",
                    error = %error,
                    "Worker job task finished with join error"
                );
            }
        }
    }

    fn abort(&self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }

    async fn wait_after_abort(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for WorkerJobTask {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            if !handle.is_finished() {
                handle.abort();
            }
        }
    }
}

const LEASE_ACTIVE: u8 = 0;
const LEASE_FINALIZING: u8 = 1;
const LEASE_LOST: u8 = 2;

#[derive(Clone, Default)]
struct LeaseFinalization {
    state: Arc<AtomicU8>,
}

impl LeaseFinalization {
    /// Atomically wins the right to perform the lease-releasing transition.
    /// If lease loss won first, the caller must leave the job for redelivery.
    fn begin(&self) -> bool {
        self.try_transition_from_active(LEASE_FINALIZING)
    }

    fn is_finalizing(&self) -> bool {
        self.state.load(Ordering::Acquire) == LEASE_FINALIZING
    }

    /// Atomically marks an active lease as lost. Finalization and loss are
    /// mutually exclusive so a normal backend release cannot cancel its own
    /// post-transition work.
    fn mark_lost(&self) -> bool {
        self.try_transition_from_active(LEASE_LOST)
    }

    fn try_transition_from_active(&self, next: u8) -> bool {
        self.state
            .compare_exchange(LEASE_ACTIVE, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

struct LeaseHeartbeat {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    lease_lost: tokio::sync::watch::Receiver<bool>,
    finalization: LeaseFinalization,
}

impl LeaseHeartbeat {
    fn spawn(runtime: Arc<JobRuntime>, queue: QueueId, token: String) -> Self {
        let heartbeat_every = runtime.lease_heartbeat_interval();
        let lease_ttl = runtime.lease_ttl();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let (lost_tx, lost_rx) = tokio::sync::watch::channel(false);
        let finalization = LeaseFinalization::default();
        let heartbeat_finalization = finalization.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(heartbeat_every);
            let mut last_renewed = tokio::time::Instant::now();
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    _ = interval.tick() => {
                        let renewal = runtime.renew_job_lease(&queue, &token).await;
                        match renewal {
                            Ok(true) => {
                                last_renewed = tokio::time::Instant::now();
                            }
                            Ok(false) if heartbeat_finalization.is_finalizing() => {
                                // The final backend transition intentionally
                                // releases the lease. Do not race that normal
                                // completion against the worker's cancellation
                                // branch while post-transition work finishes.
                                break;
                            }
                            Ok(false) => {
                                if !heartbeat_finalization.mark_lost() {
                                    break;
                                }
                                // The lease is definitively gone (expired or
                                // claimed elsewhere). Signal the executor so
                                // the job stops instead of running
                                // concurrently with its redelivery.
                                tracing::warn!(
                                    target: "foundry.worker",
                                    queue = %queue,
                                    token = %token,
                                    "Job lease no longer held; cancelling execution"
                                );
                                let _ = lost_tx.send(true);
                                break;
                            }
                            Err(error) => {
                                // Transient backend error: keep retrying while
                                // the last successful renewal still covers the
                                // lease TTL; past that we can no longer prove
                                // ownership and must stop the job.
                                if last_renewed.elapsed() >= lease_ttl {
                                    if !heartbeat_finalization.mark_lost() {
                                        break;
                                    }
                                    tracing::warn!(
                                        target: "foundry.worker",
                                        queue = %queue,
                                        token = %token,
                                        error = %error,
                                        "Job lease presumed expired after repeated renewal failures; cancelling execution"
                                    );
                                    let _ = lost_tx.send(true);
                                    break;
                                }
                                tracing::warn!(
                                    target: "foundry.worker",
                                    queue = %queue,
                                    token = %token,
                                    error = %error,
                                    "Failed to renew lease; will retry"
                                );
                            }
                        }
                    }
                }
            }
        });

        Self {
            shutdown: Some(shutdown_tx),
            handle: Some(handle),
            lease_lost: lost_rx,
            finalization,
        }
    }

    fn lease_lost_signal(&self) -> tokio::sync::watch::Receiver<bool> {
        self.lease_lost.clone()
    }

    fn finalization(&self) -> LeaseFinalization {
        self.finalization.clone()
    }

    async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

/// Resolves once the heartbeat reports the lease as lost. Never resolves if
/// the heartbeat ends without signalling loss (e.g. normal shutdown).
async fn lease_lost(mut signal: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *signal.borrow() {
            return;
        }
        if signal.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

/// Terminal status for a job recorded in the `job_history` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, foundry_macros::AppEnum)]
pub enum JobHistoryStatus {
    Succeeded,
    Retried,
    DeadLettered,
}

impl JobHistoryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Retried => "retried",
            Self::DeadLettered => "dead_lettered",
        }
    }
}

impl std::fmt::Display for JobHistoryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

struct JobHistoryEntry<'a> {
    job_id: &'a JobId,
    queue: &'a QueueId,
    status: JobHistoryStatus,
    attempt: u32,
    error: Option<&'a str>,
    started_at: i64,
    duration_ms: i64,
    payload: Option<serde_json::Value>,
}

fn job_history_trace_payload(envelope: &JobEnvelope) -> Option<serde_json::Value> {
    envelope
        .trace
        .as_ref()
        .map(|trace| serde_json::json!({ "trace": trace }))
}

impl Worker {
    async fn record_job_history(&self, entry: JobHistoryEntry<'_>) {
        let JobHistoryEntry {
            job_id,
            queue,
            status,
            attempt,
            error,
            started_at,
            duration_ms,
            payload,
        } = entry;
        if !self.runtime.config.track_history {
            return;
        }
        let Ok(db) = self.app.database() else {
            return;
        };
        if !db.is_configured() {
            return;
        }

        if let Err(error) = Query::insert_into("job_history")
            .values([
                ("job_id", DbValue::Text(job_id.to_string())),
                ("queue", DbValue::Text(queue.to_string())),
                ("status", DbValue::Text(status.to_string())),
                (
                    "payload",
                    payload
                        .map(DbValue::Json)
                        .unwrap_or(DbValue::Null(DbType::Json)),
                ),
                ("attempt", DbValue::Int32(attempt as i32)),
                (
                    "error",
                    if let Some(e) = error {
                        DbValue::Text(e.to_string())
                    } else {
                        DbValue::Null(DbType::Text)
                    },
                ),
                ("duration_ms", DbValue::Int64(duration_ms)),
            ])
            .value_expr(
                "started_at",
                Sql::to_timestamp_millis(DbValue::Int64(started_at)),
            )
            .value_expr("completed_at", Sql::now())
            .execute(db.as_ref())
            .await
        {
            tracing::warn!(
                target: "foundry.worker",
                job = %job_id,
                error = %error,
                "failed to record job history"
            );
        }
    }
}

pub fn spawn_worker(app: AppContext) -> Result<tokio::task::JoinHandle<()>> {
    let worker_app = app.clone();
    if let Some(handle) =
        app.spawn_managed_background_task("foundry.worker", move |shutdown_rx| {
            let worker = Worker::from_app(worker_app)?;
            Ok(async move {
                let result = worker
                    .run_until(async move {
                        let _ = shutdown_rx.await;
                    })
                    .await;
                if let Err(error) = result {
                    tracing::error!("foundry worker exited with error: {error}");
                }
            })
        })?
    {
        return Ok(handle);
    }

    let kernel = crate::kernel::worker::WorkerKernel::new(app)?;
    Ok(tokio::spawn(async move {
        if let Err(error) = kernel.run().await {
            tracing::error!("foundry worker exited with error: {error}");
        }
    }))
}

pub(crate) type JobRegistryHandle = Arc<Mutex<JobRegistryBuilder>>;

#[derive(Default)]
pub(crate) struct JobRegistryBuilder {
    jobs: HashMap<JobId, JobRegistrationBuilder>,
}

impl JobRegistryBuilder {
    pub(crate) fn shared() -> JobRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register<J>(&mut self) -> Result<()>
    where
        J: Job,
    {
        if self.jobs.contains_key(&J::ID) {
            return Err(Error::message(format!(
                "job `{}` already registered",
                J::ID
            )));
        }

        self.jobs.insert(
            J::ID,
            JobRegistrationBuilder {
                queue: J::QUEUE.clone(),
                handler: Arc::new(JobHandlerAdapter::<J> {
                    marker: PhantomData,
                }),
            },
        );
        Ok(())
    }

    pub(crate) fn freeze_shared(
        handle: JobRegistryHandle,
        config: &JobsConfig,
    ) -> JobRegistrySnapshot {
        let mut builder = lock_unpoisoned(&handle, "job registry");
        let jobs = std::mem::take(&mut builder.jobs)
            .into_iter()
            .map(|(name, registration)| {
                let queue = registration.queue.unwrap_or_else(|| config.queue.clone());
                (
                    name,
                    JobRegistration {
                        queue,
                        handler: registration.handler,
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let mut queues = HashSet::new();
        queues.insert(config.queue.clone());
        for registration in jobs.values() {
            queues.insert(registration.queue.clone());
        }
        for queue in config.queue_priorities.keys() {
            queues.insert(QueueId::owned(queue.clone()));
        }

        let mut queues: Vec<QueueId> = queues.into_iter().collect();
        // Sort by configured priority (lower = higher priority, default = 5)
        queues.sort_by_key(|q| {
            config
                .queue_priorities
                .get(q.as_ref())
                .copied()
                .unwrap_or(5)
        });

        JobRegistrySnapshot { jobs, queues }
    }
}

pub(crate) struct JobRuntime {
    backend: RuntimeBackend,
    config: JobsConfig,
    registry: JobRegistrySnapshot,
}

impl JobRuntime {
    pub(crate) fn new(
        backend: RuntimeBackend,
        config: JobsConfig,
        registry: JobRegistrySnapshot,
    ) -> Self {
        Self {
            backend,
            config,
            registry,
        }
    }

    fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.config.poll_interval_ms.max(1))
    }

    fn lease_ttl(&self) -> Duration {
        Duration::from_millis(self.config.lease_ttl_ms.max(1))
    }

    fn lease_heartbeat_interval(&self) -> Duration {
        let millis = (self.config.lease_ttl_ms / 3).max(1);
        Duration::from_millis(millis)
    }

    fn shutdown_timeout(&self) -> Duration {
        Duration::from_millis(self.config.shutdown_timeout_ms)
    }

    async fn promote_due_jobs(&self, now_millis: i64) -> Result<usize> {
        self.backend
            .promote_due_jobs(
                &self.registry.queues,
                now_millis,
                self.config.requeue_batch_size,
            )
            .await
    }

    async fn requeue_expired_jobs(&self, now_millis: i64) -> Result<usize> {
        self.backend
            .requeue_expired_jobs(
                &self.registry.queues,
                now_millis,
                self.config.requeue_batch_size,
            )
            .await
    }

    async fn claim_job(&self) -> Result<Option<ClaimedJobLease>> {
        self.backend
            .claim_job(&self.registry.queues, self.lease_ttl())
            .await
    }

    async fn renew_job_lease(&self, queue: &QueueId, token: &str) -> Result<bool> {
        self.backend
            .renew_job_lease(queue, token, self.lease_ttl())
            .await
    }

    async fn retry_job(
        &self,
        queue: &QueueId,
        token: &str,
        new_token: &str,
        payload: &str,
        run_at_millis: i64,
    ) -> Result<bool> {
        self.backend
            .retry_job(queue, token, new_token, payload, run_at_millis)
            .await
    }

    async fn dead_letter_job(&self, queue: &QueueId, token: &str, payload: &str) -> Result<bool> {
        self.backend.dead_letter_job(queue, token, payload).await
    }

    async fn complete_successful_job(
        &self,
        queue: &QueueId,
        token: &str,
        effects: SuccessfulJobEffects,
    ) -> Result<backend::SuccessfulJobCompletion> {
        self.backend
            .complete_successful_job(queue, token, &self.config.queue, effects)
            .await
    }
}

type ClaimedJobProcessingResult<T> = std::result::Result<T, Box<ClaimedJobInfraError>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimedJobPhase {
    PoisonDeadLetter,
    UnknownJobDeadLetter,
    RateLimitCheck,
    RateLimitPayload,
    RateLimitRequeue,
    ExecuteJob,
    BuildChainContinuation,
    SuccessFinalization,
    RetryPayload,
    RetrySchedule,
    DeadLetterPayload,
    DeadLetterTransition,
}

impl ClaimedJobPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::PoisonDeadLetter => "poison_dead_letter",
            Self::UnknownJobDeadLetter => "unknown_job_dead_letter",
            Self::RateLimitCheck => "rate_limit_check",
            Self::RateLimitPayload => "rate_limit_payload",
            Self::RateLimitRequeue => "rate_limit_requeue",
            Self::ExecuteJob => "execute_job",
            Self::BuildChainContinuation => "build_chain_continuation",
            Self::SuccessFinalization => "success_finalization",
            Self::RetryPayload => "retry_payload",
            Self::RetrySchedule => "retry_schedule",
            Self::DeadLetterPayload => "dead_letter_payload",
            Self::DeadLetterTransition => "dead_letter_transition",
        }
    }
}

impl std::fmt::Display for ClaimedJobPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClaimedJobContext {
    job: Option<String>,
    queue: String,
    token: String,
    attempt: Option<u32>,
}

impl ClaimedJobContext {
    fn new(job: Option<String>, queue: String, token: String, attempt: Option<u32>) -> Self {
        Self {
            job,
            queue,
            token,
            attempt,
        }
    }

    fn from_envelope(lease: &ClaimedJobLease, envelope: &JobEnvelope, attempt: u32) -> Self {
        Self::new(
            Some(envelope.job.to_string()),
            lease.queue.to_string(),
            lease.token.clone(),
            Some(attempt),
        )
    }
}

#[derive(Debug)]
struct ClaimedJobInfraError {
    phase: ClaimedJobPhase,
    context: ClaimedJobContext,
    error: Error,
}

impl ClaimedJobInfraError {
    fn new(phase: ClaimedJobPhase, context: ClaimedJobContext, error: Error) -> Box<Self> {
        Box::new(Self {
            phase,
            context,
            error,
        })
    }

    fn log_recovery(&self) {
        tracing::error!(
            target: "foundry.worker",
            phase = %self.phase,
            job = self.context.job.as_deref().unwrap_or("unknown"),
            queue = %self.context.queue,
            token = %self.context.token,
            attempt = ?self.context.attempt,
            error = %self.error,
            "Claimed job processing errored; lease left for expiry recovery"
        );
    }
}

fn claimed_job_result<T>(
    phase: ClaimedJobPhase,
    context: &ClaimedJobContext,
    result: Result<T>,
) -> ClaimedJobProcessingResult<T> {
    result.map_err(|error| ClaimedJobInfraError::new(phase, context.clone(), error))
}

pub(crate) struct JobRegistrySnapshot {
    jobs: HashMap<JobId, JobRegistration>,
    queues: Vec<QueueId>,
}

impl JobRegistrySnapshot {
    pub(crate) fn include_queue(&mut self, queue: QueueId, config: &JobsConfig) {
        if !self.queues.contains(&queue) {
            self.queues.push(queue);
        }
        self.queues.sort_by_key(|queue| {
            config
                .queue_priorities
                .get(queue.as_ref())
                .copied()
                .unwrap_or(5)
        });
    }
}

struct JobRegistrationBuilder {
    queue: Option<QueueId>,
    handler: Arc<dyn DynJobHandler>,
}

struct JobRegistration {
    queue: QueueId,
    handler: Arc<dyn DynJobHandler>,
}

#[derive(Clone, Serialize, Deserialize)]
struct JobEnvelope {
    job: JobId,
    queue: QueueId,
    attempts: u32,
    scheduled_at: i64,
    payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trace: Option<crate::logging::TraceContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chain_remaining: Option<Vec<ChainedJob>>,
}

/// A serialized job entry used in chain sequences.
#[derive(Clone, Serialize, Deserialize)]
struct ChainedJob {
    job: JobId,
    queue: QueueId,
    payload: serde_json::Value,
}

#[derive(Clone, Serialize, Deserialize)]
struct FailedJobEnvelope {
    failed_at: i64,
    error: String,
    envelope: JobEnvelope,
}

struct DeadLetterClaimedJob<'a> {
    lease: &'a ClaimedJobLease,
    envelope: JobEnvelope,
    error: String,
    attempts: u32,
    started_at: i64,
    middleware: Option<&'a JobMiddlewareRegistry>,
    job_context: Option<&'a JobContext>,
    finalization: &'a LeaseFinalization,
}

enum JobExecutionOutcome {
    Success,
    Retry {
        run_at_millis: i64,
        attempts: u32,
        error: String,
    },
    DeadLetter {
        error: String,
        attempts: u32,
    },
}

#[async_trait]
trait DynJobHandler: Send + Sync {
    async fn execute(
        &self,
        app: &AppContext,
        envelope: &JobEnvelope,
        default_max_retries: u32,
        default_timeout: Duration,
    ) -> Result<JobExecutionOutcome>;

    /// Check whether the job type has a rate limit, and if so, return it.
    /// Deserializes the payload to read the concrete job's `rate_limit()`.
    fn check_rate_limit(&self, envelope: &JobEnvelope) -> Option<(u32, Duration)>;
}

struct JobHandlerAdapter<J> {
    marker: PhantomData<J>,
}

#[async_trait]
impl<J> DynJobHandler for JobHandlerAdapter<J>
where
    J: Job,
{
    async fn execute(
        &self,
        app: &AppContext,
        envelope: &JobEnvelope,
        default_max_retries: u32,
        default_timeout: Duration,
    ) -> Result<JobExecutionOutcome> {
        let job: J = match serde_json::from_value(envelope.payload.clone()) {
            Ok(job) => job,
            Err(error) => {
                return Ok(JobExecutionOutcome::DeadLetter {
                    error: error.to_string(),
                    attempts: envelope.attempts + 1,
                });
            }
        };

        let timeout_duration = job.timeout().unwrap_or(default_timeout);
        let context = JobContext::new(app.clone(), envelope.queue.clone(), envelope.attempts + 1);
        let error_msg = match catch_sync_panic(|| job.handle(context)) {
            Ok(job_future) => {
                let result =
                    tokio::time::timeout(timeout_duration, catch_future_panic(job_future)).await;

                match result {
                    Ok(Ok(Ok(()))) => return Ok(JobExecutionOutcome::Success),
                    Ok(Ok(Err(error))) => error.to_string(),
                    Ok(Err(panic)) => format!("job panicked: {}", panic_payload_message(panic)),
                    Err(_elapsed) => format!("job timed out after {}s", timeout_duration.as_secs()),
                }
            }
            Err(panic) => format!("job panicked: {}", panic_payload_message(panic)),
        };

        // Failure — decide retry vs dead-letter
        let attempts = envelope.attempts + 1;
        let max_retries = job.max_retries().unwrap_or(default_max_retries);
        if attempts >= max_retries {
            return Ok(JobExecutionOutcome::DeadLetter {
                error: error_msg,
                attempts,
            });
        } else {
            let run_at_millis =
                Utc::now().timestamp_millis() + job.backoff(attempts).as_millis() as i64;
            return Ok(JobExecutionOutcome::Retry {
                run_at_millis,
                attempts,
                error: error_msg,
            });
        }
    }

    fn check_rate_limit(&self, envelope: &JobEnvelope) -> Option<(u32, Duration)> {
        let job: J = serde_json::from_value(envelope.payload.clone()).ok()?;
        job.rate_limit()
    }
}

fn next_delivery_token() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    format!(
        "{:x}-{:x}",
        Utc::now().timestamp_micros(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use serde::{Deserialize, Serialize};
    use tokio::sync::Notify;

    use super::{
        checked_dispatch_time_after, ChainedJob, Job, JobContext, JobDeadLetterContext,
        JobDispatcher, JobEnvelope, JobMiddleware, JobMiddlewareRegistryBuilder,
        JobRegistryBuilder, JobRuntime, SuccessfulJobEffects, Worker,
    };
    use crate::config::JobsConfig;
    use crate::foundation::{AppContext, Container, Error};
    use crate::logging::{ReadinessRegistryBuilder, RuntimeBackendKind, RuntimeDiagnostics};
    use crate::support::runtime::RuntimeBackend;
    use crate::support::{DateTime, JobId, QueueId};
    use crate::validation::RuleRegistry;

    #[test]
    fn claimed_job_infra_error_preserves_phase_and_context() {
        let context = super::ClaimedJobContext::new(
            Some("email.job".to_string()),
            "critical".to_string(),
            "token-1".to_string(),
            Some(3),
        );

        let error = super::claimed_job_result::<()>(
            super::ClaimedJobPhase::RetrySchedule,
            &context,
            Err(Error::message("redis unavailable")),
        )
        .unwrap_err();

        assert_eq!(error.phase, super::ClaimedJobPhase::RetrySchedule);
        assert_eq!(error.phase.to_string(), "retry_schedule");
        assert_eq!(error.context, context);
        assert_eq!(error.error.to_string(), "redis unavailable");
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct FailingJob;

    #[async_trait]
    impl Job for FailingJob {
        const ID: JobId = JobId::new("failing.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            Err(Error::message("boom"))
        }

        fn max_retries(&self) -> Option<u32> {
            Some(1)
        }

        fn backoff(&self, _attempt: u32) -> Duration {
            Duration::from_millis(0)
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct PanickingJob;

    #[async_trait]
    impl Job for PanickingJob {
        const ID: JobId = JobId::new("panicking.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            panic!("job explode")
        }

        fn max_retries(&self) -> Option<u32> {
            Some(1)
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct FactoryPanickingJob;

    impl Job for FactoryPanickingJob {
        const ID: JobId = JobId::new("factory.panicking.job");

        fn handle<'life0, 'async_trait>(
            &'life0 self,
            _context: JobContext,
        ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            panic!("job factory explode")
        }

        fn max_retries(&self) -> Option<u32> {
            Some(1)
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct PanicThenSucceedJob;

    #[async_trait]
    impl Job for PanicThenSucceedJob {
        const ID: JobId = JobId::new("panic.then.succeed.job");

        async fn handle(&self, context: JobContext) -> crate::Result<()> {
            if context.attempt() == 1 {
                panic!("flaky panic")
            }
            Ok(())
        }

        fn max_retries(&self) -> Option<u32> {
            Some(2)
        }

        fn backoff(&self, _attempt: u32) -> Duration {
            Duration::from_millis(0)
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct UniqueOkJob {
        key: String,
    }

    #[async_trait]
    impl Job for UniqueOkJob {
        const ID: JobId = JobId::new("unique.ok.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            Ok(())
        }

        fn unique_for(&self) -> Option<Duration> {
            Some(Duration::from_secs(60))
        }

        fn unique_key(&self) -> Option<String> {
            Some(self.key.clone())
        }
    }

    #[derive(Debug, Deserialize)]
    struct UniqueSerializationFailJob;

    impl serde::Serialize for UniqueSerializationFailJob {
        fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("unique job serialization failed"))
        }
    }

    #[async_trait]
    impl Job for UniqueSerializationFailJob {
        const ID: JobId = JobId::new("unique.serialization.fail.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            Ok(())
        }

        fn unique_for(&self) -> Option<Duration> {
            Some(Duration::from_secs(60))
        }

        fn unique_key(&self) -> Option<String> {
            Some("fixed".to_string())
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct BlockingJob {
        tag: String,
    }

    #[async_trait]
    impl Job for BlockingJob {
        const ID: JobId = JobId::new("blocking.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            let mut state = take_worker_lifecycle_state(&self.tag);
            if let Some(started) = state.started.take() {
                let _ = started.send(());
            }

            let _guard = WorkerLifecycleGuard {
                completed: state.completed_flag.clone(),
                aborted: state.aborted_flag.clone(),
            };

            if let Some(release) = state.release.take() {
                let _ = release.await;
            } else {
                std::future::pending::<()>().await;
            }

            state.completed_flag.store(true, Ordering::SeqCst);
            if let Some(completed) = state.completed.take() {
                let _ = completed.send(());
            }
            Ok(())
        }
    }

    struct WorkerLifecycleState {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        release: Option<tokio::sync::oneshot::Receiver<()>>,
        completed: Option<tokio::sync::oneshot::Sender<()>>,
        completed_flag: Arc<AtomicBool>,
        aborted_flag: Arc<AtomicBool>,
    }

    struct WorkerLifecycleProbe {
        started: tokio::sync::oneshot::Receiver<()>,
        release: Option<tokio::sync::oneshot::Sender<()>>,
        completed: tokio::sync::oneshot::Receiver<()>,
        completed_flag: Arc<AtomicBool>,
        aborted_flag: Arc<AtomicBool>,
    }

    struct WorkerLifecycleGuard {
        completed: Arc<AtomicBool>,
        aborted: Arc<AtomicBool>,
    }

    impl Drop for WorkerLifecycleGuard {
        fn drop(&mut self) {
            if !self.completed.load(Ordering::SeqCst) {
                self.aborted.store(true, Ordering::SeqCst);
            }
        }
    }

    static WORKER_LIFECYCLE_STATES: std::sync::LazyLock<
        Mutex<std::collections::HashMap<String, WorkerLifecycleState>>,
    > = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

    fn worker_lifecycle_probe(tag: &str, releasable: bool) -> WorkerLifecycleProbe {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let completed_flag = Arc::new(AtomicBool::new(false));
        let aborted_flag = Arc::new(AtomicBool::new(false));

        WORKER_LIFECYCLE_STATES.lock().unwrap().insert(
            tag.to_string(),
            WorkerLifecycleState {
                started: Some(started_tx),
                release: releasable.then_some(release_rx),
                completed: Some(completed_tx),
                completed_flag: completed_flag.clone(),
                aborted_flag: aborted_flag.clone(),
            },
        );

        WorkerLifecycleProbe {
            started: started_rx,
            release: releasable.then_some(release_tx),
            completed: completed_rx,
            completed_flag,
            aborted_flag,
        }
    }

    fn take_worker_lifecycle_state(tag: &str) -> WorkerLifecycleState {
        WORKER_LIFECYCLE_STATES
            .lock()
            .unwrap()
            .remove(tag)
            .unwrap_or_else(|| panic!("missing lifecycle state for `{tag}`"))
    }

    async fn wait_for_flag(flag: &AtomicBool) {
        for _ in 0..50 {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("flag was not set");
    }

    fn build_app(runtime: Arc<JobRuntime>, diagnostics: Arc<RuntimeDiagnostics>) -> AppContext {
        let container = Container::new();
        let app = AppContext::new(
            container,
            crate::config::ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        app.container().singleton_arc(runtime).unwrap();
        app.container().singleton_arc(diagnostics).unwrap();
        app
    }

    fn build_blocking_runtime(
        namespace: &str,
        jobs_config: JobsConfig,
    ) -> (
        RuntimeBackend,
        Arc<JobRuntime>,
        Arc<RuntimeDiagnostics>,
        JobDispatcher,
    ) {
        let backend = RuntimeBackend::memory(namespace);
        let mut registry = JobRegistryBuilder::default();
        registry.register::<BlockingJob>().unwrap();

        let runtime = Arc::new(JobRuntime::new(
            backend.clone(),
            jobs_config.clone(),
            JobRegistryBuilder::freeze_shared(Arc::new(Mutex::new(registry)), &jobs_config),
        ));
        let diagnostics = Arc::new(RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(ReadinessRegistryBuilder::shared()),
        ));
        let dispatcher = JobDispatcher::new(runtime.clone(), diagnostics.clone());
        (backend, runtime, diagnostics, dispatcher)
    }

    fn build_panic_runtime(
        namespace: &str,
        jobs_config: JobsConfig,
    ) -> (
        RuntimeBackend,
        Arc<JobRuntime>,
        Arc<RuntimeDiagnostics>,
        JobDispatcher,
    ) {
        let backend = RuntimeBackend::memory(namespace);
        let mut registry = JobRegistryBuilder::default();
        registry.register::<PanickingJob>().unwrap();
        registry.register::<FactoryPanickingJob>().unwrap();
        registry.register::<PanicThenSucceedJob>().unwrap();

        let runtime = Arc::new(JobRuntime::new(
            backend.clone(),
            jobs_config.clone(),
            JobRegistryBuilder::freeze_shared(Arc::new(Mutex::new(registry)), &jobs_config),
        ));
        let diagnostics = Arc::new(RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(ReadinessRegistryBuilder::shared()),
        ));
        let dispatcher = JobDispatcher::new(runtime.clone(), diagnostics.clone());
        (backend, runtime, diagnostics, dispatcher)
    }

    #[tokio::test]
    async fn explicit_dispatch_queue_is_serialized_and_configured_dynamic_queues_are_polled() {
        let jobs_config = JobsConfig::default();
        let (backend, _runtime, _diagnostics, dispatcher) =
            build_panic_runtime("jobs-explicit-dispatch-queue", jobs_config);
        let queue = QueueId::new("mail-critical");

        dispatcher
            .dispatch_on(PanickingJob, queue.clone())
            .await
            .unwrap();
        let lease = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        let envelope: JobEnvelope = serde_json::from_str(&lease.payload).unwrap();
        assert_eq!(lease.queue, queue);
        assert_eq!(envelope.queue, queue);

        let error = dispatcher
            .dispatch_on(PanickingJob, QueueId::owned("  "))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("queue name cannot be empty"));

        let mut config = JobsConfig::default();
        config.queue_priorities.insert("dynamic-low".to_string(), 9);
        config
            .queue_priorities
            .insert("dynamic-high".to_string(), 1);
        let snapshot = JobRegistryBuilder::freeze_shared(JobRegistryBuilder::shared(), &config);
        assert!(snapshot.queues.contains(&QueueId::new("dynamic-low")));
        assert!(snapshot.queues.contains(&QueueId::new("dynamic-high")));
        assert!(
            snapshot
                .queues
                .iter()
                .position(|queue| queue == &QueueId::new("dynamic-high"))
                < snapshot
                    .queues
                    .iter()
                    .position(|queue| queue == &QueueId::new("dynamic-low"))
        );
    }

    struct PanicRecordingMiddleware {
        failed: Arc<Mutex<Vec<String>>>,
        dead_lettered: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl JobMiddleware for PanicRecordingMiddleware {
        async fn failed(
            &self,
            job_id: &JobId,
            _context: &JobContext,
            error: &str,
        ) -> crate::Result<()> {
            self.failed
                .lock()
                .unwrap()
                .push(format!("{job_id}:{error}"));
            Ok(())
        }

        async fn on_dead_lettered(&self, context: &JobDeadLetterContext) -> crate::Result<()> {
            self.dead_lettered
                .lock()
                .unwrap()
                .push(context.class.clone());
            Ok(())
        }
    }

    fn register_panic_middleware(
        app: &AppContext,
        failed: Arc<Mutex<Vec<String>>>,
        dead_lettered: Arc<Mutex<Vec<String>>>,
    ) {
        let mut middleware_builder = JobMiddlewareRegistryBuilder::default();
        middleware_builder.register(Arc::new(PanicRecordingMiddleware {
            failed,
            dead_lettered,
        }));
        app.container()
            .singleton_arc(Arc::new(JobMiddlewareRegistryBuilder::freeze_shared(
                Arc::new(Mutex::new(middleware_builder)),
            )))
            .unwrap();
    }

    #[tokio::test]
    async fn panicking_job_run_once_dead_letters_without_panicking() {
        let backend_namespace = "job-panic-dead-letter";
        let (backend, runtime, diagnostics, dispatcher) =
            build_panic_runtime(backend_namespace, JobsConfig::default());
        let app = build_app(runtime, diagnostics.clone());
        let failed = Arc::new(Mutex::new(Vec::new()));
        let dead_lettered = Arc::new(Mutex::new(Vec::new()));
        register_panic_middleware(&app, failed.clone(), dead_lettered.clone());

        dispatcher.dispatch(PanickingJob).await.unwrap();
        let worker = Worker::from_app(app).unwrap();

        assert!(worker.run_once().await.unwrap());

        let dead_letters = backend
            .dead_letters(&QueueId::new("default"))
            .await
            .unwrap();
        assert_eq!(dead_letters.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&dead_letters[0]).unwrap();
        assert_eq!(payload["error"], "job panicked: job explode");

        assert_eq!(
            failed.lock().unwrap().as_slice(),
            &["panicking.job:job panicked: job explode"]
        );
        assert_eq!(dead_lettered.lock().unwrap().as_slice(), &["panicking.job"]);

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.jobs.dead_lettered_total, 1);
        assert_eq!(snapshot.jobs.retried_total, 0);
        assert_eq!(snapshot.jobs.succeeded_total, 0);
    }

    #[tokio::test]
    async fn panicking_job_factory_run_once_dead_letters_without_panicking() {
        let backend_namespace = "job-factory-panic-dead-letter";
        let (backend, runtime, diagnostics, dispatcher) =
            build_panic_runtime(backend_namespace, JobsConfig::default());
        let app = build_app(runtime, diagnostics.clone());
        let failed = Arc::new(Mutex::new(Vec::new()));
        let dead_lettered = Arc::new(Mutex::new(Vec::new()));
        register_panic_middleware(&app, failed.clone(), dead_lettered.clone());

        dispatcher.dispatch(FactoryPanickingJob).await.unwrap();
        let worker = Worker::from_app(app).unwrap();

        assert!(worker.run_once().await.unwrap());

        let dead_letters = backend
            .dead_letters(&QueueId::new("default"))
            .await
            .unwrap();
        assert_eq!(dead_letters.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&dead_letters[0]).unwrap();
        assert_eq!(payload["error"], "job panicked: job factory explode");

        assert_eq!(
            failed.lock().unwrap().as_slice(),
            &["factory.panicking.job:job panicked: job factory explode"]
        );
        assert_eq!(
            dead_lettered.lock().unwrap().as_slice(),
            &["factory.panicking.job"]
        );

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.jobs.dead_lettered_total, 1);
        assert_eq!(snapshot.jobs.retried_total, 0);
        assert_eq!(snapshot.jobs.succeeded_total, 0);
    }

    #[tokio::test]
    async fn panicking_job_retries_then_succeeds() {
        let (_backend, runtime, diagnostics, dispatcher) = build_panic_runtime(
            "job-panic-retry",
            JobsConfig {
                poll_interval_ms: 1,
                lease_ttl_ms: 50,
                ..JobsConfig::default()
            },
        );
        let app = build_app(runtime, diagnostics.clone());
        let failed = Arc::new(Mutex::new(Vec::new()));
        let dead_lettered = Arc::new(Mutex::new(Vec::new()));
        register_panic_middleware(&app, failed.clone(), dead_lettered.clone());

        dispatcher.dispatch(PanicThenSucceedJob).await.unwrap();
        let worker = Worker::from_app(app).unwrap();

        assert!(worker.run_once().await.unwrap());
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.jobs.retried_total, 1);
        assert_eq!(snapshot.jobs.dead_lettered_total, 0);
        assert_eq!(snapshot.jobs.succeeded_total, 0);
        assert_eq!(
            failed.lock().unwrap().as_slice(),
            &["panic.then.succeed.job:job panicked: flaky panic"]
        );

        assert!(worker.run_once().await.unwrap());
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.jobs.retried_total, 1);
        assert_eq!(snapshot.jobs.dead_lettered_total, 0);
        assert_eq!(snapshot.jobs.succeeded_total, 1);
        assert!(dead_lettered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn worker_shutdown_waits_for_active_job_completion() {
        let jobs_config = JobsConfig {
            poll_interval_ms: 1,
            lease_ttl_ms: 50,
            shutdown_timeout_ms: 500,
            ..JobsConfig::default()
        };
        let (_backend, runtime, diagnostics, dispatcher) =
            build_blocking_runtime("worker-shutdown-drain", jobs_config);
        let app = build_app(runtime, diagnostics);
        let mut probe = worker_lifecycle_probe("drain", true);

        dispatcher
            .dispatch(BlockingJob {
                tag: "drain".to_string(),
            })
            .await
            .unwrap();

        let worker = Worker::from_app(app).unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let worker_task = tokio::spawn(async move {
            worker
                .run_until(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        probe.started.await.unwrap();
        shutdown_tx.send(()).unwrap();
        probe.release.take().unwrap().send(()).unwrap();
        probe.completed.await.unwrap();

        tokio::time::timeout(Duration::from_millis(500), worker_task)
            .await
            .unwrap()
            .unwrap();
        assert!(probe.completed_flag.load(Ordering::SeqCst));
        assert!(!probe.aborted_flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn worker_shutdown_aborts_active_job_after_timeout_and_requeues_after_lease_expiry() {
        let jobs_config = JobsConfig {
            poll_interval_ms: 1,
            lease_ttl_ms: 30,
            shutdown_timeout_ms: 1,
            max_concurrent_jobs: 1,
            ..JobsConfig::default()
        };
        let (_backend, runtime, diagnostics, dispatcher) =
            build_blocking_runtime("worker-shutdown-abort", jobs_config);
        let app = build_app(runtime.clone(), diagnostics);
        let probe = worker_lifecycle_probe("abort", false);

        dispatcher
            .dispatch(BlockingJob {
                tag: "abort".to_string(),
            })
            .await
            .unwrap();

        let worker = Worker::from_app(app).unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let worker_task = tokio::spawn(async move {
            worker
                .run_until(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        probe.started.await.unwrap();
        shutdown_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_millis(500), worker_task)
            .await
            .unwrap()
            .unwrap();
        wait_for_flag(&probe.aborted_flag).await;
        assert!(!probe.completed_flag.load(Ordering::SeqCst));

        tokio::time::sleep(Duration::from_millis(80)).await;
        let requeued = runtime
            .requeue_expired_jobs(chrono::Utc::now().timestamp_millis())
            .await
            .unwrap();
        assert_eq!(requeued, 1);
        assert!(runtime.claim_job().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn aborting_worker_coordinator_aborts_active_jobs() {
        let jobs_config = JobsConfig {
            poll_interval_ms: 1,
            lease_ttl_ms: 50,
            shutdown_timeout_ms: 500,
            max_concurrent_jobs: 1,
            ..JobsConfig::default()
        };
        let (_backend, runtime, diagnostics, dispatcher) =
            build_blocking_runtime("worker-coordinator-abort", jobs_config);
        let app = build_app(runtime, diagnostics);
        let probe = worker_lifecycle_probe("coordinator-abort", false);

        dispatcher
            .dispatch(BlockingJob {
                tag: "coordinator-abort".to_string(),
            })
            .await
            .unwrap();

        let worker = Worker::from_app(app).unwrap();
        let worker_task = tokio::spawn(async move {
            worker
                .run_until(std::future::pending::<()>())
                .await
                .unwrap();
        });

        probe.started.await.unwrap();
        worker_task.abort();
        let _ = worker_task.await;
        wait_for_flag(&probe.aborted_flag).await;
        assert!(!probe.completed_flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn moves_failed_jobs_to_dead_letter() {
        let _guard = tracing::subscriber::set_default(tracing::subscriber::NoSubscriber::default());
        let backend = RuntimeBackend::memory("jobs-unit-tests");
        let mut registry = JobRegistryBuilder::default();
        registry.register::<FailingJob>().unwrap();

        let jobs_config = JobsConfig {
            max_retries: 1,
            poll_interval_ms: 1,
            lease_ttl_ms: 50,
            requeue_batch_size: 8,
            ..JobsConfig::default()
        };
        let runtime = Arc::new(JobRuntime::new(
            backend.clone(),
            jobs_config.clone(),
            JobRegistryBuilder::freeze_shared(Arc::new(Mutex::new(registry)), &jobs_config),
        ));
        let diagnostics = Arc::new(RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(ReadinessRegistryBuilder::shared()),
        ));
        let dispatcher = JobDispatcher::new(runtime.clone(), diagnostics.clone());
        let app = build_app(runtime.clone(), diagnostics);

        dispatcher.dispatch(FailingJob).await.unwrap();
        let worker = Worker::from_app(app).unwrap();
        assert!(worker.run_once().await.unwrap());

        let dead_letters = backend
            .dead_letters(&QueueId::new("default"))
            .await
            .unwrap();
        assert_eq!(dead_letters.len(), 1);
    }

    struct RecordingMiddleware {
        target: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl JobMiddleware for RecordingMiddleware {
        async fn on_dead_lettered(&self, context: &JobDeadLetterContext) -> crate::Result<()> {
            self.target
                .lock()
                .unwrap()
                .push(format!("{}:{}", context.class, context.id));
            Ok(())
        }
    }

    #[derive(Default)]
    struct PanickingMiddleware {
        before: bool,
        after: bool,
        failed: bool,
        dead_lettered: bool,
    }

    #[async_trait]
    impl JobMiddleware for PanickingMiddleware {
        async fn before(&self, _job_id: &JobId, _context: &JobContext) -> crate::Result<()> {
            if self.before {
                panic!("middleware before explode");
            }
            Ok(())
        }

        async fn after(&self, _job_id: &JobId, _context: &JobContext) -> crate::Result<()> {
            if self.after {
                panic!("middleware after explode");
            }
            Ok(())
        }

        async fn failed(
            &self,
            _job_id: &JobId,
            _context: &JobContext,
            _error: &str,
        ) -> crate::Result<()> {
            if self.failed {
                panic!("middleware failed explode");
            }
            Ok(())
        }

        async fn on_dead_lettered(&self, _context: &JobDeadLetterContext) -> crate::Result<()> {
            if self.dead_lettered {
                panic!("middleware dead-letter explode");
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct FactoryPanickingMiddleware {
        before: bool,
        after: bool,
        failed: bool,
        dead_lettered: bool,
    }

    impl JobMiddleware for FactoryPanickingMiddleware {
        fn before<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _job_id: &'life1 JobId,
            _context: &'life2 JobContext,
        ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            if self.before {
                panic!("middleware before factory explode");
            }
            Box::pin(async { Ok(()) })
        }

        fn after<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _job_id: &'life1 JobId,
            _context: &'life2 JobContext,
        ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            if self.after {
                panic!("middleware after factory explode");
            }
            Box::pin(async { Ok(()) })
        }

        fn failed<'life0, 'life1, 'life2, 'life3, 'async_trait>(
            &'life0 self,
            _job_id: &'life1 JobId,
            _context: &'life2 JobContext,
            _error: &'life3 str,
        ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            'life3: 'async_trait,
            Self: 'async_trait,
        {
            if self.failed {
                panic!("middleware failed factory explode");
            }
            Box::pin(async { Ok(()) })
        }

        fn on_dead_lettered<'life0, 'life1, 'async_trait>(
            &'life0 self,
            _context: &'life1 JobDeadLetterContext,
        ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            if self.dead_lettered {
                panic!("middleware dead-letter factory explode");
            }
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Default)]
    struct SlowMiddleware {
        before_started: Option<Arc<Notify>>,
        before_delay: Option<Duration>,
        after_started: Option<Arc<Notify>>,
        after_delay: Option<Duration>,
    }

    #[async_trait]
    impl JobMiddleware for SlowMiddleware {
        async fn before(&self, _job_id: &JobId, _context: &JobContext) -> crate::Result<()> {
            if let Some(started) = &self.before_started {
                started.notify_one();
            }
            if let Some(delay) = self.before_delay {
                tokio::time::sleep(delay).await;
            }
            Ok(())
        }

        async fn after(&self, _job_id: &JobId, _context: &JobContext) -> crate::Result<()> {
            if let Some(started) = &self.after_started {
                started.notify_one();
            }
            if let Some(delay) = self.after_delay {
                tokio::time::sleep(delay).await;
            }
            Ok(())
        }
    }

    fn register_job_middleware(app: &AppContext, middleware: Arc<dyn JobMiddleware>) {
        let mut middleware_builder = JobMiddlewareRegistryBuilder::default();
        middleware_builder.register(middleware);
        app.container()
            .singleton_arc(Arc::new(JobMiddlewareRegistryBuilder::freeze_shared(
                Arc::new(Mutex::new(middleware_builder)),
            )))
            .unwrap();
    }

    #[tokio::test]
    async fn dead_lettered_jobs_trigger_middleware_hook() {
        let _guard = tracing::subscriber::set_default(tracing::subscriber::NoSubscriber::default());
        let backend = RuntimeBackend::memory("jobs-dead-letter-hook");
        let mut registry = JobRegistryBuilder::default();
        registry.register::<FailingJob>().unwrap();

        let jobs_config = JobsConfig {
            max_retries: 1,
            poll_interval_ms: 1,
            lease_ttl_ms: 50,
            requeue_batch_size: 8,
            ..JobsConfig::default()
        };
        let runtime = Arc::new(JobRuntime::new(
            backend,
            jobs_config.clone(),
            JobRegistryBuilder::freeze_shared(Arc::new(Mutex::new(registry)), &jobs_config),
        ));
        let diagnostics = Arc::new(RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(ReadinessRegistryBuilder::shared()),
        ));
        let dispatcher = JobDispatcher::new(runtime.clone(), diagnostics.clone());
        let app = build_app(runtime, diagnostics);
        let target = Arc::new(Mutex::new(Vec::new()));
        register_job_middleware(
            &app,
            Arc::new(RecordingMiddleware {
                target: target.clone(),
            }),
        );

        dispatcher.dispatch(FailingJob).await.unwrap();
        let worker = Worker::from_app(app).unwrap();
        assert!(worker.run_once().await.unwrap());

        let entries = target.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].starts_with("failing.job:"));
    }

    #[tokio::test]
    async fn middleware_before_after_panics_do_not_block_success_finalization() {
        let tag = "middleware-success-panic";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("middleware-success-panic");
        let app = build_app(runtime, diagnostics.clone());
        register_job_middleware(
            &app,
            Arc::new(PanickingMiddleware {
                before: true,
                after: true,
                ..PanickingMiddleware::default()
            }),
        );

        dispatcher
            .dispatch(StepJob {
                tag: tag.into(),
                name: "ok".into(),
            })
            .await
            .unwrap();
        let worker = Worker::from_app(app).unwrap();

        assert!(worker.run_once().await.unwrap());
        assert!(!worker.run_once().await.unwrap());
        assert_eq!(read_log_filtered(&format!("{tag}:")), vec!["ok"]);
        assert_eq!(diagnostics.snapshot().jobs.succeeded_total, 1);
    }

    #[tokio::test]
    async fn middleware_before_after_factory_panics_do_not_block_success_finalization() {
        let tag = "middleware-success-factory-panic";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("middleware-success-factory-panic");
        let app = build_app(runtime, diagnostics.clone());
        register_job_middleware(
            &app,
            Arc::new(FactoryPanickingMiddleware {
                before: true,
                after: true,
                ..FactoryPanickingMiddleware::default()
            }),
        );

        dispatcher
            .dispatch(StepJob {
                tag: tag.into(),
                name: "ok".into(),
            })
            .await
            .unwrap();
        let worker = Worker::from_app(app).unwrap();

        assert!(worker.run_once().await.unwrap());
        assert!(!worker.run_once().await.unwrap());
        assert_eq!(read_log_filtered(&format!("{tag}:")), vec!["ok"]);
        assert_eq!(diagnostics.snapshot().jobs.succeeded_total, 1);
    }

    #[tokio::test]
    async fn middleware_failure_panics_do_not_block_dead_letter_transition() {
        let (backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("middleware-failure-panic");
        let app = build_app(runtime, diagnostics.clone());
        register_job_middleware(
            &app,
            Arc::new(PanickingMiddleware {
                failed: true,
                dead_lettered: true,
                ..PanickingMiddleware::default()
            }),
        );

        dispatcher.dispatch(FailingJob).await.unwrap();
        let worker = Worker::from_app(app).unwrap();

        assert!(worker.run_once().await.unwrap());
        let dead_letters = backend
            .dead_letters(&QueueId::new("default"))
            .await
            .unwrap();
        assert_eq!(dead_letters.len(), 1);
        assert_eq!(diagnostics.snapshot().jobs.dead_lettered_total, 1);
    }

    #[tokio::test]
    async fn middleware_failure_factory_panics_do_not_block_dead_letter_transition() {
        let (backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("middleware-failure-factory-panic");
        let app = build_app(runtime, diagnostics.clone());
        register_job_middleware(
            &app,
            Arc::new(FactoryPanickingMiddleware {
                failed: true,
                dead_lettered: true,
                ..FactoryPanickingMiddleware::default()
            }),
        );

        dispatcher.dispatch(FailingJob).await.unwrap();
        let worker = Worker::from_app(app).unwrap();

        assert!(worker.run_once().await.unwrap());
        let dead_letters = backend
            .dead_letters(&QueueId::new("default"))
            .await
            .unwrap();
        assert_eq!(dead_letters.len(), 1);
        assert_eq!(diagnostics.snapshot().jobs.dead_lettered_total, 1);
    }

    #[tokio::test]
    async fn lease_heartbeat_covers_slow_before_middleware() {
        let tag = "middleware-before-heartbeat";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("middleware-before-heartbeat");
        let slow_app = build_app(runtime.clone(), diagnostics.clone());
        let competing_app = build_app(runtime, diagnostics.clone());
        let before_started = Arc::new(Notify::new());
        register_job_middleware(
            &slow_app,
            Arc::new(SlowMiddleware {
                before_started: Some(before_started.clone()),
                before_delay: Some(Duration::from_millis(120)),
                ..SlowMiddleware::default()
            }),
        );

        dispatcher
            .dispatch(StepJob {
                tag: tag.into(),
                name: "ok".into(),
            })
            .await
            .unwrap();
        let slow_worker = Worker::from_app(slow_app).unwrap();
        let competing_worker = Worker::from_app(competing_app).unwrap();

        let slow_handle = tokio::spawn(async move { slow_worker.run_once().await.unwrap() });
        tokio::time::timeout(Duration::from_secs(1), before_started.notified())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(70)).await;

        assert!(!competing_worker.run_once().await.unwrap());
        assert!(slow_handle.await.unwrap());
        assert!(!competing_worker.run_once().await.unwrap());
        assert_eq!(read_log_filtered(&format!("{tag}:")), vec!["ok"]);
        assert_eq!(diagnostics.snapshot().jobs.succeeded_total, 1);
    }

    #[tokio::test]
    async fn lease_heartbeat_covers_slow_after_middleware() {
        let tag = "middleware-after-heartbeat";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("middleware-after-heartbeat");
        let slow_app = build_app(runtime.clone(), diagnostics.clone());
        let competing_app = build_app(runtime, diagnostics.clone());
        let after_started = Arc::new(Notify::new());
        register_job_middleware(
            &slow_app,
            Arc::new(SlowMiddleware {
                after_started: Some(after_started.clone()),
                after_delay: Some(Duration::from_millis(120)),
                ..SlowMiddleware::default()
            }),
        );

        dispatcher
            .dispatch(StepJob {
                tag: tag.into(),
                name: "ok".into(),
            })
            .await
            .unwrap();
        let slow_worker = Worker::from_app(slow_app).unwrap();
        let competing_worker = Worker::from_app(competing_app).unwrap();

        let slow_handle = tokio::spawn(async move { slow_worker.run_once().await.unwrap() });
        tokio::time::timeout(Duration::from_secs(1), after_started.notified())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(70)).await;

        assert!(!competing_worker.run_once().await.unwrap());
        assert!(slow_handle.await.unwrap());
        assert!(!competing_worker.run_once().await.unwrap());
        assert_eq!(read_log_filtered(&format!("{tag}:")), vec!["ok"]);
        assert_eq!(diagnostics.snapshot().jobs.succeeded_total, 1);
    }

    #[test]
    fn lease_finalization_and_loss_have_exactly_one_winner() {
        for _ in 0..32 {
            let finalization = super::LeaseFinalization::default();
            let barrier = Arc::new(std::sync::Barrier::new(3));

            let (finalization_won, loss_won) = std::thread::scope(|scope| {
                let finalization_state = finalization.clone();
                let finalization_barrier = barrier.clone();
                let finalization_thread = scope.spawn(move || {
                    finalization_barrier.wait();
                    finalization_state.begin()
                });

                let loss_state = finalization.clone();
                let loss_barrier = barrier.clone();
                let loss_thread = scope.spawn(move || {
                    loss_barrier.wait();
                    loss_state.mark_lost()
                });

                barrier.wait();
                (
                    finalization_thread.join().unwrap(),
                    loss_thread.join().unwrap(),
                )
            });

            assert_ne!(finalization_won, loss_won);
        }
    }

    #[tokio::test]
    async fn lease_heartbeat_keeps_finalizing_lease_alive_and_ignores_its_release() {
        let (backend, runtime, _diagnostics, _dispatcher) =
            build_runtime_and_dispatcher("heartbeat-finalization-release");
        let queue = QueueId::new("default");
        backend
            .enqueue_job(&queue, "heartbeat-finalization-token", "{}")
            .await
            .unwrap();
        let lease = runtime.claim_job().await.unwrap().unwrap();
        let heartbeat =
            super::LeaseHeartbeat::spawn(runtime.clone(), lease.queue.clone(), lease.token.clone());
        let lost_signal = heartbeat.lease_lost_signal();
        let finalization = heartbeat.finalization();

        assert!(finalization.begin());
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(
            runtime
                .requeue_expired_jobs(Utc::now().timestamp_millis())
                .await
                .unwrap(),
            0
        );

        let completion = runtime
            .complete_successful_job(&lease.queue, &lease.token, SuccessfulJobEffects::default())
            .await
            .unwrap();
        assert!(completion.lease_released);

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(!*lost_signal.borrow());
        heartbeat.shutdown().await;
    }

    #[tokio::test]
    async fn lease_heartbeat_reports_release_that_did_not_begin_finalization() {
        let (backend, runtime, _diagnostics, _dispatcher) =
            build_runtime_and_dispatcher("heartbeat-unexpected-release");
        let queue = QueueId::new("default");
        backend
            .enqueue_job(&queue, "heartbeat-unexpected-token", "{}")
            .await
            .unwrap();
        let lease = runtime.claim_job().await.unwrap().unwrap();
        let heartbeat =
            super::LeaseHeartbeat::spawn(runtime.clone(), lease.queue.clone(), lease.token.clone());
        let lost_signal = heartbeat.lease_lost_signal();

        let completion = runtime
            .complete_successful_job(&lease.queue, &lease.token, SuccessfulJobEffects::default())
            .await
            .unwrap();
        assert!(completion.lease_released);

        tokio::time::timeout(
            Duration::from_secs(1),
            super::lease_lost(lost_signal.clone()),
        )
        .await
        .unwrap();
        assert!(*lost_signal.borrow());
        heartbeat.shutdown().await;
    }

    // --- Batch & chain test helpers ---

    static EXECUTION_LOG: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

    fn append_log(entry: String) {
        EXECUTION_LOG.lock().unwrap().push(entry);
    }

    fn read_log_filtered(prefix: &str) -> Vec<String> {
        EXECUTION_LOG
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.starts_with(prefix))
            .map(|e| e.strip_prefix(prefix).unwrap_or(e).to_string())
            .collect()
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct StepJob {
        tag: String,
        name: String,
    }

    #[async_trait]
    impl Job for StepJob {
        const ID: JobId = JobId::new("step.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            append_log(format!("{}:{}", self.tag, self.name));
            Ok(())
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct TraceCaptureJob {
        tag: String,
    }

    #[async_trait]
    impl Job for TraceCaptureJob {
        const ID: JobId = JobId::new("trace.capture.job");

        async fn handle(&self, context: JobContext) -> crate::Result<()> {
            append_log(format!(
                "{}:{}:{}",
                self.tag,
                context.trace_id().unwrap_or("none"),
                context.request_id().unwrap_or("none")
            ));
            Ok(())
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct CompletionJob {
        tag: String,
        label: String,
    }

    #[async_trait]
    impl Job for CompletionJob {
        const ID: JobId = JobId::new("completion.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            append_log(format!("{}:complete:{}", self.tag, self.label));
            Ok(())
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct RateLimitedJob {
        tag: String,
    }

    #[async_trait]
    impl Job for RateLimitedJob {
        const ID: JobId = JobId::new("rate.limited.job");

        async fn handle(&self, _context: JobContext) -> crate::Result<()> {
            append_log(format!("{}:handled", self.tag));
            Ok(())
        }

        fn rate_limit(&self) -> Option<(u32, Duration)> {
            Some((0, Duration::from_secs(60)))
        }
    }

    fn build_runtime_and_dispatcher(
        namespace: &str,
    ) -> (
        RuntimeBackend,
        Arc<JobRuntime>,
        Arc<RuntimeDiagnostics>,
        JobDispatcher,
    ) {
        let backend = RuntimeBackend::memory(namespace);
        let mut registry = JobRegistryBuilder::default();
        registry.register::<FailingJob>().unwrap();
        registry.register::<StepJob>().unwrap();
        registry.register::<TraceCaptureJob>().unwrap();
        registry.register::<CompletionJob>().unwrap();
        registry.register::<RateLimitedJob>().unwrap();

        let jobs_config = JobsConfig {
            max_retries: 1,
            poll_interval_ms: 1,
            lease_ttl_ms: 50,
            requeue_batch_size: 8,
            ..JobsConfig::default()
        };
        let runtime = Arc::new(JobRuntime::new(
            backend.clone(),
            jobs_config.clone(),
            JobRegistryBuilder::freeze_shared(Arc::new(Mutex::new(registry)), &jobs_config),
        ));
        let diagnostics = Arc::new(RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(ReadinessRegistryBuilder::shared()),
        ));
        let dispatcher = JobDispatcher::new(runtime.clone(), diagnostics.clone());
        (backend, runtime, diagnostics, dispatcher)
    }

    async fn scheduled_run_times(backend: &RuntimeBackend) -> Vec<i64> {
        let runtime = match backend {
            RuntimeBackend::Memory(runtime) => runtime,
            RuntimeBackend::Redis(_) => unreachable!("test uses memory runtime"),
        };
        runtime
            .scheduled_jobs
            .lock()
            .await
            .get(&QueueId::new("default"))
            .map(|jobs| jobs.iter().map(|job| job.run_at_millis).collect())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn dispatch_at_accepts_foundry_datetime() {
        let (backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("dispatch-at-datetime");
        let run_at = DateTime::now().add_days(365);

        dispatcher.dispatch_at(FailingJob, run_at).await.unwrap();

        assert_eq!(
            scheduled_run_times(&backend).await,
            [run_at.timestamp_millis()]
        );
    }

    #[tokio::test]
    async fn dispatch_later_preserves_raw_epoch_milliseconds() {
        let (backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("dispatch-later-epoch");
        let run_at_millis = DateTime::now().add_days(365).timestamp_millis();

        dispatcher
            .dispatch_later(FailingJob, run_at_millis)
            .await
            .unwrap();

        assert_eq!(scheduled_run_times(&backend).await, [run_at_millis]);
    }

    #[tokio::test]
    async fn dispatch_after_uses_checked_duration_arithmetic() {
        let (backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("dispatch-after-duration");
        let before = Utc::now().timestamp_millis();

        dispatcher
            .dispatch_after(FailingJob, Duration::from_secs(60))
            .await
            .unwrap();

        let after = Utc::now().timestamp_millis();
        let scheduled = scheduled_run_times(&backend).await;
        assert_eq!(scheduled.len(), 1);
        assert!(scheduled[0] >= before + 60_000);
        assert!(scheduled[0] <= after + 60_000);
    }

    #[tokio::test]
    async fn dispatch_after_rejects_timestamp_overflow_before_enqueueing() {
        let (backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("dispatch-after-overflow");

        let error = dispatcher
            .dispatch_after(FailingJob, Duration::MAX)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("supported timestamp range"));
        assert!(scheduled_run_times(&backend).await.is_empty());
        assert!(checked_dispatch_time_after(i64::MAX, Duration::from_millis(1)).is_err());
    }

    #[tokio::test]
    async fn malformed_job_envelope_is_dead_lettered_without_requeue_loop() {
        let (backend, runtime, diagnostics, _dispatcher) =
            build_runtime_and_dispatcher("poison-malformed-envelope");
        let queue = QueueId::new("default");
        backend
            .enqueue_job(&queue, "poison-token", "not-json")
            .await
            .unwrap();

        let app = build_app(runtime.clone(), diagnostics.clone());
        let worker = Worker::from_app(app).unwrap();
        assert!(worker.run_once().await.unwrap());

        let dead_letters = backend.dead_letters(&queue).await.unwrap();
        assert_eq!(dead_letters.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&dead_letters[0]).unwrap();
        assert!(payload["error"]
            .as_str()
            .unwrap()
            .starts_with("job envelope could not be deserialized:"));
        assert_eq!(payload["envelope"]["job"], "foundry.invalid_job_envelope");
        assert_eq!(payload["envelope"]["payload"], "not-json");

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.jobs.dead_lettered_total, 1);
        assert_eq!(
            runtime
                .requeue_expired_jobs(chrono::Utc::now().timestamp_millis())
                .await
                .unwrap(),
            0
        );
        assert!(runtime.claim_job().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unregistered_job_envelope_is_dead_lettered_without_requeue_loop() {
        let (backend, runtime, diagnostics, _dispatcher) =
            build_runtime_and_dispatcher("poison-unregistered-envelope");
        let queue = QueueId::new("default");
        let envelope = JobEnvelope {
            job: JobId::new("missing.job"),
            queue: queue.clone(),
            attempts: 0,
            scheduled_at: chrono::Utc::now().timestamp_millis(),
            payload: serde_json::json!({ "id": 123 }),
            trace: None,
            batch_id: None,
            chain_remaining: None,
        };
        let payload = serde_json::to_string(&envelope).unwrap();
        backend
            .enqueue_job(&queue, "missing-token", &payload)
            .await
            .unwrap();

        let app = build_app(runtime.clone(), diagnostics.clone());
        let worker = Worker::from_app(app).unwrap();
        assert!(worker.run_once().await.unwrap());

        let dead_letters = backend.dead_letters(&queue).await.unwrap();
        assert_eq!(dead_letters.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&dead_letters[0]).unwrap();
        assert_eq!(payload["error"], "job `missing.job` is not registered");
        assert_eq!(payload["envelope"]["job"], "missing.job");
        assert_eq!(payload["envelope"]["attempts"], 1);

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.jobs.dead_lettered_total, 1);
        assert_eq!(
            runtime
                .requeue_expired_jobs(chrono::Utc::now().timestamp_millis())
                .await
                .unwrap(),
            0
        );
        assert!(runtime.claim_job().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unique_dispatch_serialization_failure_rolls_back_reservation() {
        let (backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("unique-serialization-rollback");
        let unique_key = format!(
            "jobs:unique:{}:{}",
            <UniqueSerializationFailJob as Job>::ID,
            "fixed"
        );

        let result = dispatcher.dispatch(UniqueSerializationFailJob).await;

        assert!(result.is_err());
        assert!(!backend.key_exists(&unique_key).await.unwrap());
        assert!(backend
            .set_nx_value(&unique_key, "after-rollback", 60)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn successful_unique_dispatch_keeps_reservation_and_skips_duplicates() {
        let (backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("unique-success-keeps-reservation");
        let unique_key = format!("jobs:unique:{}:{}", <UniqueOkJob as Job>::ID, "same");
        let queue = QueueId::new("default");

        dispatcher
            .dispatch(UniqueOkJob {
                key: "same".to_string(),
            })
            .await
            .unwrap();
        dispatcher
            .dispatch(UniqueOkJob {
                key: "same".to_string(),
            })
            .await
            .unwrap();

        assert!(backend.key_exists(&unique_key).await.unwrap());
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_some());
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn dispatched_job_preserves_current_trace_context() {
        let tag = "trace-dispatch";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("trace-dispatch");
        let app = build_app(runtime, diagnostics);

        crate::logging::scope_current_trace(
            crate::logging::TraceContext::http("req-job-trace".to_string()),
            dispatcher.dispatch(TraceCaptureJob { tag: tag.into() }),
        )
        .await
        .unwrap();

        let worker = Worker::from_app(app).unwrap();
        assert!(worker.run_once().await.unwrap());

        assert_eq!(
            read_log_filtered(&format!("{tag}:")),
            vec!["req-job-trace:req-job-trace"]
        );
    }

    #[tokio::test]
    async fn batch_dispatches_all_jobs_and_fires_on_complete() {
        let tag = "batch1";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("batch-complete");
        let app = build_app(runtime, diagnostics);

        let batch_id = dispatcher
            .batch("test")
            .add(StepJob {
                tag: tag.into(),
                name: "a".into(),
            })
            .unwrap()
            .add(StepJob {
                tag: tag.into(),
                name: "b".into(),
            })
            .unwrap()
            .on_complete(CompletionJob {
                tag: tag.into(),
                label: "done".into(),
            })
            .unwrap()
            .dispatch()
            .await
            .unwrap();
        assert!(batch_id.starts_with("batch-test-"));

        let worker = Worker::from_app(app).unwrap();
        // Process both batch jobs
        assert!(worker.run_once().await.unwrap());
        assert!(worker.run_once().await.unwrap());
        // Process the on_complete callback
        assert!(worker.run_once().await.unwrap());

        let log = read_log_filtered(&format!("{tag}:"));
        // The two step jobs executed (order may vary), then the completion
        assert!(log.contains(&"a".to_string()));
        assert!(log.contains(&"b".to_string()));
        assert!(log.contains(&"complete:done".to_string()));
        // Completion is always last
        assert_eq!(log.last().unwrap(), "complete:done");
    }

    #[tokio::test]
    async fn batch_without_on_complete_works() {
        let tag = "batch2";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("batch-no-callback");
        let app = build_app(runtime, diagnostics);

        dispatcher
            .batch("simple")
            .add(StepJob {
                tag: tag.into(),
                name: "x".into(),
            })
            .unwrap()
            .dispatch()
            .await
            .unwrap();

        let worker = Worker::from_app(app).unwrap();
        assert!(worker.run_once().await.unwrap());
        // No more work
        assert!(!worker.run_once().await.unwrap());

        let log = read_log_filtered(&format!("{tag}:"));
        assert_eq!(log, vec!["x"]);
    }

    #[tokio::test]
    async fn chain_executes_jobs_sequentially() {
        let tag = "chain1";
        let (_backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("chain-sequential");
        let app = build_app(runtime, diagnostics);

        dispatcher
            .chain()
            .add(StepJob {
                tag: tag.into(),
                name: "first".into(),
            })
            .unwrap()
            .add(StepJob {
                tag: tag.into(),
                name: "second".into(),
            })
            .unwrap()
            .add(StepJob {
                tag: tag.into(),
                name: "third".into(),
            })
            .unwrap()
            .dispatch()
            .await
            .unwrap();

        let worker = Worker::from_app(app).unwrap();
        // Process chain — each run_once handles one job and enqueues the next
        for _ in 0..10 {
            let _ = worker.run_once().await;
        }

        let log = read_log_filtered(&format!("{tag}:"));
        assert_eq!(log, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn success_finalization_lost_lease_does_not_dispatch_chain_continuation() {
        let tag = "chain-lost-lease";
        let (backend, runtime, diagnostics, _dispatcher) =
            build_runtime_and_dispatcher("chain-lost-lease");
        let app = build_app(runtime.clone(), diagnostics.clone());
        let queue = QueueId::new("default");
        let first = StepJob {
            tag: tag.into(),
            name: "first".into(),
        };
        let second = StepJob {
            tag: tag.into(),
            name: "second".into(),
        };
        let envelope = JobEnvelope {
            job: StepJob::ID,
            queue: queue.clone(),
            attempts: 0,
            scheduled_at: chrono::Utc::now().timestamp_millis(),
            payload: serde_json::to_value(first).unwrap(),
            trace: None,
            batch_id: None,
            chain_remaining: Some(vec![ChainedJob {
                job: StepJob::ID,
                queue: queue.clone(),
                payload: serde_json::to_value(second).unwrap(),
            }]),
        };
        let payload = serde_json::to_string(&envelope).unwrap();
        backend
            .enqueue_job(&queue, "lost-success-token", &payload)
            .await
            .unwrap();

        let lease = runtime.claim_job().await.unwrap().unwrap();
        let completed = backend
            .complete_successful_job(
                &queue,
                &lease.token,
                &queue,
                SuccessfulJobEffects::default(),
            )
            .await
            .unwrap();
        assert!(completed.lease_released);

        let worker = Worker::from_app(app).unwrap();
        worker.process_claimed_job(lease).await.unwrap();

        assert!(runtime.claim_job().await.unwrap().is_none());
        let log = read_log_filtered(&format!("{tag}:"));
        assert_eq!(log, vec!["first"]);
        assert_eq!(diagnostics.snapshot().jobs.succeeded_total, 0);
    }

    #[tokio::test]
    async fn rate_limit_requeue_lost_lease_does_not_schedule_duplicate_job() {
        let tag = "rate-limit-lost-lease";
        let (backend, runtime, diagnostics, _dispatcher) =
            build_runtime_and_dispatcher("rate-limit-lost-lease");
        let app = build_app(runtime.clone(), diagnostics.clone());
        let queue = QueueId::new("default");
        let envelope = JobEnvelope {
            job: RateLimitedJob::ID,
            queue: queue.clone(),
            attempts: 0,
            scheduled_at: chrono::Utc::now().timestamp_millis(),
            payload: serde_json::to_value(RateLimitedJob { tag: tag.into() }).unwrap(),
            trace: None,
            batch_id: None,
            chain_remaining: None,
        };
        backend
            .enqueue_job(
                &queue,
                "rate-limit-lost-token",
                &serde_json::to_string(&envelope).unwrap(),
            )
            .await
            .unwrap();

        let lease = runtime.claim_job().await.unwrap().unwrap();
        let completed = backend
            .complete_successful_job(
                &queue,
                &lease.token,
                &queue,
                SuccessfulJobEffects::default(),
            )
            .await
            .unwrap();
        assert!(completed.lease_released);

        let worker = Worker::from_app(app).unwrap();
        worker.process_claimed_job(lease).await.unwrap();

        assert_eq!(
            runtime
                .promote_due_jobs(chrono::Utc::now().timestamp_millis() + 2_000)
                .await
                .unwrap(),
            0
        );
        assert!(runtime.claim_job().await.unwrap().is_none());
        assert!(read_log_filtered(&format!("{tag}:")).is_empty());

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.jobs.retried_total, 0);
        assert_eq!(snapshot.jobs.succeeded_total, 0);
        assert_eq!(snapshot.jobs.dead_lettered_total, 0);
    }

    #[tokio::test]
    async fn batch_on_complete_is_enqueued_once_when_completion_count_exceeds_total() {
        let tag = "batch-callback-once";
        let (backend, runtime, diagnostics, dispatcher) =
            build_runtime_and_dispatcher("batch-callback-once");
        let app = build_app(runtime, diagnostics);
        let queue = QueueId::new("default");

        let batch_id = dispatcher
            .batch("callback-once")
            .add(StepJob {
                tag: tag.into(),
                name: "primary".into(),
            })
            .unwrap()
            .on_complete(CompletionJob {
                tag: tag.into(),
                label: "done".into(),
            })
            .unwrap()
            .dispatch()
            .await
            .unwrap();

        let duplicate = StepJob {
            tag: tag.into(),
            name: "duplicate".into(),
        };
        let duplicate_envelope = JobEnvelope {
            job: StepJob::ID,
            queue: queue.clone(),
            attempts: 0,
            scheduled_at: chrono::Utc::now().timestamp_millis(),
            payload: serde_json::to_value(duplicate).unwrap(),
            trace: None,
            batch_id: Some(batch_id),
            chain_remaining: None,
        };
        backend
            .enqueue_job(
                &queue,
                "duplicate-batch-completion-token",
                &serde_json::to_string(&duplicate_envelope).unwrap(),
            )
            .await
            .unwrap();

        let worker = Worker::from_app(app).unwrap();
        assert!(worker.run_once().await.unwrap());
        assert!(worker.run_once().await.unwrap());
        assert!(worker.run_once().await.unwrap());
        assert!(!worker.run_once().await.unwrap());

        let log = read_log_filtered(&format!("{tag}:"));
        assert!(log.contains(&"primary".to_string()));
        assert!(log.contains(&"duplicate".to_string()));
        assert_eq!(
            log.iter().filter(|entry| *entry == "complete:done").count(),
            1
        );
    }

    #[tokio::test]
    async fn empty_batch_returns_error() {
        let (_backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("batch-empty");
        let result = dispatcher.batch("empty").dispatch().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn empty_chain_returns_error() {
        let (_backend, _runtime, _diagnostics, dispatcher) =
            build_runtime_and_dispatcher("chain-empty");
        let result = dispatcher.chain().dispatch().await;
        assert!(result.is_err());
    }
}
