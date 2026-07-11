use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::config::{ConfigRepository, LoggingConfig, ObservabilityConfig};
use crate::foundation::{Error, Result};
use crate::support::sync::lock_unpoisoned;
use crate::support::{Clock, Timezone};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    #[default]
    Json,
    Text,
}

mod context;
mod diagnostics;
mod file_writer;
mod metrics;
mod middleware;
mod observability;
mod panic_boundary;
mod probes;
mod reporter;
mod request_id;
mod types;

pub(crate) use diagnostics::{HttpRequestRecord, RuntimeDiagnosticsConfig};
pub use diagnostics::{RuntimeDiagnostics, RuntimeSnapshot};
pub use file_writer::LogWriterRuntimeSnapshot;
pub use observability::ObservabilityOptions;
pub use probes::{
    LivenessReport, ProbeResult, ReadinessCheck, ReadinessReport, FRAMEWORK_BOOTSTRAP_PROBE,
    REDIS_PING_PROBE, RUNTIME_BACKEND_PROBE,
};
pub(crate) use probes::{ReadinessRegistryBuilder, ReadinessRegistryHandle};
pub use reporter::{
    ErrorReporter, HandlerErrorReport, JobDeadLetteredReport, PanicContext, PanicReport,
};
pub use request_id::{RequestId, RequestIdError, REQUEST_ID_HEADER, REQUEST_ID_MAX_LENGTH};
pub use types::{
    AuthOutcome, HttpOutcomeClass, JobOutcome, LogLevel, ProbeState, RuntimeBackendKind,
    SchedulerLeadershipState, WebSocketConnectionState,
};

pub(crate) use context::{
    current_actor, current_execution, current_execution_trace_parent, current_request,
    current_trace_context, scope_current_actor, scope_current_execution, scope_current_request,
    scope_current_trace, trace_context_for_child, ExecutionContext, TraceContext, TraceParent,
};
pub use context::{current_trace_id, CurrentRequest};
pub(crate) use middleware::{request_context_middleware, request_origin_middleware};
pub(crate) use observability::register_observability_routes;
pub(crate) use observability::{register_openapi_route, set_openapi_spec};
pub(crate) use panic_boundary::{catch_async_panic, catch_future_panic, catch_sync_panic};
pub(crate) use reporter::{
    mark_handler_error_response, panic_payload_message, report_handler_error_response,
    set_global_panic_reporters, ErrorReporterJobMiddleware, ErrorReporterRegistry,
};

/// Timer that formats timestamps using the framework's configured timezone.
struct FoundryTimer {
    timezone: Timezone,
}

impl FoundryTimer {
    fn new(timezone: Timezone) -> Self {
        Self { timezone }
    }
}

impl FormatTime for FoundryTimer {
    fn format_time(&self, writer: &mut Writer<'_>) -> std::fmt::Result {
        let clock = Clock::new(self.timezone.clone());
        let now = clock.now();
        write!(writer, "{}", now.format_in(&self.timezone))
    }
}

pub fn init(config: &ConfigRepository) -> Result<()> {
    static LOGGING: OnceLock<Mutex<bool>> = OnceLock::new();
    let initialized = LOGGING.get_or_init(|| Mutex::new(false));
    let mut initialized = lock_unpoisoned(initialized, "logging initialization");

    if *initialized {
        return Ok(());
    }

    let logging_config = config.logging()?;
    let observability_config = config.observability()?;
    let timezone = config.app()?.timezone;
    let level = logging_config.level.as_filter_directive();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    match logging_config.format {
        LogFormat::Json => init_json(filter, &logging_config, &timezone, &observability_config)?,
        LogFormat::Text => init_text(filter, &observability_config)?,
    }

    // Panic hook — capture panics as structured error events
    std::panic::set_hook(Box::new(|panic_info| {
        let location = panic_info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let message = panic_info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        let message = crate::support::redaction::redact_sensitive_text(&message);
        tracing::error!(
            target: "foundry.panic",
            location = %location,
            error = %message,
            "Thread panicked"
        );
        reporter::report_panic_from_hook(message, location);
    }));

    *initialized = true;
    Ok(())
}

fn init_json(
    filter: EnvFilter,
    logging_config: &LoggingConfig,
    timezone: &Timezone,
    _otel_config: &ObservabilityConfig,
) -> Result<()> {
    let timer = FoundryTimer::new(timezone.clone());
    let clock = Clock::new(timezone.clone());

    if logging_config.log_dir.is_empty() {
        // stdout only
        let stdout_layer = tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_target(true)
            .with_timer(timer);

        let registry = tracing_subscriber::registry()
            .with(filter)
            .with(stdout_layer);

        #[cfg(feature = "otel")]
        let registry = registry.with(build_otel_layer(_otel_config)?);

        finish_subscriber_install(registry.try_init())?;
    } else {
        if logging_config.file_flush_timeout_ms == 0 {
            return Err(Error::message(
                "logging file_flush_timeout_ms must be greater than zero",
            ));
        }
        let (file_writer, file_writer_controller) = file_writer::BoundedFileWriter::open(
            &logging_config.log_dir,
            &clock,
            logging_config.retention_days,
            logging_config.file_queue_capacity,
            logging_config.file_max_record_bytes,
        )
        .map_err(|error| {
            Error::message(format!(
                "failed to open log dir '{}': {error}",
                logging_config.log_dir
            ))
        })?;

        let stdout_layer = tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_target(true)
            .with_timer(FoundryTimer::new(timezone.clone()));

        let file_layer = tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_target(true)
            .with_timer(timer)
            .with_writer(file_writer);

        let registry = tracing_subscriber::registry()
            .with(filter)
            .with(stdout_layer)
            .with(file_layer);

        #[cfg(feature = "otel")]
        let registry = registry.with(build_otel_layer(_otel_config)?);

        if let Err(error) = finish_subscriber_install(registry.try_init()) {
            let _ = file_writer_controller.shutdown(Duration::from_secs(1));
            return Err(error);
        }
        file_writer_controller.install_global().map_err(|error| {
            Error::message(format!("failed to install log file writer: {error}"))
        })?;
    }
    Ok(())
}

fn init_text(filter: EnvFilter, _otel_config: &ObservabilityConfig) -> Result<()> {
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);

    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);

    #[cfg(feature = "otel")]
    let registry = registry.with(build_otel_layer(_otel_config)?);

    finish_subscriber_install(registry.try_init())
}

fn finish_subscriber_install<E>(result: std::result::Result<(), E>) -> Result<()>
where
    E: std::fmt::Display,
{
    result.map_err(|error| {
        Error::message(format!(
            "failed to install Foundry tracing subscriber; use App::builder().use_external_tracing_subscriber() when the host owns tracing: {error}"
        ))
    })
}

pub(crate) async fn flush_file_writer(timeout: Duration) -> Result<()> {
    file_writer::flush_global(timeout).await
}

/// Builds the OpenTelemetry tracing layer. Called only when the `otel` feature is enabled.
/// Returns `None` when `tracing_enabled` is `false`, which makes the layer a transparent no-op.
#[cfg(feature = "otel")]
fn build_otel_layer<S>(
    config: &ObservabilityConfig,
) -> Result<Option<tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig as _;

    if !config.tracing_enabled {
        return Ok(None);
    }

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.otlp_endpoint)
        .build()
        .map_err(|error| Error::message(format!("failed to build OTLP span exporter: {error}")))?;

    let tracer_provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(opentelemetry_sdk::Resource::new(vec![
            opentelemetry::KeyValue::new("service.name", config.service_name.clone()),
        ]))
        .build();

    let tracer = tracer_provider.tracer(config.service_name.clone());
    opentelemetry::global::set_tracer_provider(tracer_provider);

    Ok(Some(tracing_opentelemetry::layer().with_tracer(tracer)))
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::{
        finish_subscriber_install, ProbeResult, ProbeState, ReadinessCheck,
        ReadinessRegistryBuilder, RuntimeBackendKind, RuntimeDiagnostics,
    };
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container, Error};
    use crate::support::ProbeId;
    use crate::validation::RuleRegistry;

    struct PassingProbe;

    #[test]
    fn subscriber_install_failure_is_returned_with_hosted_runtime_guidance() {
        let error =
            finish_subscriber_install::<&str>(Err("global subscriber already set")).unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to install Foundry tracing subscriber"));
        assert!(error
            .to_string()
            .contains("use_external_tracing_subscriber"));
        assert!(error.to_string().contains("global subscriber already set"));
    }

    #[async_trait]
    impl ReadinessCheck for PassingProbe {
        async fn run(&self, _app: &AppContext) -> crate::Result<ProbeResult> {
            Ok(ProbeResult::healthy(ProbeId::new("provider.pass")))
        }
    }

    struct FailingProbe;

    #[async_trait]
    impl ReadinessCheck for FailingProbe {
        async fn run(&self, _app: &AppContext) -> crate::Result<ProbeResult> {
            Err(Error::message("not ready"))
        }
    }

    struct PanickingProbe;

    #[async_trait]
    impl ReadinessCheck for PanickingProbe {
        async fn run(&self, _app: &AppContext) -> crate::Result<ProbeResult> {
            panic!("probe boom")
        }
    }

    struct FactoryPanickingProbe;

    impl ReadinessCheck for FactoryPanickingProbe {
        fn run<'life0, 'life1, 'async_trait>(
            &'life0 self,
            _app: &'life1 AppContext,
        ) -> Pin<Box<dyn Future<Output = crate::Result<ProbeResult>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            panic!("probe factory boom")
        }
    }

    #[test]
    fn rejects_duplicate_probe_registration() {
        let mut builder = ReadinessRegistryBuilder::default();
        builder
            .register_arc(ProbeId::new("database"), Arc::new(PassingProbe))
            .unwrap();
        let error = builder
            .register_arc(ProbeId::new("database"), Arc::new(PassingProbe))
            .unwrap_err();

        assert!(error.to_string().contains("already registered"));
    }

    #[tokio::test]
    async fn readiness_aggregation_reports_failures() {
        let mut builder = ReadinessRegistryBuilder::default();
        builder
            .register_arc(ProbeId::new("provider.pass"), Arc::new(PassingProbe))
            .unwrap();
        builder
            .register_arc(ProbeId::new("provider.fail"), Arc::new(FailingProbe))
            .unwrap();

        let diagnostics = RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(Arc::new(Mutex::new(builder))),
        );
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        let report = diagnostics.run_readiness_checks(&app).await.unwrap();

        assert_eq!(report.state, ProbeState::Unhealthy);
        assert_eq!(report.probes.len(), 2);
        assert_eq!(report.probes[0].state, ProbeState::Healthy);
        assert_eq!(report.probes[1].state, ProbeState::Unhealthy);
        assert_eq!(report.probes[1].id, ProbeId::new("provider.fail"));
    }

    #[tokio::test]
    async fn readiness_aggregation_reports_panics_as_unhealthy() {
        let mut builder = ReadinessRegistryBuilder::default();
        builder
            .register_arc(ProbeId::new("provider.panic"), Arc::new(PanickingProbe))
            .unwrap();

        let diagnostics = RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(Arc::new(Mutex::new(builder))),
        );
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        let report = diagnostics.run_readiness_checks(&app).await.unwrap();

        assert_eq!(report.state, ProbeState::Unhealthy);
        assert_eq!(report.probes.len(), 1);
        assert_eq!(report.probes[0].id, ProbeId::new("provider.panic"));
        assert_eq!(report.probes[0].state, ProbeState::Unhealthy);
        assert_eq!(
            report.probes[0].message.as_deref(),
            Some("readiness check panicked: probe boom")
        );
    }

    #[tokio::test]
    async fn readiness_aggregation_reports_factory_panics_as_unhealthy() {
        let mut builder = ReadinessRegistryBuilder::default();
        builder
            .register_arc(
                ProbeId::new("provider.factory_panic"),
                Arc::new(FactoryPanickingProbe),
            )
            .unwrap();

        let diagnostics = RuntimeDiagnostics::new(
            RuntimeBackendKind::Memory,
            ReadinessRegistryBuilder::freeze_shared(Arc::new(Mutex::new(builder))),
        );
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        let report = diagnostics.run_readiness_checks(&app).await.unwrap();

        assert_eq!(report.state, ProbeState::Unhealthy);
        assert_eq!(report.probes.len(), 1);
        assert_eq!(report.probes[0].id, ProbeId::new("provider.factory_panic"));
        assert_eq!(report.probes[0].state, ProbeState::Unhealthy);
        assert_eq!(
            report.probes[0].message.as_deref(),
            Some("readiness check panicked: probe factory boom")
        );
    }
}
