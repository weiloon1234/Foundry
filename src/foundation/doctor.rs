use clap::{Arg, ArgAction, Command};
use serde::Serialize;
use serde_json::{json, Value};

use crate::cli::{CommandInvocation, CommandRegistrar};
use crate::database::lifecycle::migration_status_summary_from_app;
use crate::foundation::{AppContext, Error, Result};
use crate::logging::ProbeState;
use crate::support::runtime::RuntimeBackend;
use crate::support::CommandId;

const DOCTOR_COMMAND: CommandId = CommandId::new("doctor");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DoctorStatus {
    Ok,
    Warning,
    Failed,
}

impl DoctorStatus {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Failed, _) | (_, Self::Failed) => Self::Failed,
            (Self::Warning, _) | (_, Self::Warning) => Self::Warning,
            (Self::Ok, Self::Ok) => Self::Ok,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct DoctorCheck {
    name: &'static str,
    status: DoctorStatus,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
}

impl DoctorCheck {
    fn ok(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: DoctorStatus::Ok,
            message: message.into(),
            details: None,
        }
    }

    fn ok_with_details(name: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            name,
            status: DoctorStatus::Ok,
            message: message.into(),
            details: Some(details),
        }
    }

    fn warning(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: DoctorStatus::Warning,
            message: message.into(),
            details: None,
        }
    }

    fn warning_with_details(
        name: &'static str,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            name,
            status: DoctorStatus::Warning,
            message: message.into(),
            details: Some(details),
        }
    }

    fn failed(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: DoctorStatus::Failed,
            message: message.into(),
            details: None,
        }
    }

    fn failed_with_details(name: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            name,
            status: DoctorStatus::Failed,
            message: message.into(),
            details: Some(details),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct DoctorReport {
    status: DoctorStatus,
    deploy: bool,
    strict: bool,
    checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    fn new(deploy: bool, strict: bool) -> Self {
        Self {
            status: DoctorStatus::Ok,
            deploy,
            strict,
            checks: Vec::new(),
        }
    }

    fn push(&mut self, check: DoctorCheck) {
        self.status = self.status.merge(check.status);
        self.checks.push(check);
    }

    fn failed(&self) -> bool {
        matches!(self.status, DoctorStatus::Failed)
    }

    fn should_fail_command(&self) -> bool {
        self.failed() || (self.strict && matches!(self.status, DoctorStatus::Warning))
    }
}

pub(crate) fn doctor_cli_registrar() -> CommandRegistrar {
    std::sync::Arc::new(|registry| {
        registry.command(
            DOCTOR_COMMAND,
            Command::new(DOCTOR_COMMAND.as_str().to_string())
                .about("Run runtime health checks for deploy and operator diagnostics")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(ArgAction::SetTrue)
                        .help("Print machine-readable JSON"),
                )
                .arg(
                    Arg::new("deploy")
                        .long("deploy")
                        .action(ArgAction::SetTrue)
                        .help("Run checks expected by runtime-only deploy tooling"),
                )
                .arg(
                    Arg::new("strict")
                        .long("strict")
                        .action(ArgAction::SetTrue)
                        .help("Exit non-zero when any warning is reported"),
                ),
            |invocation| async move { doctor_command(invocation).await },
        )?;
        Ok(())
    })
}

async fn doctor_command(invocation: CommandInvocation) -> Result<()> {
    let json = invocation.matches().get_flag("json");
    let deploy = invocation.matches().get_flag("deploy");
    let strict = invocation.matches().get_flag("strict");
    let report = run_doctor(invocation.app(), deploy, strict).await;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(Error::other)?
        );
    } else {
        print_text_report(&report);
    }

    if report.should_fail_command() {
        let reason = if report.failed() {
            "one or more checks failed"
        } else {
            "strict mode treats warnings as failures"
        };
        return Err(Error::message(format!("doctor failed: {reason}")));
    }

    Ok(())
}

async fn run_doctor(app: &AppContext, deploy: bool, strict: bool) -> DoctorReport {
    let mut report = DoctorReport::new(deploy, strict);
    report.push(check_app_config(app));
    report.push(check_security_tier(app));
    report.push(check_config_keys(app));
    report.push(check_legacy_env_overlays(app));
    report.push(check_database(app).await);
    report.push(check_migrations(app).await);
    report.push(check_runtime_backend(app).await);
    report.push(check_cache(app).await);
    report.push(check_readiness(app).await);
    report
}

fn check_app_config(app: &AppContext) -> DoctorCheck {
    match app.config().app() {
        Ok(config) => {
            let signing_key = signing_key_state(&config);
            let security_tier = config.resolved_security_tier();
            let message = format!("app `{}` loaded for `{}`", config.name, config.environment);
            let details = json!({
                "name": config.name.clone(),
                "environment": config.environment.to_string(),
                "security_tier": security_tier.as_str(),
                "security_tier_explicit": config.security_tier.is_some(),
                "timezone": config.timezone.to_string(),
                "signing_key": signing_key,
            });

            if config.signing_key.is_empty() {
                return DoctorCheck::warning_with_details(
                    "config",
                    format!("{message}; signing key is not configured"),
                    details,
                );
            }
            if let Err(error) = config.signing_key_bytes() {
                return DoctorCheck::failed_with_details("config", error.to_string(), details);
            }

            DoctorCheck::ok_with_details("config", message, details)
        }
        Err(error) => DoctorCheck::failed("config", error.to_string()),
    }
}

fn check_security_tier(app: &AppContext) -> DoctorCheck {
    let config = match app.config().app() {
        Ok(config) => config,
        Err(error) => return DoctorCheck::failed("config_security_tier", error.to_string()),
    };
    let tier = config.resolved_security_tier();
    let details = json!({
        "environment": config.environment.to_string(),
        "security_tier": tier.as_str(),
        "explicit": config.security_tier.is_some(),
    });

    if config.custom_security_tier_requires_confirmation() {
        return DoctorCheck::warning_with_details(
            "config_security_tier",
            format!(
                "custom environment `{}` is using the strict fail-closed tier; set app.security_tier explicitly to confirm or override it",
                config.environment
            ),
            details,
        );
    }

    DoctorCheck::ok_with_details(
        "config_security_tier",
        format!("security tier resolved to `{tier}`"),
        details,
    )
}

fn check_config_keys(app: &AppContext) -> DoctorCheck {
    let unknown_config = app.config().unknown_config_keys();
    let unknown_env = app.config().unknown_prefixed_env_overlays();
    let details = json!({
        "unknown_config_keys": unknown_config,
        "unknown_prefixed_env_overlays": unknown_env,
    });

    if unknown_config.is_empty() && unknown_env.is_empty() {
        return DoctorCheck::ok_with_details(
            "config_keys",
            "framework config keys match the published schema",
            details,
        );
    }

    DoctorCheck::warning_with_details(
        "config_keys",
        format!(
            "found {} unknown framework config key(s) and {} unknown FOUNDRY-prefixed env overlay(s)",
            unknown_config.len(),
            unknown_env.len()
        ),
        details,
    )
}

fn check_legacy_env_overlays(app: &AppContext) -> DoctorCheck {
    let overlays = app.config().legacy_unprefixed_env_overlays();
    let details = json!({ "variables": overlays });
    if overlays.is_empty() {
        return DoctorCheck::ok_with_details(
            "config_env_overlays",
            "all config env overlays use the FOUNDRY namespace",
            details,
        );
    }

    DoctorCheck::warning_with_details(
        "config_env_overlays",
        format!(
            "{} legacy unprefixed env overlay(s) remain supported for migration; rename them with the FOUNDRY__ prefix",
            overlays.len()
        ),
        details,
    )
}

fn signing_key_state(config: &crate::config::AppConfig) -> &'static str {
    if config.signing_key.is_empty() {
        "missing"
    } else if config.signing_key_bytes().is_ok() {
        "configured"
    } else {
        "invalid"
    }
}

async fn check_database(app: &AppContext) -> DoctorCheck {
    let database = match app.database() {
        Ok(database) => database,
        Err(error) => return DoctorCheck::failed("database", error.to_string()),
    };

    if !database.is_configured() {
        return DoctorCheck::warning("database", "database is not configured");
    }

    match database.ping().await {
        Ok(()) => {
            let target = if database.has_read_pool() {
                "primary and read replica"
            } else {
                "primary"
            };
            DoctorCheck::ok("database", format!("database ping succeeded for {target}"))
        }
        Err(error) => DoctorCheck::failed("database", error.to_string()),
    }
}

async fn check_migrations(app: &AppContext) -> DoctorCheck {
    let database = match app.database() {
        Ok(database) => database,
        Err(error) => return DoctorCheck::failed("migrations", error.to_string()),
    };

    if !database.is_configured() {
        return DoctorCheck::warning(
            "migrations",
            "migration status skipped; database is not configured",
        );
    }

    match migration_status_summary_from_app(app).await {
        Ok(summary) if summary.missing_applied == 0 => DoctorCheck::ok_with_details(
            "migrations",
            format!(
                "{} registered, {} applied, {} pending",
                summary.registered, summary.applied, summary.pending
            ),
            json!(summary),
        ),
        Ok(summary) => DoctorCheck::failed_with_details(
            "migrations",
            format!(
                "{} applied migration(s) are missing from the current binary",
                summary.missing_applied
            ),
            json!(summary),
        ),
        Err(error) => DoctorCheck::failed("migrations", error.to_string()),
    }
}

async fn check_runtime_backend(app: &AppContext) -> DoctorCheck {
    let backend = match app.resolve::<RuntimeBackend>() {
        Ok(backend) => backend,
        Err(error) => return DoctorCheck::failed("runtime_backend", error.to_string()),
    };
    let kind = backend.kind();

    match backend.ping().await {
        Ok(()) => DoctorCheck::ok_with_details(
            "runtime_backend",
            format!("{kind:?} runtime backend ping succeeded"),
            json!({ "kind": kind }),
        ),
        Err(error) => DoctorCheck::failed_with_details(
            "runtime_backend",
            error.to_string(),
            json!({ "kind": kind }),
        ),
    }
}

async fn check_cache(app: &AppContext) -> DoctorCheck {
    let config = match app.config().cache() {
        Ok(config) => config,
        Err(error) => return DoctorCheck::failed("cache", error.to_string()),
    };
    if matches!(&config.driver, crate::config::CacheDriver::Redis) {
        match app.config().redis() {
            Ok(redis) if redis.url.trim().is_empty() => {
                return DoctorCheck::warning(
                    "cache",
                    "cache driver is redis but redis is not configured",
                );
            }
            Ok(_) => {}
            Err(error) => return DoctorCheck::failed("cache", error.to_string()),
        }
    }

    let cache = match app.cache() {
        Ok(cache) => cache,
        Err(error) => return DoctorCheck::failed("cache", error.to_string()),
    };
    let key = format!("foundry:doctor:{}", uuid::Uuid::now_v7());
    let value = "ok".to_string();
    let details = json!({
        "driver": config.driver,
        "error_mode": config.error_mode,
    });

    match cache
        .put(&key, &value, std::time::Duration::from_secs(30))
        .await
    {
        Ok(()) => {}
        Err(error) => {
            return DoctorCheck::failed_with_details("cache", error.to_string(), details);
        }
    }
    match cache.get::<String>(&key).await {
        Ok(Some(found)) if found == value => {}
        Ok(_) => {
            let _ = cache.forget(&key).await;
            return DoctorCheck::failed_with_details(
                "cache",
                "cache roundtrip did not return stored value",
                details,
            );
        }
        Err(error) => {
            let _ = cache.forget(&key).await;
            return DoctorCheck::failed_with_details("cache", error.to_string(), details);
        }
    }
    match cache.forget(&key).await {
        Ok(_) => DoctorCheck::ok_with_details("cache", "cache roundtrip succeeded", details),
        Err(error) => DoctorCheck::failed_with_details("cache", error.to_string(), details),
    }
}

async fn check_readiness(app: &AppContext) -> DoctorCheck {
    let diagnostics = match app.diagnostics() {
        Ok(diagnostics) => diagnostics,
        Err(error) => return DoctorCheck::failed("readiness", error.to_string()),
    };

    match diagnostics.run_readiness_checks(app).await {
        Ok(report) if report.state == ProbeState::Healthy => {
            DoctorCheck::ok_with_details("readiness", "readiness checks are healthy", json!(report))
        }
        Ok(report) => DoctorCheck::failed_with_details(
            "readiness",
            "one or more readiness checks are unhealthy",
            json!(report),
        ),
        Err(error) => DoctorCheck::failed("readiness", error.to_string()),
    }
}

fn print_text_report(report: &DoctorReport) {
    let strict = if report.strict { " strict" } else { "" };
    println!("Foundry doctor: {}{}", report.status.as_str(), strict);
    for check in &report.checks {
        println!(
            "[{}] {}: {}",
            check.status.as_str(),
            check.name,
            check.message
        );
    }
    println!("{}", readiness_verdict(report));
}

fn readiness_verdict(report: &DoctorReport) -> &'static str {
    match (report.deploy, report.strict, report.status) {
        (_, _, DoctorStatus::Failed) => {
            "Production readiness: failed - fix failed checks before deploy."
        }
        (true, true, DoctorStatus::Warning) => {
            "Production readiness: blocked - strict mode treats warnings as failures."
        }
        (true, false, DoctorStatus::Warning) => {
            "Production readiness: warning - deploy checks completed with warnings; rerun with --strict to enforce them."
        }
        (true, _, DoctorStatus::Ok) => "Production readiness: ready - deploy checks passed.",
        (false, true, DoctorStatus::Warning) => {
            "Doctor verdict: blocked - strict mode treats warnings as failures."
        }
        (false, false, DoctorStatus::Warning) => {
            "Doctor verdict: warning - checks completed with warnings."
        }
        (false, _, DoctorStatus::Ok) => "Doctor verdict: ok - checks passed.",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{DoctorReport, DoctorStatus};
    use crate::config::SecurityTier;
    use crate::foundation::App;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn report_status_tracks_worst_check_status() {
        let mut report = DoctorReport::new(true, false);
        assert_eq!(report.status, DoctorStatus::Ok);

        report.push(super::DoctorCheck::warning("warn", "warning"));
        assert_eq!(report.status, DoctorStatus::Warning);

        report.push(super::DoctorCheck::failed("fail", "failed"));
        assert_eq!(report.status, DoctorStatus::Failed);

        report.push(super::DoctorCheck::ok("ok", "ok"));
        assert_eq!(report.status, DoctorStatus::Failed);
    }

    #[tokio::test]
    async fn default_app_doctor_reports_warnings_without_failure() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let report = super::run_doctor(kernel.app(), true, false).await;

        assert_eq!(report.status, DoctorStatus::Warning);
        assert!(report.deploy);
        assert!(!report.strict);
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "config" && check.status == DoctorStatus::Warning));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "database" && check.status == DoctorStatus::Warning));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "migrations" && check.status == DoctorStatus::Warning));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "cache" && check.status == DoctorStatus::Warning));
        assert!(!report.failed());
        assert!(!report.should_fail_command());
    }

    #[tokio::test]
    async fn strict_doctor_treats_warnings_as_command_failure() {
        let kernel = App::builder().build_cli_kernel().await.unwrap();
        let report = super::run_doctor(kernel.app(), true, true).await;

        assert_eq!(report.status, DoctorStatus::Warning);
        assert!(report.strict);
        assert!(report.should_fail_command());
        assert_eq!(
            super::readiness_verdict(&report),
            "Production readiness: blocked - strict mode treats warnings as failures."
        );
    }

    #[tokio::test]
    async fn custom_environment_fails_closed_and_strict_doctor_requires_confirmation() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                environment = "eu-prod"
            "#,
        )
        .unwrap();
        let kernel = App::builder()
            .load_config_dir(directory.path())
            .build_cli_kernel()
            .await
            .unwrap();

        assert_eq!(
            kernel
                .app()
                .config()
                .app()
                .unwrap()
                .resolved_security_tier(),
            SecurityTier::Strict
        );
        let report = super::run_doctor(kernel.app(), true, true).await;
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "config_security_tier")
            .unwrap();

        assert_eq!(check.status, DoctorStatus::Warning);
        assert_eq!(check.details.as_ref().unwrap()["security_tier"], "strict");
        assert!(report.should_fail_command());
    }

    #[tokio::test]
    async fn strict_doctor_blocks_unknown_keys_and_legacy_env_overlays() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                enviroment = "production"

                [databse]
                url = "postgres://localhost/foundry"
            "#,
        )
        .unwrap();
        std::env::set_var("FOUNDRY__SERVRE__PORT", "4555");
        std::env::set_var("CFG03_LEGACY__FLAG", "true");
        let kernel = App::builder()
            .load_config_dir(directory.path())
            .build_cli_kernel()
            .await
            .unwrap();
        std::env::remove_var("FOUNDRY__SERVRE__PORT");
        std::env::remove_var("CFG03_LEGACY__FLAG");

        let report = super::run_doctor(kernel.app(), true, true).await;
        let keys = report
            .checks
            .iter()
            .find(|check| check.name == "config_keys")
            .unwrap();
        let overlays = report
            .checks
            .iter()
            .find(|check| check.name == "config_env_overlays")
            .unwrap();

        assert_eq!(keys.status, DoctorStatus::Warning);
        assert_eq!(
            keys.details.as_ref().unwrap()["unknown_config_keys"],
            json!(["app.enviroment", "databse.url"])
        );
        assert_eq!(
            keys.details.as_ref().unwrap()["unknown_prefixed_env_overlays"],
            json!(["FOUNDRY__SERVRE__PORT"])
        );
        assert_eq!(overlays.status, DoctorStatus::Warning);
        assert!(overlays.details.as_ref().unwrap()["variables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "CFG03_LEGACY__FLAG"));
        assert!(report.should_fail_command());
    }

    #[tokio::test]
    async fn doctor_reports_cache_roundtrip_status() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-cache.toml"),
            r#"
                [cache]
                driver = "memory"
            "#,
        )
        .unwrap();
        let kernel = App::builder()
            .load_config_dir(directory.path())
            .build_cli_kernel()
            .await
            .unwrap();

        let report = super::run_doctor(kernel.app(), true, false).await;
        let cache = report
            .checks
            .iter()
            .find(|check| check.name == "cache")
            .expect("cache check should be present");

        assert_eq!(cache.status, DoctorStatus::Ok);
        assert!(cache.message.contains("cache roundtrip succeeded"));
    }

    #[tokio::test]
    async fn doctor_fails_invalid_signing_key_configuration() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            r#"
                [app]
                signing_key = "AA=="
            "#,
        )
        .unwrap();
        let kernel = App::builder()
            .load_config_dir(directory.path())
            .build_cli_kernel()
            .await
            .unwrap();

        let report = super::run_doctor(kernel.app(), true, false).await;
        let config = report
            .checks
            .iter()
            .find(|check| check.name == "config")
            .expect("config check should be present");

        assert_eq!(config.status, DoctorStatus::Failed);
        assert!(config.message.contains("at least 32 bytes"));
        assert_eq!(config.details.as_ref().unwrap()["signing_key"], "invalid");
    }

    #[test]
    fn doctor_report_json_includes_deploy_flag_and_status() {
        let mut report = DoctorReport::new(true, true);
        report.push(super::DoctorCheck::ok("config", "loaded"));

        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["status"], "ok");
        assert_eq!(value["deploy"], true);
        assert_eq!(value["strict"], true);
        assert_eq!(value["checks"][0]["name"], "config");
    }
}
