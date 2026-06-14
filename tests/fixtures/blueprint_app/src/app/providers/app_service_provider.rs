use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use foundry::prelude::*;

use crate::app::ids;

pub struct AppServiceProvider;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.singleton_arc(shared_log())?;
        registrar.register_guard(
            ids::AuthGuard::Api,
            StaticBearerAuthenticator::new().token(
                "fixture-token",
                Actor::new("fixture-user", ids::AuthGuard::Api).with_permissions([
                    ids::Ability::DashboardView,
                    ids::Ability::RealtimeChat,
                ]),
            ),
        )?;
        crate::app::datatables::register(registrar)?;
        Ok(())
    }

    async fn boot(&self, app: &AppContext) -> Result<()> {
        let entries = app.resolve::<Mutex<Vec<String>>>()?;
        entries.lock().unwrap().push("provider:boot".to_string());
        Ok(())
    }
}

pub fn shared_log() -> Arc<Mutex<Vec<String>>> {
    static LOG: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
    LOG.get_or_init(|| Arc::new(Mutex::new(Vec::new()))).clone()
}
