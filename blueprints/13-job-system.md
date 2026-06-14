# Rust Job System Blueprint (Framework-Level)

## Overview

This document defines the full design of the **background job system** in Foundry — covering what's built, what's missing, and phased improvements to bring it to production-grade.

Goal:

> Provide a type-safe, configurable, Redis-backed job queue with concurrent workers, per-job timeouts, lifecycle hooks, and advanced dispatch patterns (batching, chaining, rate limiting) — with a DX as clean as Laravel Horizon.

---

# Current State (What's Built)

**Status: Core complete — production-ready for simple use cases**

### Job Trait

```rust
#[async_trait]
pub trait Job: Serialize + DeserializeOwned + Send + Sync + Debug + 'static {
    const ID: JobId;
    const QUEUE: Option<QueueId> = None;

    async fn handle(&self, context: JobContext) -> Result<()>;

    fn max_retries(&self) -> Option<u32> { None }
    fn backoff(&self, attempt: u32) -> Duration {
        match attempt {
            1 => Duration::from_secs(5),
            2 => Duration::from_secs(30),
            3 => Duration::from_secs(60),
            4 => Duration::from_secs(300),
            _ => Duration::from_secs(600),
        }
    }
}
```

### Dispatch

```rust
app.jobs()?.dispatch(MyJob { ... }).await?;           // immediate
app.jobs()?.dispatch_later(MyJob { ... }, ts).await?;  // scheduled
```

### Infrastructure

| Feature | Status |
|---------|--------|
| Job trait with typed ID | ✅ |
| JobDispatcher (dispatch/dispatch_later) | ✅ |
| Redis backend (Lua scripts for atomicity) | ✅ |
| Memory backend (dev/testing) | ✅ |
| Retry with exponential backoff | ✅ |
| Dead letter queue | ✅ |
| Lease-based locking with heartbeat | ✅ |
| Worker kernel | ✅ |
| Job registration via ServiceProvider | ✅ |
| Diagnostics/metrics recording | ✅ |
| Configurable poll interval, lease TTL, max retries | ✅ |

### Framework-Provided Jobs

| Job | Module | Purpose |
|-----|--------|---------|
| `SendQueuedEmailJob` | `src/email/job.rs` | Async email delivery |
| `DatatableExportJob` | `src/datatable/export_job.rs` | XLSX generation |
| `SendNotificationJob` | `src/notifications/job.rs` | Queued notification dispatch |

### What's Missing

| Feature | Priority | Impact |
|---------|----------|--------|
| Worker concurrency | **Critical** | Can only process 1 job at a time |
| Per-job timeout | **Critical** | Jobs can hang indefinitely |
| Graceful shutdown for worker | **High** | Worker can't drain in-flight jobs |
| Job middleware (before/after/failed hooks) | **High** | No lifecycle extensibility |
| Per-job timeout configuration | **High** | Global only, not per-job |
| Job batching | **Medium** | Can't dispatch group with completion callback |
| Job chaining | **Medium** | Can't sequence A → B → C |
| Rate limiting per job type | **Medium** | No throttling |
| Unique jobs | **Medium** | No duplicate prevention |
| Job status tracking | **Low** | No query-able job state |
| Dashboard/monitoring | **Low** | No web UI |

---

# Phase 1: Critical — Worker Concurrency + Timeout + Shutdown

## 1.1 Worker Concurrency

**Current:** Worker processes 1 job at a time in a loop.
**Target:** Configurable concurrent workers (e.g., 4 tasks processing simultaneously).

### Config

```toml
[jobs]
workers = 4              # max concurrent jobs (semaphore limit)
poll_interval_ms = 100
lease_ttl_ms = 30000
max_retries = 5
timeout_seconds = 300    # per-job default timeout
```

### Worker Architecture

Spawn-per-job with semaphore-bounded concurrency:
- Single claim loop runs maintenance (promote scheduled, requeue expired)
- On each claimed job, acquires a semaphore permit, then `tokio::spawn`s the job
- When the job completes (success/retry/dead-letter), the permit is released
- `workers` config = semaphore limit (max concurrent jobs, NOT fixed thread count)
- On shutdown: stops claiming, waits for all in-flight permits to return, then exits
- For IO-heavy workloads (AI API calls), increase `workers` to 50+ — tokio handles thousands of suspended tasks

### Internal Design

Replace the single `Worker::run()` loop with N spawned tasks:

```rust
pub async fn run(self) -> Result<()> {
    let mut handles = Vec::with_capacity(self.config.workers);
    for worker_id in 0..self.config.workers {
        let worker = self.clone();
        handles.push(tokio::spawn(async move {
            worker.run_single(worker_id).await
        }));
    }
    // Wait for shutdown signal, then cancel all tasks
    shutdown_signal().await;
    for handle in handles {
        handle.abort();
    }
    Ok(())
}
```

Each task independently claims and processes jobs. The Redis Lua `CLAIM_JOB_SCRIPT` is already atomic — concurrent claims are safe.

### Consumer DX

No change — just config:

```toml
[jobs]
workers = 4
```

### Files

- `src/jobs/mod.rs` — modify `Worker::run()` to spawn N tasks
- `src/config/mod.rs` — add `workers` field to `JobsConfig`
- `src/kernel/worker.rs` — integrate shutdown signal

---

## 1.2 Per-Job Timeout

**Current:** No timeout — jobs can run forever.
**Target:** Configurable timeout per job type, with a global default.

### Config

```toml
[jobs]
timeout_seconds = 300    # global default: 5 minutes
```

### Job Trait Addition

```rust
pub trait Job: ... {
    // ... existing methods ...

    /// Maximum execution time for this job. Override for long-running jobs.
    fn timeout(&self) -> Duration {
        Duration::from_secs(300)  // default: 5 minutes
    }
}
```

### Internal Design

In the worker's job execution path, wrap `handle()` with `tokio::time::timeout`:

```rust
let timeout_duration = job.timeout();
match tokio::time::timeout(timeout_duration, job.handle(context)).await {
    Ok(Ok(())) => { /* success — ACK */ }
    Ok(Err(error)) => { /* job error — retry/dead letter */ }
    Err(_elapsed) => { /* timeout — treat as failure, retry/dead letter */ }
}
```

### Consumer DX

```rust
impl Job for LongRunningExport {
    const ID: JobId = JobId::new("long_export");

    async fn handle(&self, ctx: JobContext) -> Result<()> { ... }

    fn timeout(&self) -> Duration {
        Duration::from_secs(1800)  // 30 minutes for this specific job
    }
}
```

### Files

- `src/jobs/mod.rs` — add `timeout()` to Job trait, wrap handle() with tokio::time::timeout
- `src/config/mod.rs` — add `timeout_seconds` to JobsConfig

---

## 1.3 Worker Graceful Shutdown

**Current:** Worker loops forever with no signal handling.
**Target:** SIGTERM/SIGINT triggers graceful drain — finish current jobs, stop claiming new ones.

### Internal Design

Already have `kernel/shutdown.rs` with `shutdown_signal()`. Use `tokio::select!` in the worker loop:

```rust
async fn run_single(&self, worker_id: usize) {
    loop {
        tokio::select! {
            _ = shutdown_signal() => {
                tracing::info!(worker_id, "worker shutting down gracefully");
                break;
            }
            _ = self.run_once() => {}
        }
    }
}
```

The current in-flight job completes (it's inside `run_once`), then the loop breaks. No job is abandoned mid-execution.

### Files

- `src/jobs/mod.rs` — add shutdown awareness to worker loop
- `src/kernel/worker.rs` — already thin, may need minor update

---

# Phase 1.5: High — Transactional Dispatch (after_commit)

**Status: Done**

## Problem

Dispatching jobs inside a database transaction causes two bugs:
1. **Job runs before commit** — worker queries DB, data doesn't exist yet
2. **Commit fails after dispatch** — job is queued but data was rolled back

```rust
// ❌ BUG: job dispatched before commit, or commit fails after dispatch
let txn = app.begin_transaction().await?;
let order = Order::create().set(...).save(&txn).await?;
app.jobs()?.dispatch(SendConfirmationJob { order_id }).await?;  // dispatched NOW
txn.commit().await?;  // what if this fails?
```

## Solution

Buffer dispatches on `AppTransaction`, flush after successful commit:

```rust
let txn = app.begin_transaction().await?;
let order = Order::create().set(...).save(&txn).await?;

// Buffered — NOT dispatched yet
txn.dispatch_after_commit(SendConfirmationJob { order_id })?;
txn.notify_after_commit(&user, &OrderCreated { order_id })?;

txn.commit().await?;
// ✅ Commit succeeded → jobs + notifications dispatched
// ✅ Commit failed → nothing dispatched
```

## Internal Design

Add a buffer to `AppTransaction`:

```rust
pub struct AppTransaction {
    app: AppContext,
    transaction: DatabaseTransaction,
    pending_dispatches: Vec<PendingDispatch>,
}

enum PendingDispatch {
    Job(JobEnvelope),       // serialized, ready to push to queue
    Notification(SendNotificationJob),  // pre-rendered notification
}
```

On `commit()`:
1. Commit the DB transaction
2. If commit succeeds, dispatch all pending items
3. If commit fails, drop pending items (never dispatched)

On `rollback()` or `Drop`:
- Clear pending items without dispatching

## Consumer DX

```rust
impl AppTransaction {
    pub fn dispatch_after_commit<J: Job>(&mut self, job: J) -> Result<()>;
    pub fn notify_after_commit(
        &mut self,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
    ) -> Result<()>;
}
```

## Workaround (No Framework Support Needed)

For now, dispatch after commit manually:

```rust
let txn = app.begin_transaction().await?;
let order = Order::create().set(...).save(&txn).await?;
txn.commit().await?;

// Safe — data is committed
app.jobs()?.dispatch(SendConfirmationJob { order_id }).await?;
```

## Files

- `src/foundation/app.rs` — add buffer to `AppTransaction`, modify `commit()`

---

# Phase 2: High — Job Middleware + Events

## 2.1 Job Middleware

Hooks that run before/after/on-failure for any job. Used for logging, metrics, cleanup, rate limiting.

### Design

```rust
#[async_trait]
pub trait JobMiddleware: Send + Sync + 'static {
    /// Called before job execution. Return Err to prevent execution.
    async fn before(&self, _job_id: &JobId, _context: &JobContext) -> Result<()> {
        Ok(())
    }

    /// Called after successful job execution.
    async fn after(&self, _job_id: &JobId, _context: &JobContext) -> Result<()> {
        Ok(())
    }

    /// Called when job execution fails (before retry/dead-letter decision).
    async fn failed(
        &self,
        _job_id: &JobId,
        _context: &JobContext,
        _error: &crate::foundation::Error,
    ) -> Result<()> {
        Ok(())
    }
}
```

### Registration

```rust
// In ServiceProvider::register():
registrar.register_job_middleware(LoggingJobMiddleware)?;
registrar.register_job_middleware(MetricsJobMiddleware)?;
```

### Consumer DX

```rust
struct LoggingJobMiddleware;

#[async_trait]
impl JobMiddleware for LoggingJobMiddleware {
    async fn before(&self, job_id: &JobId, context: &JobContext) -> Result<()> {
        tracing::info!(job = %job_id, attempt = context.attempt(), "job starting");
        Ok(())
    }

    async fn after(&self, job_id: &JobId, _context: &JobContext) -> Result<()> {
        tracing::info!(job = %job_id, "job completed");
        Ok(())
    }

    async fn failed(&self, job_id: &JobId, _context: &JobContext, error: &Error) -> Result<()> {
        tracing::error!(job = %job_id, error = %error, "job failed");
        Ok(())
    }
}
```

### Files

- `src/jobs/mod.rs` — add `JobMiddleware` trait, middleware registry, hook calls in worker execution

---

# Phase 3: Medium — Advanced Dispatch Patterns

## 3.1 Job Batching

Dispatch a group of jobs and get a callback when all complete.

### Design

```rust
let batch = app.jobs()?.batch("export-batch")
    .add(ExportChunkJob { chunk: 1 })
    .add(ExportChunkJob { chunk: 2 })
    .add(ExportChunkJob { chunk: 3 })
    .on_complete(MergeExportsJob { batch_id })
    .dispatch().await?;
```

### Internal

- `batches` table (or Redis hash): tracks batch_id → { total, completed, failed }
- On job ACK, increment completed counter
- When completed == total, dispatch the `on_complete` job

---

## 3.2 Job Chaining

Sequential execution: A finishes → B dispatches → C dispatches.

### Design

```rust
app.jobs()?.chain()
    .add(ValidateOrderJob { order_id })
    .add(ChargePaymentJob { order_id })
    .add(SendConfirmationJob { order_id })
    .dispatch().await?;
```

### Internal

- Store the chain as a list of serialized job payloads in the first job's metadata
- On success, pop the next job from the chain and dispatch it
- On failure, the chain stops (remaining jobs are not dispatched)

---

## 3.3 Rate Limiting Per Job Type

Throttle specific job types to prevent overwhelming external APIs.

### Design

```rust
impl Job for SendSmsJob {
    const ID: JobId = JobId::new("send_sms");

    fn rate_limit(&self) -> Option<RateLimit> {
        Some(RateLimit::per_minute(60))  // max 60 SMS jobs per minute
    }

    async fn handle(&self, ctx: JobContext) -> Result<()> { ... }
}
```

### Internal

- Before execution, check Redis counter: `job_rate:{job_id}:{minute_bucket}`
- If over limit, requeue with delay (not retry — this is intentional throttle)

---

## 3.4 Unique Jobs

Prevent duplicate jobs within a time window.

### Design

```rust
impl Job for SyncInventoryJob {
    const ID: JobId = JobId::new("sync_inventory");

    fn unique_for(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))  // only 1 instance per 5 minutes
    }

    fn unique_key(&self) -> Option<String> {
        Some(format!("merchant:{}", self.merchant_id))
    }

    async fn handle(&self, ctx: JobContext) -> Result<()> { ... }
}
```

### Internal

- On dispatch, check Redis: `job_unique:{job_id}:{unique_key}`
- If exists and not expired, skip dispatch (return Ok without enqueuing)
- On successful dispatch, set the key with TTL

---

# Phase 4: Low — Visibility + Monitoring

## 4.1 Job Status Tracking

Track job lifecycle in a database table for queryability.

### Schema

```sql
CREATE TABLE job_history (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    job_id TEXT NOT NULL,
    queue TEXT NOT NULL,
    status TEXT NOT NULL,          -- 'pending', 'running', 'completed', 'failed', 'dead_lettered'
    payload JSONB,
    attempt INT NOT NULL DEFAULT 1,
    error TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    duration_ms BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### DX

```rust
// Query job history
let recent = app.jobs()?.history()
    .job_id("send_sms")
    .status("failed")
    .limit(10)
    .get(&app).await?;
```

---

## 4.2 Dashboard Endpoint

JSON API endpoint for job monitoring (can be consumed by a frontend dashboard).

```
GET /_foundry/jobs/stats    → { queues, pending, running, failed, dead_lettered }
GET /_foundry/jobs/failed   → [{ id, job_id, error, failed_at }]
POST /_foundry/jobs/retry   → retry specific dead-lettered jobs
```

---

# Implementation Order

| Phase | Features | Priority | Status |
|-------|----------|----------|--------|
| 1 | Concurrency + Timeout + Shutdown | Critical | ✅ Done |
| 1.5 | Transactional Dispatch (after_commit) | High | ✅ Done |
| 2 | Job Middleware | High | ✅ Done |
| 3 | Batching + Chaining + Rate Limit + Unique | Medium | ✅ Done |
| 4 | Status Tracking + Dashboard | Low | ✅ Done |

---

# Assumptions

- Redis backend is the primary production path; memory backend is for dev/testing
- Lua scripts for atomicity are the right approach (no external dependencies)
- Job serialization uses serde_json (already in place)
- Worker concurrency uses tokio tasks, not OS threads
- Per-job timeout wraps `handle()` only — setup/teardown is not timed
- Graceful shutdown waits for in-flight jobs to complete, does not forcefully abort
- Job middleware runs in registration order (first registered = first to execute)
- Batch/chain metadata stored in Redis alongside job payloads (not in a separate DB)
- Dashboard is JSON API only — no HTML UI (consumed by SPA or monitoring tools)

---

# One-Line Goal

> A Foundry job should be dispatchable with one line, configurable per-job (timeout, retries, rate limit, uniqueness), observable (middleware hooks + status tracking), and scalable (concurrent workers, batching, chaining) — all backed by Redis with memory fallback for testing.
