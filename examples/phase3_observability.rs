use async_trait::async_trait;
use foundry::prelude::*;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        #[derive(Clone, Copy)]
        pub enum ProbeKey {
            Storage,
        }

        impl From<ProbeKey> for ProbeId {
            fn from(value: ProbeKey) -> Self {
                match value {
                    ProbeKey::Storage => ProbeId::new("storage.ready"),
                }
            }
        }
    }

    pub mod providers {
        use super::*;

        #[derive(Clone)]
        pub struct AppServiceProvider;

        pub struct StorageProbe;

        #[async_trait]
        impl ReadinessCheck for StorageProbe {
            async fn run(&self, _app: &AppContext) -> Result<ProbeResult> {
                Ok(ProbeResult::healthy(ids::ProbeKey::Storage))
            }
        }

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.register_readiness_check(ids::ProbeKey::Storage, StorageProbe)?;
                Ok(())
            }
        }
    }

    pub mod portals {
        use super::*;

        pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route("/hello", get(hello));
            Ok(())
        }

        async fn hello(request_id: RequestId) -> impl IntoResponse {
            Json(serde_json::json!({
                "message": "hello from foundry",
                "request_id": request_id.to_string(),
            }))
        }
    }
}

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(app::providers::AppServiceProvider)
        .register_routes(app::portals::router)
        .enable_observability()
        .run_http()
}
