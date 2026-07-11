#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use foundry::prelude::*;
use semver::{Version, VersionReq};
use serde_json::json;

const SURFACE_COMMAND: CommandId = CommandId::new("surface:ping");
const PLUGIN_COMMAND: CommandId = CommandId::new("surface:plugin");
const SURFACE_GUARD: GuardId = GuardId::new("surface");
const SURFACE_POLICY: PolicyId = PolicyId::new("surface.view");
const SURFACE_PERMISSION: PermissionId = PermissionId::new("surface:view");
const SURFACE_JOB: JobId = JobId::new("surface.job");
const TEST_SHUTDOWN_PLUGIN: PluginId = PluginId::new("surface-test-shutdown");
const SURFACE_MIDDLEWARE: MiddlewareGroupId = MiddlewareGroupId::new("surface");

#[derive(Debug, foundry::Model)]
#[foundry(table = "surface_widgets")]
struct SurfaceWidget {
    id: ModelId<Self>,
    name: String,
    created_at: DateTime,
    updated_at: DateTime,
}

#[derive(Debug, Deserialize, Validate)]
struct SurfaceRequest {
    #[validate(required, min(3))]
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SurfaceJob;

#[async_trait]
impl Job for SurfaceJob {
    const ID: JobId = SURFACE_JOB;

    async fn handle(&self, _context: JobContext) -> Result<()> {
        Ok(())
    }
}

struct SurfacePolicy;

#[async_trait]
impl Policy for SurfacePolicy {
    async fn evaluate(&self, actor: &Actor, _app: &AppContext) -> Result<bool> {
        Ok(actor.guard == SURFACE_GUARD)
    }
}

struct SurfaceProvider;

#[async_trait]
impl ServiceProvider for SurfaceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_guard(
            SURFACE_GUARD,
            StaticBearerAuthenticator::new().token(
                "surface-token",
                Actor::new("surface-user", SURFACE_GUARD).with_permissions([SURFACE_PERMISSION]),
            ),
        )?;
        registrar.register_policy(SURFACE_POLICY, SurfacePolicy)?;
        registrar.register_job::<SurfaceJob>()?;
        Ok(())
    }
}

struct SurfacePlugin;

impl Plugin for SurfacePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            PluginId::new("surface-plugin"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        )
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar.register_commands(|commands| {
            commands.command(
                PLUGIN_COMMAND,
                Command::new(PLUGIN_COMMAND.as_str().to_string()),
                |_invocation| async move { Ok(()) },
            )?;
            Ok(())
        });
        Ok(())
    }
}

struct TestShutdownPlugin {
    shutdown: Arc<AtomicBool>,
}

#[async_trait]
impl Plugin for TestShutdownPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            TEST_SHUTDOWN_PLUGIN,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        )
    }

    fn register(&self, _registrar: &mut PluginRegistrar) -> Result<()> {
        Ok(())
    }

    async fn shutdown(&self, _app: &AppContext) -> Result<()> {
        self.shutdown.store(true, Ordering::Release);
        Ok(())
    }
}

fn surface_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.route_with_options(
        "/surface",
        post(surface_handler),
        HttpRouteOptions::new()
            .guard(SURFACE_GUARD)
            .permission(SURFACE_PERMISSION)
            .middleware_group(SURFACE_MIDDLEWARE),
    );
    Ok(())
}

fn surface_commands(commands: &mut CommandRegistry) -> Result<()> {
    commands.command(
        SURFACE_COMMAND,
        Command::new(SURFACE_COMMAND.as_str().to_string()),
        |_invocation| async move { Ok(()) },
    )?;
    Ok(())
}

async fn surface_handler(Validated(payload): Validated<SurfaceRequest>) -> impl IntoResponse {
    Json(json!({ "name": payload.name }))
}

#[test]
fn database_notification_repository_surface_is_nameable_from_the_prelude() {
    let scope = DatabaseNotificationScope::new("surface_user", "surface-user").unwrap();
    let repository = DatabaseNotificationRepository::from_scope(scope.clone());
    let _id: ModelId<DatabaseNotification> = ModelId::generate();

    assert_eq!(repository.scope(), &scope);
    assert_eq!(scope.notifiable_type(), "surface_user");
    assert_eq!(scope.notifiable_id(), "surface-user");
}

#[tokio::test]
async fn blessed_public_surface_composes_for_consumer_apps() {
    let kernel = App::builder()
        .register_provider(SurfaceProvider)
        .register_plugin(SurfacePlugin)
        .middleware_group(SURFACE_MIDDLEWARE, Vec::new())
        .register_routes(surface_routes)
        .register_commands(surface_commands)
        .build_cli_kernel()
        .await
        .unwrap();

    let _query = SurfaceWidget::model_query()
        .where_(SurfaceWidget::NAME.eq("demo"))
        .limit(1);

    let mut validator = Validator::new(kernel.app().clone());
    SurfaceRequest {
        name: "demo".to_string(),
    }
    .validate(&mut validator)
    .await
    .unwrap();
    validator.finish().unwrap();

    let blocking_result = run_blocking("public surface", || Ok(42)).await.unwrap();
    assert_eq!(blocking_result, 42);

    let currency = CountryCurrency {
        code: "MYR".to_string(),
        name: Some("Malaysian ringgit".to_string()),
        symbol: Some("RM".to_string()),
        minor_units: Some(2),
    };
    assert_eq!(currency.code, "MYR");
    assert_eq!(
        CountryStatus::parse("enabled"),
        Some(CountryStatus::Enabled)
    );

    kernel
        .run_with_args(vec![String::from("foundry"), String::from("surface:ping")])
        .await
        .unwrap();
}

#[tokio::test]
async fn doctor_strict_flag_is_available_to_deploy_tooling() {
    let kernel = App::builder().build_cli_kernel().await.unwrap();

    let error = kernel
        .run_with_args(vec![
            String::from("foundry"),
            String::from("doctor"),
            String::from("--deploy"),
            String::from("--strict"),
            String::from("--json"),
        ])
        .await
        .unwrap_err();

    assert!(error.to_string().contains("strict mode"));
}

#[tokio::test]
async fn testing_builders_are_nameable_and_test_apps_shutdown_gracefully() {
    let shutdown = Arc::new(AtomicBool::new(false));
    let builder: TestAppBuilder =
        TestApp::from_builder(App::builder().register_plugin(TestShutdownPlugin {
            shutdown: shutdown.clone(),
        }));
    let app = builder.build().await.unwrap();
    let request: TestRequestBuilder = app.client().get("/health");
    drop(request);

    app.shutdown().await.unwrap();
    assert!(shutdown.load(Ordering::Acquire));
}

#[tokio::test]
async fn custom_kernel_hosts_can_shutdown_the_full_runtime_idempotently() {
    let shutdown = Arc::new(AtomicBool::new(false));
    let kernel = App::builder()
        .register_plugin(TestShutdownPlugin {
            shutdown: shutdown.clone(),
        })
        .build_cli_kernel()
        .await
        .unwrap();

    kernel.app().shutdown().await.unwrap();
    kernel.app().shutdown().await.unwrap();
    assert!(shutdown.load(Ordering::Acquire));
}
