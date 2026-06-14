use async_trait::async_trait;
use foundry::prelude::*;
use foundry_fixture_plugin_base::{FixtureBaseService, BASE_PLUGIN_ID};
use semver::{Version, VersionReq};

pub const DEPENDENT_PLUGIN_ID: PluginId = PluginId::new("fixture.plugin.dependent");

#[derive(Clone)]
pub struct FixtureDependentPlugin;

#[derive(Clone)]
pub struct FixtureDependentService(pub String);

#[derive(Clone)]
struct FixtureDependentProvider;

#[async_trait]
impl ServiceProvider for FixtureDependentProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        let service = registrar.resolve::<FixtureBaseService>()?;
        registrar.singleton(FixtureDependentService(format!("dep:{}", service.0)))?;
        Ok(())
    }
}

#[async_trait]
impl Plugin for FixtureDependentPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            DEPENDENT_PLUGIN_ID,
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .dependency(PluginDependency::new(
            BASE_PLUGIN_ID,
            VersionReq::parse("^1").unwrap(),
        ))
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar.register_provider(FixtureDependentProvider);
        Ok(())
    }
}
