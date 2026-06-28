# Background Processing Guide

Jobs, scheduled tasks, and domain events — the async processing triad. Events trigger jobs, schedules dispatch jobs, jobs fire events.

---

## Jobs

### Defining a Job

```rust
#[derive(Debug, Serialize, Deserialize)]
struct SendWelcomeEmail {
    user_id: String,
    email: String,
}

const WELCOME_EMAIL_JOB: JobId = JobId::new("send_welcome_email");

#[async_trait]
impl Job for SendWelcomeEmail {
    const ID: JobId = WELCOME_EMAIL_JOB;

    async fn handle(&self, ctx: JobContext) -> Result<()> {
        let email_manager = ctx.app().email()?;
        email_manager.send(
            EmailMessage::new("Welcome!")
                .to(&self.email)
                .text_body("Thanks for signing up.")
        ).await
    }
}
```

### Registering Jobs

Jobs must be registered before they can be dispatched:

```rust
// In ServiceProvider
registrar.register_job::<SendWelcomeEmail>()?;

// Or in Plugin
registrar.register_job::<SendWelcomeEmail>();
```

### Dispatching

```rust
let jobs = app.jobs()?;

// Dispatch immediately
jobs.dispatch(SendWelcomeEmail {
    user_id: "123".into(),
    email: "alice@example.com".into(),
})?;

// Dispatch with delay
let run_at = DateTime::now().add_days(1).timestamp_millis();
jobs.dispatch_later(SendWelcomeEmail { /* ... */ }, run_at)?;
```

### Job Options

Override defaults per-job by implementing optional trait methods:

```rust
#[async_trait]
impl Job for ProcessVideo {
    const ID: JobId = JobId::new("process_video");
    const QUEUE: Option<QueueId> = Some(QueueId::new("heavy"));  // dedicated queue

    async fn handle(&self, ctx: JobContext) -> Result<()> { /* ... */ }

    fn max_retries(&self) -> Option<u32> {
        Some(2)  // retry twice on failure
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(600))  // 10 minute timeout
    }

    fn backoff(&self, attempt: u32) -> Duration {
        // Custom retry delays
        match attempt {
            1 => Duration::from_secs(10),
            2 => Duration::from_secs(60),
            _ => Duration::from_secs(300),
        }
    }

    fn rate_limit(&self) -> Option<(u32, Duration)> {
        Some((5, Duration::from_secs(60)))  // max 5 per minute
    }

    fn unique_for(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))  // deduplicate within 5 minutes
    }

    fn unique_key(&self) -> Option<String> {
        Some(format!("video:{}", self.video_id))  // custom dedup key
    }
}
```

### Batches (Concurrent)

Run multiple jobs concurrently with an optional completion callback:

```rust
let batch_id = app.jobs()?.batch("onboard-user")
    .add(SendWelcomeEmail { user_id: id.clone(), email: email.clone() })?
    .add(CreateDefaultSettings { user_id: id.clone() })?
    .add(SyncToAnalytics { user_id: id.clone() })?
    .on_complete(NotifyAdmin { message: format!("User {id} onboarded") })?
    .dispatch()
    .await?;
```

All jobs in a batch run concurrently. The `on_complete` job fires only after ALL batch jobs succeed.

### Chains (Sequential)

Run jobs one after another — next job starts only when previous succeeds:

```rust
app.jobs()?.chain()
    .add(DownloadFile { url: url.clone() })?
    .add(ProcessFile { path: path.clone() })?
    .add(UploadResult { path: output.clone() })?
    .add(NotifyComplete { user_id: user_id.clone() })?
    .dispatch()
    .await?;
```

If any job in the chain fails, subsequent jobs are NOT dispatched.

### Transactional Dispatch

Dispatch jobs only after a database transaction commits:

```rust
let mut tx = app.begin_transaction().await?;

// ... create order in transaction ...

tx.dispatch_after_commit(SendOrderConfirmation {
    order_id: order.id.to_string(),
});

tx.commit().await?;
// Job is dispatched only if commit succeeds
```

### Job Middleware

Hook into the job lifecycle for cross-cutting concerns:

```rust
struct LogJobExecution;

#[async_trait]
impl JobMiddleware for LogJobExecution {
    async fn before(&self, job_id: &JobId, ctx: &JobContext) -> Result<()> {
        tracing::info!(job = %job_id, attempt = ctx.attempt(), "job starting");
        Ok(())
    }

    async fn after(&self, job_id: &JobId, _ctx: &JobContext) -> Result<()> {
        tracing::info!(job = %job_id, "job completed");
        Ok(())
    }

    async fn failed(&self, job_id: &JobId, _ctx: &JobContext, error: &str) -> Result<()> {
        tracing::error!(job = %job_id, error, "job failed");
        Ok(())
    }

    async fn on_dead_lettered(&self, ctx: &JobDeadLetterContext) -> Result<()> {
        tracing::error!(
            job = %ctx.class,
            id = %ctx.id,
            attempts = ctx.attempts,
            error = %ctx.last_error,
            "job dead-lettered"
        );
        Ok(())
    }
}

// Register
registrar.register_job_middleware(LogJobExecution)?;
```

`on_dead_lettered()` runs once after retries are exhausted and the job has been moved to the
dead-letter queue. Middleware hooks are called in registration order, and hook failures are logged
without aborting the dead-letter transition.

Job failures include returned errors, timeouts, and panics from `Job::handle`. A job panic is
recorded as `job panicked: ...` and counts as one failed attempt, so the normal backoff,
retry, and dead-letter rules still apply. Middleware hook errors or panics are logged and do not
block success, retry, or dead-letter finalization.

### Error Reporters

Use `ErrorReporter` when you want one integration point for handler 500s, panics, and dead-lettered
jobs:

```rust
#[derive(Default)]
struct OpsReporter;

#[async_trait]
impl ErrorReporter for OpsReporter {
    async fn report_handler_error(&self, report: HandlerErrorReport) {
        tracing::error!(path = %report.path, status = report.status, error = %report.error);
    }

    async fn report_panic(&self, report: PanicReport) {
        tracing::error!(location = %report.location, message = %report.message);
    }

    async fn report_job_dead_lettered(&self, report: JobDeadLetteredReport) {
        tracing::error!(job = %report.job_class, id = %report.job_id, error = %report.last_error);
    }
}

App::builder()
    .register_error_reporter::<OpsReporter>();
```

Reporters are code-registered, not config-driven. Foundry fans out to every registered reporter.

### Running the Worker

```rust
// As a separate process
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .run_worker()?;
```

Or use the worker kernel for testing:

```rust
let kernel = app_builder.build_worker_kernel().await?;
kernel.run_once().await?;  // process one job
```

On shutdown, workers stop claiming new jobs and drain active jobs for
`jobs.shutdown_timeout_ms`. If the timeout elapses, or the value is `0`, remaining active jobs are
aborted. Foundry does not ack, retry, or dead-letter those shutdown-aborted jobs directly; their lease
expires and the existing requeue flow makes them runnable again for another worker or a restarted
service.

Workers started with `spawn_worker(app)` inside another kernel are owned by the app lifecycle. Foundry
still applies `app.background_shutdown_timeout_ms` as the outer cap for the managed worker task.
Process-manager hard kills such as SIGKILL cannot be caught, so unacked jobs recover through the
same lease-expiry path.

### Config

```toml
# config/app.toml
[app]
background_shutdown_timeout_ms = 30000  # 0 = abort managed background tasks immediately

# config/40-runtime.toml
[runtime]
worker_threads = 0        # 0 = Tokio default for Foundry-owned sync runners
max_blocking_threads = 0  # 0 = Tokio default blocking pool cap

[jobs]
queue = "default"              # default queue name
max_retries = 5                # global total attempt limit (1 = no retry)
poll_interval_ms = 100         # how often to check for jobs
lease_ttl_ms = 30000           # lease duration before requeue
max_concurrent_jobs = 0        # 0 = unlimited
timeout_seconds = 300          # global job timeout
shutdown_timeout_ms = 30000    # active job drain timeout on shutdown (0 = abort immediately)
track_history = true           # write to job_history table
history_retention_days = 30    # auto-prune job_history older than N days (0 = keep forever)
history_prune_interval_ms = 3600000
history_prune_batch_size = 1000
```

`job_history` is the persistent observability store behind `/_foundry/jobs/stats` and
`/_foundry/jobs/failed`. Workers own pruning internally, coordinated by a distributed lock, so
consumer apps do not need to register a scheduler task. Runtime job counters remain process-local
and reset on restart.

---

## Scheduler

### Registering Scheduled Tasks

```rust
fn schedules(registry: &mut ScheduleRegistry) -> Result<()> {
    // Cron expression
    registry.cron(
        ScheduleId::new("daily_report"),
        CronExpression::daily_at("09:00")?,
        |inv| async move {
            let db = inv.app().database()?;
            // generate report...
            Ok(())
        },
    )?;

    // Convenience methods
    registry.every_minute(ScheduleId::new("heartbeat"), |_| async { Ok(()) })?;
    registry.every_five_minutes(ScheduleId::new("sync"), |_| async { Ok(()) })?;
    registry.hourly(ScheduleId::new("cleanup"), |_| async { Ok(()) })?;
    registry.daily(ScheduleId::new("backup"), |_| async { Ok(()) })?;
    registry.weekly(ScheduleId::new("digest"), |_| async { Ok(()) })?;

    // Fixed interval
    registry.interval(
        ScheduleId::new("health_ping"),
        Duration::from_secs(30),
        |inv| async move {
            // ping health endpoint
            Ok(())
        },
    )?;

    Ok(())
}
```

Register in bootstrap:

```rust
App::builder()
    .register_schedule(schedules)
    .run_scheduler()?;
```

### Cron Expressions

```rust
CronExpression::parse("0 9 * * *")?       // 9:00 AM daily
CronExpression::parse("*/5 * * * *")?      // every 5 minutes
CronExpression::parse("0 0 * * 1")?        // midnight every Monday
CronExpression::parse("0 0 1 * *")?        // midnight on 1st of month

// Convenience constructors
CronExpression::every_minute()?
CronExpression::every_five_minutes()?
CronExpression::every_ten_minutes()?
CronExpression::every_fifteen_minutes()?
CronExpression::every_thirty_minutes()?
CronExpression::hourly()?
CronExpression::daily()?
CronExpression::daily_at("15:30")?         // 3:30 PM daily
CronExpression::weekly()?
CronExpression::monthly()?
```

### Schedule Options

```rust
registry.cron_with_options(
    ScheduleId::new("heavy_job"),
    CronExpression::hourly()?,
    ScheduleOptions::new()
        .without_overlapping()                // skip if previous run still active
        .environments(&["production"])         // only run in production
        .before(|app| async move {
            tracing::info!("starting heavy job");
            Ok(())
        })
        .after(|app| async move {
            tracing::info!("heavy job completed");
            Ok(())
        })
        .on_failure(|app| async move {
            // alert on failure
            Ok(())
        }),
    |inv| async move {
        // heavy job logic
        Ok(())
    },
)?;
```

### Dispatching Jobs from Schedules

Schedules can dispatch background jobs instead of running work inline:

```rust
registry.daily(ScheduleId::new("nightly_export"), |inv| async move {
    inv.app().jobs()?.dispatch(GenerateNightlyExport {
        date: Date::today().to_string(),
    })?;
    Ok(())
})?;
```

### Distributed Safety

When running multiple scheduler instances (e.g., Kubernetes replicas), only ONE acquires leadership and executes tasks. The others stay idle. Leadership is managed via Redis with configurable lease TTL.

```toml
# config/scheduler.toml
[scheduler]
tick_interval_ms = 1000        # check for due tasks every second
leader_lease_ttl_ms = 5000     # leadership lease duration
shutdown_timeout_ms = 30000    # active schedule drain timeout on shutdown (0 = abort immediately)
```

Schedule handler errors and panics are treated as schedule failures and run the configured
`on_failure` hook. `before`, `after`, and `on_failure` hook errors or panics are logged without
crashing the scheduler task, and `without_overlapping` locks are released even when a handler fails
or panics.

### Running the Scheduler

```rust
App::builder()
    .register_schedule(schedules)
    .run_scheduler()?;
```

---

## Events

### Defining Events

```rust
#[derive(Clone, Serialize)]
struct OrderPlaced {
    order_id: String,
    customer_id: String,
    total: f64,
}

impl Event for OrderPlaced {
    const ID: EventId = EventId::new("order.placed");
}
```

### Defining Listeners

```rust
struct SendOrderConfirmationListener;

#[async_trait]
impl EventListener<OrderPlaced> for SendOrderConfirmationListener {
    async fn handle(&self, ctx: &EventContext, event: &OrderPlaced) -> Result<()> {
        let email = ctx.app().email()?;
        email.send(EmailMessage::new("Order Confirmation")
            .to(&event.customer_id)  // resolve email from customer_id
            .text_body(format!("Order {} placed. Total: ${:.2}", event.order_id, event.total))
        ).await
    }
}

struct UpdateInventoryListener;

#[async_trait]
impl EventListener<OrderPlaced> for UpdateInventoryListener {
    async fn handle(&self, ctx: &EventContext, event: &OrderPlaced) -> Result<()> {
        // deduct inventory...
        Ok(())
    }
}
```

### Registering Listeners

```rust
// In ServiceProvider
registrar.listen_event::<OrderPlaced, _>(SendOrderConfirmationListener)?;
registrar.listen_event::<OrderPlaced, _>(UpdateInventoryListener)?;

// Or in Plugin
registrar.listen_event::<OrderPlaced, _>(SendOrderConfirmationListener);
```

Multiple listeners can handle the same event. They run in registration order.

### Dispatching Events

```rust
let events = app.events()?;
events.dispatch(OrderPlaced {
    order_id: "ORD-123".into(),
    customer_id: "CUST-456".into(),
    total: 99.99,
}).await?;
```

### Event Origin Metadata

`EventContext` now carries optional request and actor origin metadata. Model lifecycle events fired
from HTTP writes automatically include the authenticated actor, IP, user-agent, and request ID.

```rust
struct RecordOrigin;

#[async_trait]
impl EventListener<ModelUpdatedEvent> for RecordOrigin {
    async fn handle(&self, ctx: &EventContext, event: &ModelUpdatedEvent) -> Result<()> {
        tracing::info!(
            table = %event.table,
            actor_id = ctx.actor().map(|actor| actor.id.as_str()).unwrap_or("system"),
            request_id = ?ctx.request_id(),
            ip = ?ctx.ip(),
            user_agent = ?ctx.user_agent(),
            "model updated"
        );
        Ok(())
    }
}
```

Scheduled tasks, CLI commands, and background jobs default to `None` for origin metadata unless you
explicitly pass an origin with `EventBus::dispatch_with_origin(...)`.

### Event → Job (Async Processing)

Instead of handling work inline in the listener, dispatch a job:

```rust
// Register using the helper
registrar.listen_event::<OrderPlaced, _>(
    dispatch_job(|event: &OrderPlaced| SendOrderConfirmationJob {
        order_id: event.order_id.clone(),
    })
)?;
```

This is the recommended pattern for work that's slow or can fail — the event listener returns immediately, and the job handles retry/backoff.

Built-in model post-write events (`ModelCreatedEvent`, `ModelUpdatedEvent`, and
`ModelDeletedEvent`) are already post-commit events. A listener can safely dispatch jobs or create
dependent rows from these events without racing the original transaction. The matching pre-write
events (`ModelCreatingEvent`, `ModelUpdatingEvent`, and `ModelDeletingEvent`) still run before
commit and can reject the write.

### Event → WebSocket (Real-Time)

Broadcast events to connected WebSocket clients:

```rust
registrar.listen_event::<OrderPlaced, _>(
    publish_websocket(|event: &OrderPlaced| ServerMessage {
        channel: ChannelId::new("orders"),
        event: ChannelEventId::new("new_order"),
        room: None,
        payload: json!({ "order_id": event.order_id, "total": event.total }),
    })
)?;
```

### Events in Transactions

Fire events only after successful commit:

```rust
let mut tx = app.begin_transaction().await?;

// ... create order in transaction ...

// This notification uses events internally
tx.notify_after_commit(&customer, &OrderPlacedNotification {
    order_id: order.id.to_string(),
});

tx.commit().await?;
// Events/notifications fire only after commit succeeds
```

---

## How They Connect

The three systems work together:

```
User Action
    │
    ├─ Event dispatched ──→ Listener handles immediately
    │                        └─ OR dispatches a Job for async processing
    │
    ├─ Job dispatched ──→ Worker picks up and executes
    │                      └─ Job can dispatch more events
    │
    └─ Schedule fires ──→ Handler runs on cron/interval
                           └─ Can dispatch jobs or events
```

### Example: Order Processing Pipeline

```rust
// 1. Event: order placed
events.dispatch(OrderPlaced { order_id, customer_id, total }).await?;

// 2. Listeners (registered in ServiceProvider):
//    - SendOrderConfirmationListener → dispatches SendEmailJob
//    - UpdateInventoryListener → runs inline
//    - publish_websocket → broadcasts to "orders" channel

// 3. Job: SendEmailJob runs in worker process
//    - Sends email via email driver
//    - On failure: retries with exponential backoff

// 4. Schedule: daily at midnight
//    - Dispatches GenerateDailySalesReport job
//    - Report job queries orders, generates XLSX, emails to finance
```

### Example: Registration

```rust
// ServiceProvider
registrar.listen_event::<OrderPlaced, _>(SendOrderConfirmationListener)?;
registrar.listen_event::<OrderPlaced, _>(UpdateInventoryListener)?;
registrar.listen_event::<OrderPlaced, _>(
    publish_websocket(|e: &OrderPlaced| ServerMessage { /* ... */ })
)?;

registrar.register_job::<SendOrderConfirmationJob>()?;
registrar.register_job::<GenerateDailySalesReport>()?;
```

```rust
// Bootstrap
App::builder()
    .register_provider(AppServiceProvider)
    .register_schedule(|s| {
        s.daily(ScheduleId::new("daily_report"), |inv| async move {
            inv.app().jobs()?.dispatch(GenerateDailySalesReport {
                date: Date::today().to_string(),
            })?;
            Ok(())
        })
    })
    .run_http()?;   // or run_worker()? or run_scheduler()?
```
