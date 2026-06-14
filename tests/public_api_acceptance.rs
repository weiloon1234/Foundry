#![allow(dead_code)]

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

fn surface_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.route_with_options(
        "/surface",
        post(surface_handler),
        HttpRouteOptions::new()
            .guard(SURFACE_GUARD)
            .permission(SURFACE_PERMISSION),
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

#[tokio::test]
async fn blessed_public_surface_composes_for_consumer_apps() {
    let kernel = App::builder()
        .register_provider(SurfaceProvider)
        .register_plugin(SurfacePlugin)
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
