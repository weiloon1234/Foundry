# Scheduler Production Hardening

> **Status:** ✅ Complete
> **Created:** 2026-04-13
> **Purpose:** Fix critical scheduler bugs and add per-task options for production use.

---

# Critical Fixes

## 1. Error isolation — one failing task must not kill the scheduler

**Current:** `(task.handler)(app).await?` — the `?` propagates errors, aborting the tick and stopping all subsequent tasks.

**Fix:** Catch errors per task, log them, continue to the next task.

## 2. Parallel execution — tasks should not block each other

**Current:** Tasks run sequentially in the tick loop. A 10-second task blocks all others.

**Fix:** `tokio::spawn` each due task.

## 3. Overlap prevention

**Current:** None. A slow task can overlap with its next invocation.

**Fix:** Per-task `without_overlapping` flag. Uses a Redis/memory lock keyed by `schedule:{id}`.

---

# Per-Task Options (ScheduleOptions builder)

```rust
registry.cron_with_options(
    "report:daily",
    CronExpression::parse("0 0 2 * * *")?,
    ScheduleOptions::new()
        .without_overlapping()
        .environments(&["production"])
        .before(|app| async { tracing::info!("starting report"); Ok(()) })
        .after(|app| async { tracing::info!("report done"); Ok(()) }),
    |inv| async { generate_report(inv.app()).await },
)?;
```

## Convenience cron helpers

```rust
registry.every_minute("ping", handler)?;
registry.every_five_minutes("sync", handler)?;
registry.hourly("cleanup", handler)?;
registry.daily("report", handler)?;
registry.daily_at("backup", "03:00", handler)?;
registry.weekly("digest", handler)?;
```

---

# Implementation

All changes in `src/scheduler/mod.rs` and `src/kernel/scheduler.rs`.
