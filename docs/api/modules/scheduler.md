# scheduler

Cron + interval scheduling with Redis-safe leadership

[Back to index](../index.md)

## foundry::scheduler

```rust
pub type ScheduleRegistrar = Arc<dyn Fn(&mut ScheduleRegistry) -> Result<()> + Send + Sync>;
enum ScheduleKind { Cron, Interval }
struct CronExpression
  fn parse(value: impl Into<String>) -> Result<Self>
  fn as_str(&self) -> &str
  fn every_minute() -> Result<Self>
  fn every_five_minutes() -> Result<Self>
  fn every_ten_minutes() -> Result<Self>
  fn every_fifteen_minutes() -> Result<Self>
  fn every_thirty_minutes() -> Result<Self>
  fn hourly() -> Result<Self>
  fn daily() -> Result<Self>
  fn daily_at(time: &str) -> Result<Self>
  fn weekly() -> Result<Self>
  fn monthly() -> Result<Self>
struct ScheduleInvocation
  fn app(&self) -> &AppContext
struct ScheduleOptions
  fn new() -> Self
  fn without_overlapping(self) -> Self
  fn without_overlapping_for(self, ttl: Duration) -> Self
  fn environments(self, envs: &[&str]) -> Self
  fn before<F, Fut>(self, hook: F) -> Self
  fn after<F, Fut>(self, hook: F) -> Self
  fn on_failure<F, Fut>(self, hook: F) -> Self
struct ScheduleRegistry
  fn new() -> Self
  fn cron<I, F, Fut>( &mut self, id: I, expression: CronExpression, job: F, ) -> Result<&mut Self>
  fn cron_with_options<I, F, Fut>( &mut self, id: I, expression: CronExpression, options: ScheduleOptions, job: F, ) -> Result<&mut Self>
  fn interval<I, F, Fut>( &mut self, id: I, every: Duration, job: F, ) -> Result<&mut Self>
  fn interval_with_options<I, F, Fut>( &mut self, id: I, every: Duration, options: ScheduleOptions, job: F, ) -> Result<&mut Self>
  fn every_minute<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
  fn every_five_minutes<I, F, Fut>( &mut self, id: I, job: F, ) -> Result<&mut Self>
  fn hourly<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
  fn daily<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
  fn daily_at<I, F, Fut>( &mut self, id: I, time: &str, job: F, ) -> Result<&mut Self>
  fn weekly<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
struct ScheduledTask
```

## Notes

- Schedule handler panics are handled as schedule failures and route through `ScheduleOptions::on_failure`.
- Scheduler hooks are isolated: hook panics are logged and do not crash the scheduler task.
- `SchedulerConfig.shutdown_timeout_ms` defaults to `30000`; `0` aborts active schedules immediately on shutdown.
