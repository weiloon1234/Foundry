use std::any::Any;
use std::backtrace::Backtrace;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use axum::response::Response;
use serde_json::Value;

use crate::auth::Actor;
use crate::events::EventOrigin;
use crate::foundation::AppContext;
use crate::jobs::{JobDeadLetterContext, JobMiddleware};
use crate::support::redaction::{redact_sensitive_json, redact_sensitive_text};
use crate::support::sync::lock_unpoisoned;

use super::{
    catch_async_panic, current_execution, current_trace_context, scope_current_trace,
    CurrentRequest, ExecutionContext, TraceContext,
};

#[async_trait]
pub trait ErrorReporter: Send + Sync + 'static {
    async fn report_handler_error(&self, report: HandlerErrorReport);

    async fn report_panic(&self, report: PanicReport);

    async fn report_job_dead_lettered(&self, report: JobDeadLetteredReport);
}

#[derive(Clone, Debug)]
pub struct HandlerErrorReport {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub error: String,
    pub chain: Vec<String>,
    pub origin: Option<EventOrigin>,
    pub request_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum PanicContext {
    Http {
        request_id: Option<String>,
        method: String,
        path: String,
    },
    Job {
        id: String,
        class: String,
    },
    Scheduler {
        id: String,
    },
    Other,
}

#[derive(Clone, Debug)]
pub struct PanicReport {
    pub message: String,
    pub location: String,
    pub backtrace: Option<String>,
    pub context: PanicContext,
}

#[derive(Clone, Debug)]
pub struct JobDeadLetteredReport {
    pub job_class: String,
    pub job_id: String,
    pub attempts: u32,
    pub last_error: String,
    pub payload: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct HandlerErrorResponseExtension {
    status: u16,
    error: String,
    chain: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct ErrorReporterRegistry {
    reporters: Vec<Arc<dyn ErrorReporter>>,
    handler_min_status: u16,
}

impl ErrorReporterRegistry {
    pub(crate) fn new(reporters: Vec<Arc<dyn ErrorReporter>>) -> Self {
        Self {
            reporters,
            handler_min_status: 500,
        }
    }

    pub(crate) async fn report_handler_error(&self, report: HandlerErrorReport) {
        if report.status < self.handler_min_status {
            return;
        }
        let report = redact_handler_error_report(report);

        for (index, reporter) in self.reporters.iter().enumerate() {
            report_with_panic_boundary("handler_error", index, || {
                reporter.report_handler_error(report.clone())
            })
            .await;
        }
    }

    pub(crate) async fn report_panic(&self, report: PanicReport) {
        let report = redact_panic_report(report);
        for (index, reporter) in self.reporters.iter().enumerate() {
            report_with_panic_boundary("panic", index, || reporter.report_panic(report.clone()))
                .await;
        }
    }

    pub(crate) async fn report_job_dead_lettered(&self, report: JobDeadLetteredReport) {
        let report = redact_dead_letter_report(report);
        for (index, reporter) in self.reporters.iter().enumerate() {
            report_with_panic_boundary("job_dead_lettered", index, || {
                reporter.report_job_dead_lettered(report.clone())
            })
            .await;
        }
    }
}

fn redact_handler_error_report(mut report: HandlerErrorReport) -> HandlerErrorReport {
    report.error = redact_sensitive_text(&report.error);
    report.chain = report
        .chain
        .into_iter()
        .map(|entry| redact_sensitive_text(&entry))
        .collect();
    report
}

fn redact_panic_report(mut report: PanicReport) -> PanicReport {
    report.message = redact_sensitive_text(&report.message);
    report
}

fn redact_dead_letter_report(mut report: JobDeadLetteredReport) -> JobDeadLetteredReport {
    report.last_error = redact_sensitive_text(&report.last_error);
    redact_sensitive_json(&mut report.payload);
    report
}

async fn report_with_panic_boundary<F, Fut>(report: &'static str, reporter_index: usize, run: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let result = ERROR_REPORTER_DELIVERY
        .scope((), catch_async_panic(run))
        .await;
    if let Err(panic) = result {
        tracing::warn!(
            target: "foundry::logging::reporter",
            report = report,
            reporter_index = reporter_index,
            panic = %panic_payload_message(panic),
            "Error reporter panicked"
        );
    }
}

async fn deliver_with_trace_context<F, T>(trace_context: Option<TraceContext>, future: F) -> T
where
    F: Future<Output = T>,
{
    if let Some(trace_context) = trace_context {
        scope_current_trace(trace_context, future).await
    } else {
        future.await
    }
}

fn error_reporter_delivery_active() -> bool {
    ERROR_REPORTER_DELIVERY.try_with(|_| ()).is_ok()
}

tokio::task_local! {
    static ERROR_REPORTER_DELIVERY: ();
}

pub(crate) fn mark_handler_error_response(
    response: &mut Response,
    status: u16,
    error: String,
    chain: Vec<String>,
) {
    response
        .extensions_mut()
        .insert(HandlerErrorResponseExtension {
            status,
            error: redact_sensitive_text(&error),
            chain: chain
                .into_iter()
                .map(|entry| redact_sensitive_text(&entry))
                .collect(),
        });
}

pub(crate) async fn report_handler_error_response(
    app: &AppContext,
    method: &str,
    path: &str,
    request: &CurrentRequest,
    actor: Option<Actor>,
    extension: Option<HandlerErrorResponseExtension>,
) {
    let Some(extension) = extension else {
        return;
    };

    let trace_context = current_trace_context().or_else(|| {
        request
            .request_id
            .as_ref()
            .map(|request_id| TraceContext::http(request_id.clone()))
    });
    let trace_id = trace_context
        .as_ref()
        .map(|context| context.trace_id.clone());
    if extension.status >= 500 {
        tracing::error!(
            method = %method,
            path = %path,
            status = extension.status,
            request_id = ?request.request_id,
            trace_id = ?trace_id,
            error = %extension.error,
            chain = ?extension.chain,
            "Handler returned server error response"
        );
    } else {
        tracing::warn!(
            method = %method,
            path = %path,
            status = extension.status,
            request_id = ?request.request_id,
            trace_id = ?trace_id,
            error = %extension.error,
            chain = ?extension.chain,
            "Handler returned client error response"
        );
    }

    let Ok(registry) = app.resolve::<ErrorReporterRegistry>() else {
        return;
    };

    let origin = EventOrigin::from_request(actor, Some(request));
    let report = HandlerErrorReport {
        method: method.to_string(),
        path: path.to_string(),
        status: extension.status,
        error: extension.error,
        chain: extension.chain,
        origin,
        request_id: request.request_id.clone(),
    };

    deliver_with_trace_context(trace_context, registry.report_handler_error(report)).await;
}

pub(crate) async fn report_job_dead_lettered(app: &AppContext, report: JobDeadLetteredReport) {
    let Ok(registry) = app.resolve::<ErrorReporterRegistry>() else {
        return;
    };

    deliver_with_trace_context(
        current_trace_context(),
        registry.report_job_dead_lettered(report),
    )
    .await;
}

pub(crate) fn panic_payload_message(payload: Box<dyn Any + Send>) -> String {
    let message = if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    };
    redact_sensitive_text(&message)
}

pub(crate) fn set_global_panic_reporters(registry: Arc<ErrorReporterRegistry>) {
    let slot = GLOBAL_PANIC_REPORTERS.get_or_init(|| Mutex::new(None));
    *lock_unpoisoned(slot, "panic reporter registry") = Some(registry);
}

pub(crate) fn report_panic_from_hook(message: String, location: String) {
    if error_reporter_delivery_active() {
        tracing::warn!(
            target: "foundry::logging::reporter",
            "Skipping panic reporter delivery while already delivering an error report"
        );
        return;
    }

    let registry = GLOBAL_PANIC_REPORTERS
        .get()
        .and_then(|slot| slot.lock().ok().and_then(|guard| guard.clone()));
    let Some(registry) = registry else {
        return;
    };

    let report = PanicReport {
        message,
        location,
        backtrace: Some(Backtrace::force_capture().to_string()),
        context: current_execution()
            .map(panic_context_from_execution)
            .unwrap_or(PanicContext::Other),
    };
    let trace_context = current_trace_context();

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            deliver_with_trace_context(trace_context, registry.report_panic(report)).await;
        });
    }
}

fn panic_context_from_execution(context: ExecutionContext) -> PanicContext {
    match context {
        ExecutionContext::Http {
            request_id,
            method,
            path,
        } => PanicContext::Http {
            request_id,
            method,
            path,
        },
        ExecutionContext::Job { class, id } => PanicContext::Job { id, class },
        ExecutionContext::Scheduler { id } => PanicContext::Scheduler { id },
        ExecutionContext::Other => PanicContext::Other,
    }
}

static GLOBAL_PANIC_REPORTERS: OnceLock<Mutex<Option<Arc<ErrorReporterRegistry>>>> =
    OnceLock::new();

pub(crate) struct ErrorReporterJobMiddleware;

#[async_trait]
impl JobMiddleware for ErrorReporterJobMiddleware {
    async fn on_dead_lettered(
        &self,
        context: &JobDeadLetterContext,
    ) -> crate::foundation::Result<()> {
        let mut payload = context.payload.clone();
        redact_sensitive_json(&mut payload);
        report_job_dead_lettered(
            &context.app,
            JobDeadLetteredReport {
                job_class: context.class.clone(),
                job_id: context.id.clone(),
                attempts: context.attempts,
                last_error: redact_sensitive_text(&context.last_error),
                payload,
            },
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::auth::Actor;
    use crate::config::ConfigRepository;
    use crate::foundation::Container;
    use crate::support::GuardId;
    use crate::validation::RuleRegistry;

    #[derive(Default)]
    struct StubReporter {
        handler_reports: Mutex<Vec<HandlerErrorReport>>,
        panic_reports: Mutex<Vec<PanicReport>>,
        dead_letter_reports: Mutex<Vec<JobDeadLetteredReport>>,
        handler_trace_ids: Mutex<Vec<Option<String>>>,
        panic_trace_ids: Mutex<Vec<Option<String>>>,
        dead_letter_trace_ids: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl ErrorReporter for StubReporter {
        async fn report_handler_error(&self, report: HandlerErrorReport) {
            self.handler_trace_ids
                .lock()
                .unwrap()
                .push(crate::logging::current_trace_id());
            self.handler_reports.lock().unwrap().push(report);
        }

        async fn report_panic(&self, report: PanicReport) {
            self.panic_trace_ids
                .lock()
                .unwrap()
                .push(crate::logging::current_trace_id());
            self.panic_reports.lock().unwrap().push(report);
        }

        async fn report_job_dead_lettered(&self, report: JobDeadLetteredReport) {
            self.dead_letter_trace_ids
                .lock()
                .unwrap()
                .push(crate::logging::current_trace_id());
            self.dead_letter_reports.lock().unwrap().push(report);
        }
    }

    struct RecursivePanicReporter {
        panic_messages: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ErrorReporter for RecursivePanicReporter {
        async fn report_handler_error(&self, _report: HandlerErrorReport) {}

        async fn report_panic(&self, report: PanicReport) {
            self.panic_messages.lock().unwrap().push(report.message);
            report_panic_from_hook(
                "recursive reporter panic".to_string(),
                "src/tests.rs:2".to_string(),
            );
            panic!("recursive reporter explode")
        }

        async fn report_job_dead_lettered(&self, _report: JobDeadLetteredReport) {}
    }

    struct PanickingReporter;

    #[async_trait]
    impl ErrorReporter for PanickingReporter {
        async fn report_handler_error(&self, _report: HandlerErrorReport) {
            panic!("handler reporter explode")
        }

        async fn report_panic(&self, _report: PanicReport) {
            panic!("panic reporter explode")
        }

        async fn report_job_dead_lettered(&self, _report: JobDeadLetteredReport) {
            panic!("dead-letter reporter explode")
        }
    }

    struct FactoryPanickingReporter;

    impl ErrorReporter for FactoryPanickingReporter {
        fn report_handler_error<'life0, 'async_trait>(
            &'life0 self,
            _report: HandlerErrorReport,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            panic!("handler reporter factory explode")
        }

        fn report_panic<'life0, 'async_trait>(
            &'life0 self,
            _report: PanicReport,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            panic!("panic reporter factory explode")
        }

        fn report_job_dead_lettered<'life0, 'async_trait>(
            &'life0 self,
            _report: JobDeadLetteredReport,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            panic!("dead-letter reporter factory explode")
        }
    }

    fn test_app_with_stub_reporter(reporter: Arc<StubReporter>) -> AppContext {
        test_app_with_reporters(vec![reporter])
    }

    fn test_app_with_reporters(reporters: Vec<Arc<dyn ErrorReporter>>) -> AppContext {
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        let registry = Arc::new(ErrorReporterRegistry::new(reporters));
        app.container().singleton_arc(registry).unwrap();
        app
    }

    static GLOBAL_REPORTER_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    #[tokio::test]
    async fn reports_handler_errors_with_origin() {
        let reporter = Arc::new(StubReporter::default());
        let app = test_app_with_stub_reporter(reporter.clone());
        let request = CurrentRequest {
            request_id: Some("req-handler".to_string()),
            ip: Some("203.0.113.5".parse().unwrap()),
            user_agent: Some("FoundryReporter/1.0".to_string()),
            audit_area: None,
        };

        report_handler_error_response(
            &app,
            "GET",
            "/boom",
            &request,
            Some(Actor::new("admin-1", GuardId::new("admin"))),
            Some(HandlerErrorResponseExtension {
                status: 500,
                error: "boom".to_string(),
                chain: vec!["cause".to_string()],
            }),
        )
        .await;

        let reports = reporter.handler_reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, 500);
        assert_eq!(reports[0].request_id.as_deref(), Some("req-handler"));
        assert_eq!(
            reporter.handler_trace_ids.lock().unwrap().as_slice(),
            &[Some("req-handler".to_string())]
        );
        assert_eq!(
            reports[0]
                .origin
                .as_ref()
                .and_then(|origin| origin.actor.as_ref())
                .map(|actor| actor.id.as_str()),
            Some("admin-1")
        );
    }

    #[tokio::test]
    async fn reports_panics_using_execution_context() {
        let _guard = GLOBAL_REPORTER_TEST_LOCK.lock().await;
        let reporter = Arc::new(StubReporter::default());
        let registry = Arc::new(ErrorReporterRegistry::new(vec![reporter.clone()]));
        set_global_panic_reporters(registry);

        crate::logging::scope_current_trace(
            crate::logging::TraceContext::new("trace-panic"),
            crate::logging::scope_current_execution(
                ExecutionContext::Job {
                    class: "email.send".to_string(),
                    id: "job-1".to_string(),
                },
                async {
                    report_panic_from_hook("oops".to_string(), "src/tests.rs:1".to_string());
                },
            ),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let reports = reporter.panic_reports.lock().unwrap();
        let report = reports
            .iter()
            .find(|report| report.message == "oops" && report.location == "src/tests.rs:1")
            .expect("expected scoped panic report");
        match &report.context {
            PanicContext::Job { id, class } => {
                assert_eq!(id, "job-1");
                assert_eq!(class, "email.send");
            }
            other => panic!("unexpected panic context: {other:?}"),
        }
        assert!(reporter
            .panic_trace_ids
            .lock()
            .unwrap()
            .contains(&Some("trace-panic".to_string())));
    }

    #[tokio::test]
    async fn panic_reporter_recursion_is_skipped_during_reporter_delivery() {
        let _guard = GLOBAL_REPORTER_TEST_LOCK.lock().await;
        let recursive_messages = Arc::new(Mutex::new(Vec::new()));
        let later_reporter = Arc::new(StubReporter::default());
        let registry = Arc::new(ErrorReporterRegistry::new(vec![
            Arc::new(RecursivePanicReporter {
                panic_messages: recursive_messages.clone(),
            }) as Arc<dyn ErrorReporter>,
            later_reporter.clone() as Arc<dyn ErrorReporter>,
        ]));

        registry
            .report_panic(PanicReport {
                message: "outer panic".to_string(),
                location: "src/tests.rs:1".to_string(),
                backtrace: None,
                context: PanicContext::Other,
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        assert_eq!(
            recursive_messages.lock().unwrap().as_slice(),
            &["outer panic"]
        );
        let reports = later_reporter.panic_reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].message, "outer panic");
    }

    #[tokio::test]
    async fn reports_dead_lettered_jobs() {
        let reporter = Arc::new(StubReporter::default());
        let app = test_app_with_stub_reporter(reporter.clone());

        report_job_dead_lettered(
            &app,
            JobDeadLetteredReport {
                job_class: "email.send".to_string(),
                job_id: "job-1".to_string(),
                attempts: 3,
                last_error: "boom".to_string(),
                payload: serde_json::json!({ "email": "hello@example.com" }),
            },
        )
        .await;

        let reports = reporter.dead_letter_reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].job_class, "email.send");
        assert_eq!(reports[0].job_id, "job-1");
    }

    #[tokio::test]
    async fn reporter_delivery_redacts_sensitive_error_text_and_payloads() {
        let reporter = Arc::new(StubReporter::default());
        let registry = ErrorReporterRegistry::new(vec![reporter.clone() as Arc<dyn ErrorReporter>]);

        registry
            .report_handler_error(HandlerErrorReport {
                method: "POST".to_string(),
                path: "/login".to_string(),
                status: 500,
                error: "database postgres://user:secret@example.test/app token=abc".to_string(),
                chain: vec!["Authorization: Bearer abc.def".to_string()],
                origin: None,
                request_id: Some("req-redact".to_string()),
            })
            .await;
        registry
            .report_panic(PanicReport {
                message: "panic with password=\"pw\"".to_string(),
                location: "src/tests.rs:1".to_string(),
                backtrace: None,
                context: PanicContext::Other,
            })
            .await;
        registry
            .report_job_dead_lettered(JobDeadLetteredReport {
                job_class: "email.send".to_string(),
                job_id: "job-1".to_string(),
                attempts: 3,
                last_error: "failed api_key=secret".to_string(),
                payload: serde_json::json!({
                    "token": "abc",
                    "nested": { "password": "pw", "safe": "visible" }
                }),
            })
            .await;

        let handler_reports = reporter.handler_reports.lock().unwrap();
        assert_eq!(
            handler_reports[0].error,
            "database postgres://[redacted]@example.test/app token=[redacted]"
        );
        assert_eq!(
            handler_reports[0].chain[0],
            "Authorization: Bearer [redacted]"
        );

        let panic_reports = reporter.panic_reports.lock().unwrap();
        assert_eq!(
            panic_reports[0].message,
            "panic with password=\"[redacted]\""
        );

        let dead_letter_reports = reporter.dead_letter_reports.lock().unwrap();
        assert_eq!(
            dead_letter_reports[0].last_error,
            "failed api_key=[redacted]"
        );
        assert_eq!(dead_letter_reports[0].payload["token"], "[redacted]");
        assert_eq!(
            dead_letter_reports[0].payload["nested"]["password"],
            "[redacted]"
        );
        assert_eq!(dead_letter_reports[0].payload["nested"]["safe"], "visible");
    }

    #[tokio::test]
    async fn reporter_panics_do_not_block_later_reporters() {
        let reporter = Arc::new(StubReporter::default());
        let registry = ErrorReporterRegistry::new(vec![
            Arc::new(PanickingReporter) as Arc<dyn ErrorReporter>,
            reporter.clone() as Arc<dyn ErrorReporter>,
        ]);

        registry
            .report_handler_error(HandlerErrorReport {
                method: "GET".to_string(),
                path: "/boom".to_string(),
                status: 500,
                error: "boom".to_string(),
                chain: Vec::new(),
                origin: None,
                request_id: Some("req-reporter-panic".to_string()),
            })
            .await;
        registry
            .report_panic(PanicReport {
                message: "panic".to_string(),
                location: "src/tests.rs:1".to_string(),
                backtrace: None,
                context: PanicContext::Other,
            })
            .await;
        registry
            .report_job_dead_lettered(JobDeadLetteredReport {
                job_class: "email.send".to_string(),
                job_id: "job-1".to_string(),
                attempts: 3,
                last_error: "boom".to_string(),
                payload: serde_json::json!({ "email": "hello@example.com" }),
            })
            .await;

        assert_eq!(reporter.handler_reports.lock().unwrap().len(), 1);
        assert_eq!(reporter.panic_reports.lock().unwrap().len(), 1);
        assert_eq!(reporter.dead_letter_reports.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reporter_factory_panics_do_not_block_later_reporters() {
        let reporter = Arc::new(StubReporter::default());
        let registry = ErrorReporterRegistry::new(vec![
            Arc::new(FactoryPanickingReporter) as Arc<dyn ErrorReporter>,
            reporter.clone() as Arc<dyn ErrorReporter>,
        ]);

        registry
            .report_handler_error(HandlerErrorReport {
                method: "GET".to_string(),
                path: "/boom".to_string(),
                status: 500,
                error: "boom".to_string(),
                chain: Vec::new(),
                origin: None,
                request_id: Some("req-reporter-factory-panic".to_string()),
            })
            .await;
        registry
            .report_panic(PanicReport {
                message: "panic".to_string(),
                location: "src/tests.rs:1".to_string(),
                backtrace: None,
                context: PanicContext::Other,
            })
            .await;
        registry
            .report_job_dead_lettered(JobDeadLetteredReport {
                job_class: "email.send".to_string(),
                job_id: "job-1".to_string(),
                attempts: 3,
                last_error: "boom".to_string(),
                payload: serde_json::json!({ "email": "hello@example.com" }),
            })
            .await;

        assert_eq!(reporter.handler_reports.lock().unwrap().len(), 1);
        assert_eq!(reporter.panic_reports.lock().unwrap().len(), 1);
        assert_eq!(reporter.dead_letter_reports.lock().unwrap().len(), 1);
    }
}
