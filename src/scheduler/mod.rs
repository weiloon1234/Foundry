mod leadership;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime as ChronoDateTime, TimeZone as ChronoTimeZone};
use cron::Schedule as CronSchedule;
use serde::{Deserialize, Serialize};

use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_sync_panic, panic_payload_message};
use crate::support::{boxed, BoxFuture};
use crate::support::{DateTime, ScheduleId, Timezone};

pub type ScheduleRegistrar = Arc<dyn Fn(&mut ScheduleRegistry) -> Result<()> + Send + Sync>;
pub(crate) type ScheduleHandler = Arc<dyn Fn(AppContext) -> BoxFuture<Result<()>> + Send + Sync>;
pub(crate) type ScheduleHook = Arc<dyn Fn(AppContext) -> BoxFuture<Result<()>> + Send + Sync>;

const DEFAULT_OVERLAP_LOCK_TTL: Duration = Duration::from_secs(3_600);

pub(crate) fn build_registry(registrars: &[ScheduleRegistrar]) -> Result<ScheduleRegistry> {
    let mut registry = ScheduleRegistry::new();
    for registrar in registrars {
        match catch_sync_panic(|| registrar(&mut registry)) {
            Ok(result) => result?,
            Err(panic) => return Err(schedule_registrar_panic_error(panic)),
        }
    }
    Ok(registry)
}

fn schedule_registrar_panic_error(panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.scheduler",
        panic = %message,
        "Schedule registrar panicked"
    );
    Error::message(format!("schedule registrar panicked: {message}"))
}

#[derive(Clone)]
pub struct ScheduleInvocation {
    app: AppContext,
}

impl ScheduleInvocation {
    pub(crate) fn new(app: AppContext) -> Self {
        Self { app }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }
}

#[derive(Clone)]
pub struct CronExpression {
    source: String,
    schedule: CronSchedule,
}

impl CronExpression {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let source = value.into();
        let schedule = source.parse::<CronSchedule>().map_err(Error::other)?;
        Ok(Self { source, schedule })
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }

    pub(crate) fn schedule(&self) -> &CronSchedule {
        &self.schedule
    }

    // Convenience constructors
    pub fn every_minute() -> Result<Self> {
        Self::parse("0 * * * * *")
    }

    pub fn every_five_minutes() -> Result<Self> {
        Self::parse("0 */5 * * * *")
    }

    pub fn every_ten_minutes() -> Result<Self> {
        Self::parse("0 */10 * * * *")
    }

    pub fn every_fifteen_minutes() -> Result<Self> {
        Self::parse("0 */15 * * * *")
    }

    pub fn every_thirty_minutes() -> Result<Self> {
        Self::parse("0 */30 * * * *")
    }

    pub fn hourly() -> Result<Self> {
        Self::parse("0 0 * * * *")
    }

    pub fn daily() -> Result<Self> {
        Self::parse("0 0 0 * * *")
    }

    /// Daily at a specific time (HH:MM format).
    ///
    /// Clock-driven scheduler execution interprets this wall time in the configured
    /// application timezone.
    pub fn daily_at(time: &str) -> Result<Self> {
        let parts: Vec<&str> = time.split(':').collect();
        if parts.len() != 2 {
            return Err(Error::message(format!(
                "invalid time format '{time}', expected HH:MM"
            )));
        }
        let hour: u32 = parts[0]
            .parse()
            .map_err(|_| Error::message("invalid hour"))?;
        let minute: u32 = parts[1]
            .parse()
            .map_err(|_| Error::message("invalid minute"))?;
        if hour > 23 {
            return Err(Error::message(format!(
                "invalid hour {hour}, expected 0-23"
            )));
        }
        if minute > 59 {
            return Err(Error::message(format!(
                "invalid minute {minute}, expected 0-59"
            )));
        }
        Self::parse(format!("0 {minute} {hour} * * *"))
    }

    pub fn weekly() -> Result<Self> {
        Self::parse("0 0 0 * * 1")
    }

    pub fn monthly() -> Result<Self> {
        Self::parse("0 0 0 1 * *")
    }
}

impl Serialize for CronExpression {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CronExpression {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let source = String::deserialize(deserializer)?;
        Self::parse(source).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone)]
pub enum ScheduleKind {
    Cron { expression: Box<CronExpression> },
    Interval { every: Duration },
}

/// Per-task configuration options.
#[derive(Clone)]
pub struct ScheduleOptions {
    pub(crate) without_overlapping: bool,
    pub(crate) overlap_lock_ttl: Duration,
    pub(crate) environments: Vec<String>,
    pub(crate) before_hook: Option<ScheduleHook>,
    pub(crate) after_hook: Option<ScheduleHook>,
    pub(crate) on_failure: Option<ScheduleHook>,
}

impl Default for ScheduleOptions {
    fn default() -> Self {
        Self {
            without_overlapping: false,
            overlap_lock_ttl: DEFAULT_OVERLAP_LOCK_TTL,
            environments: Vec::new(),
            before_hook: None,
            after_hook: None,
            on_failure: None,
        }
    }
}

impl ScheduleOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prevent this task from running if the previous invocation hasn't finished.
    ///
    /// Uses an owner-token distributed lock keyed by the schedule ID. The default
    /// one-hour lease is renewed while the task is running. If the lock backend is
    /// unavailable, the task is skipped instead of running without protection.
    pub fn without_overlapping(mut self) -> Self {
        self.without_overlapping = true;
        self
    }

    /// Prevent overlapping with a custom lock lease duration.
    ///
    /// The lease is renewed while the task is running. Durations below one second
    /// are normalized to one second to match the distributed lock backend.
    pub fn without_overlapping_for(mut self, ttl: Duration) -> Self {
        self.without_overlapping = true;
        self.overlap_lock_ttl = Duration::from_secs(ttl.as_secs().max(1));
        self
    }

    /// Only run this task in the specified environments.
    pub fn environments(mut self, envs: &[&str]) -> Self {
        self.environments = envs.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Hook that runs before the task executes.
    pub fn before<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(AppContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.before_hook = Some(Arc::new(move |app| boxed(hook(app))));
        self
    }

    /// Hook that runs after the task completes successfully.
    pub fn after<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(AppContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.after_hook = Some(Arc::new(move |app| boxed(hook(app))));
        self
    }

    /// Hook that runs when the task fails.
    pub fn on_failure<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(AppContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.on_failure = Some(Arc::new(move |app| boxed(hook(app))));
        self
    }
}

#[derive(Clone)]
pub struct ScheduledTask {
    pub(crate) id: ScheduleId,
    pub(crate) kind: ScheduleKind,
    pub(crate) handler: ScheduleHandler,
    pub(crate) options: ScheduleOptions,
}

#[derive(Default)]
pub struct ScheduleRegistry {
    tasks: Vec<ScheduledTask>,
}

impl ScheduleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cron<I, F, Fut>(
        &mut self,
        id: I,
        expression: CronExpression,
        job: F,
    ) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.cron_with_options(id, expression, ScheduleOptions::default(), job)
    }

    pub fn cron_with_options<I, F, Fut>(
        &mut self,
        id: I,
        expression: CronExpression,
        options: ScheduleOptions,
        job: F,
    ) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let id = id.into();
        ensure_unique_name(&self.tasks, &id)?;

        self.tasks.push(ScheduledTask {
            id,
            kind: ScheduleKind::Cron {
                expression: Box::new(expression),
            },
            handler: Arc::new(move |app| boxed(job(ScheduleInvocation::new(app)))),
            options,
        });
        Ok(self)
    }

    pub fn interval<I, F, Fut>(&mut self, id: I, every: Duration, job: F) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.interval_with_options(id, every, ScheduleOptions::default(), job)
    }

    pub fn interval_with_options<I, F, Fut>(
        &mut self,
        id: I,
        every: Duration,
        options: ScheduleOptions,
        job: F,
    ) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let id = id.into();
        ensure_unique_name(&self.tasks, &id)?;

        self.tasks.push(ScheduledTask {
            id,
            kind: ScheduleKind::Interval { every },
            handler: Arc::new(move |app| boxed(job(ScheduleInvocation::new(app)))),
            options,
        });
        Ok(self)
    }

    // Convenience methods

    pub fn every_minute<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.cron(id, CronExpression::every_minute()?, job)
    }

    pub fn every_five_minutes<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.cron(id, CronExpression::every_five_minutes()?, job)
    }

    pub fn hourly<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.cron(id, CronExpression::hourly()?, job)
    }

    pub fn daily<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.cron(id, CronExpression::daily()?, job)
    }

    pub fn daily_at<I, F, Fut>(&mut self, id: I, time: &str, job: F) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.cron(id, CronExpression::daily_at(time)?, job)
    }

    pub fn weekly<I, F, Fut>(&mut self, id: I, job: F) -> Result<&mut Self>
    where
        I: Into<ScheduleId>,
        F: Fn(ScheduleInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.cron(id, CronExpression::weekly()?, job)
    }

    pub(crate) fn tasks(self) -> Vec<ScheduledTask> {
        self.tasks
    }
}

fn ensure_unique_name(tasks: &[ScheduledTask], id: &ScheduleId) -> Result<()> {
    if tasks.iter().any(|task| &task.id == id) {
        return Err(Error::message(format!(
            "schedule `{id}` already registered"
        )));
    }
    Ok(())
}

pub(crate) fn cron_due_in_timezone(
    schedule: &CronExpression,
    previous: DateTime,
    now: DateTime,
    timezone: &Timezone,
) -> bool {
    match timezone {
        Timezone::Utc => cron_due_with_timezone(schedule, previous.as_chrono(), now.as_chrono()),
        Timezone::Iana(timezone) => cron_due_with_timezone(
            schedule,
            previous.as_chrono().with_timezone(timezone),
            now.as_chrono().with_timezone(timezone),
        ),
        Timezone::FixedOffset(timezone) => cron_due_with_timezone(
            schedule,
            previous.as_chrono().with_timezone(timezone),
            now.as_chrono().with_timezone(timezone),
        ),
    }
}

fn cron_due_with_timezone<Tz>(
    schedule: &CronExpression,
    previous: ChronoDateTime<Tz>,
    now: ChronoDateTime<Tz>,
) -> bool
where
    Tz: ChronoTimeZone,
{
    schedule
        .schedule()
        .after(&(previous - chrono::Duration::nanoseconds(1)))
        .next()
        .is_some_and(|next| next <= now)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        cron_due_in_timezone, CronExpression, ScheduleOptions, ScheduleRegistrar, ScheduleRegistry,
    };
    use crate::support::{DateTime, ScheduleId, Timezone};

    fn schedule_time(value: &str) -> DateTime {
        DateTime::parse(value).unwrap()
    }

    #[test]
    fn cron_due_interprets_fields_in_the_requested_timezone() {
        let schedule = CronExpression::daily_at("08:00").unwrap();
        let timezone = Timezone::parse("Asia/Kuala_Lumpur").unwrap();

        assert!(cron_due_in_timezone(
            &schedule,
            schedule_time("2026-04-07T23:59:59Z"),
            schedule_time("2026-04-08T00:00:00Z"),
            &timezone,
        ));
        assert!(!cron_due_in_timezone(
            &schedule,
            schedule_time("2026-04-07T23:59:59Z"),
            schedule_time("2026-04-08T00:00:00Z"),
            &Timezone::Utc,
        ));
    }

    #[test]
    fn cron_due_skips_nonexistent_and_ambiguous_local_instants() {
        let timezone = Timezone::parse("America/New_York").unwrap();
        let nonexistent = CronExpression::daily_at("02:30").unwrap();
        let ambiguous = CronExpression::daily_at("01:30").unwrap();

        assert!(!cron_due_in_timezone(
            &nonexistent,
            schedule_time("2026-03-08T06:59:59Z"),
            schedule_time("2026-03-08T07:30:00Z"),
            &timezone,
        ));
        assert!(!cron_due_in_timezone(
            &ambiguous,
            schedule_time("2026-11-01T05:29:59Z"),
            schedule_time("2026-11-01T05:30:00Z"),
            &timezone,
        ));
        assert!(!cron_due_in_timezone(
            &ambiguous,
            schedule_time("2026-11-01T06:29:59Z"),
            schedule_time("2026-11-01T06:30:00Z"),
            &timezone,
        ));
    }

    #[test]
    fn rejects_duplicate_schedule_names() {
        let mut registry = ScheduleRegistry::new();
        registry
            .interval(
                ScheduleId::new("heartbeat"),
                Duration::from_secs(5),
                |_invocation| async { Ok(()) },
            )
            .unwrap();

        let error = registry
            .interval(
                ScheduleId::new("heartbeat"),
                Duration::from_secs(5),
                |_invocation| async { Ok(()) },
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn parses_cron_expressions_before_registration() {
        let expression = CronExpression::parse("*/5 * * * * *").unwrap();
        assert_eq!(expression.as_str(), "*/5 * * * * *");
    }

    #[test]
    fn schedule_registrar_panic_becomes_error() {
        let registrars: Vec<ScheduleRegistrar> = vec![Arc::new(|_| {
            panic!("schedule registrar explode");
        })];

        let error = match super::build_registry(&registrars) {
            Ok(_) => panic!("expected schedule registrar panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "schedule registrar panicked: schedule registrar explode"
        );
    }

    #[test]
    fn convenience_cron_constructors_work() {
        assert!(CronExpression::every_minute().is_ok());
        assert!(CronExpression::every_five_minutes().is_ok());
        assert!(CronExpression::hourly().is_ok());
        assert!(CronExpression::daily().is_ok());
        assert!(CronExpression::daily_at("03:00").is_ok());
        assert!(CronExpression::weekly().is_ok());
        assert!(CronExpression::monthly().is_ok());
    }

    #[test]
    fn daily_at_rejects_invalid_format() {
        assert!(CronExpression::daily_at("3pm").is_err());
        assert!(CronExpression::daily_at("25:00").is_err());
    }

    #[test]
    fn convenience_registry_methods_work() {
        let mut registry = ScheduleRegistry::new();
        registry
            .every_minute(ScheduleId::new("a"), |_| async { Ok(()) })
            .unwrap();
        registry
            .every_five_minutes(ScheduleId::new("b"), |_| async { Ok(()) })
            .unwrap();
        registry
            .hourly(ScheduleId::new("c"), |_| async { Ok(()) })
            .unwrap();
        registry
            .daily(ScheduleId::new("d"), |_| async { Ok(()) })
            .unwrap();
        registry
            .daily_at(ScheduleId::new("e"), "14:30", |_| async { Ok(()) })
            .unwrap();
        registry
            .weekly(ScheduleId::new("f"), |_| async { Ok(()) })
            .unwrap();
        assert_eq!(registry.tasks.len(), 6);
    }

    #[test]
    fn schedule_options_builder() {
        let opts = ScheduleOptions::new()
            .without_overlapping_for(Duration::from_secs(30))
            .environments(&["production", "staging"]);
        assert!(opts.without_overlapping);
        assert_eq!(opts.overlap_lock_ttl, Duration::from_secs(30));
        assert_eq!(opts.environments.len(), 2);
    }

    #[test]
    fn overlap_lock_ttl_is_at_least_one_second() {
        let opts = ScheduleOptions::new().without_overlapping_for(Duration::ZERO);

        assert!(opts.without_overlapping);
        assert_eq!(opts.overlap_lock_ttl, Duration::from_secs(1));
    }
}
