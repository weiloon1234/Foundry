use async_trait::async_trait;
use foundry::prelude::*;
use foundry_fixture_plugin_dep::FixtureDependentService;

#[derive(Clone)]
pub struct AppProvider;

#[derive(Clone)]
pub struct AppReady(pub String);

#[async_trait]
impl ServiceProvider for AppProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        let dependent = registrar.resolve::<FixtureDependentService>()?;
        registrar.singleton(AppReady(dependent.0.clone()))?;
        Ok(())
    }
}
