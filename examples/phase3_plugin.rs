use async_trait::async_trait;
use foundry::prelude::*;
use semver::{Version, VersionReq};

const DEMO_PLUGIN_ID: PluginId = PluginId::new("example.plugin.demo");
const GREETING_COMMAND: CommandId = CommandId::new("greet");
const PLUGIN_ASSET: PluginAssetId = PluginAssetId::new("plugin-config");
const PLUGIN_SCAFFOLD: PluginScaffoldId = PluginScaffoldId::new("portal");

#[derive(Clone)]
struct DemoPlugin;

#[derive(Clone)]
struct GreetingService(String);

#[derive(Clone)]
struct DemoProvider;

#[async_trait]
impl ServiceProvider for DemoProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        let greeting = registrar
            .config()
            .string("plugin_demo.greeting")
            .unwrap_or_else(|| "hello from plugin".to_string());
        registrar.singleton(GreetingService(greeting))?;
        Ok(())
    }
}

#[async_trait]
impl Plugin for DemoPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            DEMO_PLUGIN_ID,
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .description("Example compile-time plugin")
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar
            .config_defaults(
                toml::from_str(
                    r#"
                        [plugin_demo]
                        greeting = "hello from plugin"
                    "#,
                )
                .unwrap(),
            )
            .register_provider(DemoProvider)
            .register_routes(routes)
            .register_commands(commands);
        registrar.register_assets([PluginAsset::text(
            PLUGIN_ASSET,
            PluginAssetKind::Config,
            "config/plugin-demo.toml",
            "enabled = true\n",
        )])?;
        registrar.register_scaffolds([PluginScaffold::new(PLUGIN_SCAFFOLD)
            .variable(PluginScaffoldVar::new("name"))
            .file(
                "src/generated/{{name}}.rs",
                "pub const PORTAL_NAME: &str = \"{{name}}\";\n",
            )])?;
        Ok(())
    }
}

fn routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.route("/plugin/health", get(plugin_health));
    Ok(())
}

async fn plugin_health(State(app): State<AppContext>) -> impl IntoResponse {
    let greeting = app.resolve::<GreetingService>().unwrap();
    Json(serde_json::json!({
        "greeting": greeting.0,
    }))
}

fn commands(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        GREETING_COMMAND,
        Command::new("greet").about("Example plugin command"),
        |invocation| async move {
            let greeting = invocation.app().resolve::<GreetingService>()?;
            println!("{}", greeting.0);
            Ok(())
        },
    )?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let _http = App::builder()
        .register_plugin(DemoPlugin)
        .build_http_kernel()
        .await?;

    let _cli = App::builder()
        .register_plugin(DemoPlugin)
        .build_cli_kernel()
        .await?;

    Ok(())
}
