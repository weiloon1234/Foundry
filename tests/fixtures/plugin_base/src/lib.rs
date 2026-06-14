use async_trait::async_trait;
use foundry::prelude::*;
use semver::{Version, VersionReq};

pub const BASE_PLUGIN_ID: PluginId = PluginId::new("fixture.plugin.base");
pub const BASE_COMMAND: CommandId = CommandId::new("fixture-base-ping");

#[derive(Clone)]
pub struct FixtureBasePlugin;

#[derive(Clone)]
pub struct FixtureBaseService(pub String);

#[derive(Clone)]
struct FixtureBaseProvider;

#[async_trait]
impl ServiceProvider for FixtureBaseProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        let message = registrar
            .config()
            .string("fixture_plugin.message")
            .unwrap_or_else(|| "base-plugin".to_string());
        registrar.singleton(FixtureBaseService(message))?;
        Ok(())
    }
}

#[async_trait]
impl Plugin for FixtureBasePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            BASE_PLUGIN_ID,
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar
            .config_defaults(
                toml::from_str(
                    r#"
                        [fixture_plugin]
                        message = "base-plugin"
                    "#,
                )
                .unwrap(),
            )
            .register_provider(FixtureBaseProvider)
            .register_commands(register_commands);
        Ok(())
    }
}

fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        BASE_COMMAND,
        Command::new("fixture-base-ping"),
        |invocation| async move {
            let _service = invocation.app().resolve::<FixtureBaseService>()?;
            Ok(())
        },
    )?;
    Ok(())
}
