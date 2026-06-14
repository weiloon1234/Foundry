use async_trait::async_trait;
use foundry::prelude::*;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        pub const AUDIT_JOB: JobId = JobId::new("audit.job");
    }

    pub mod domain {
        use super::*;

        #[derive(Debug, Serialize, Deserialize)]
        pub struct AuditJob {
            pub marker: String,
        }

        #[async_trait]
        impl Job for AuditJob {
            const ID: JobId = ids::AUDIT_JOB;
            const QUEUE: Option<QueueId> = Some(QueueId::new("default"));

            async fn handle(&self, _context: JobContext) -> Result<()> {
                Ok(())
            }
        }
    }

    pub mod providers {
        use super::*;

        pub struct AppServiceProvider;

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.register_job::<domain::AuditJob>()?;
                Ok(())
            }
        }
    }
}

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(app::providers::AppServiceProvider)
        .run_worker()
}
