use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use foundry::prelude::*;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        pub const USER_CREATED: EventId = EventId::new("user.created");
        pub const NOTIFY_JOB: JobId = JobId::new("notify.job");
        pub const CHAT_CHANNEL: ChannelId = ChannelId::new("chat");
        pub const ECHO_EVENT: ChannelEventId = ChannelEventId::new("echo");
    }

    pub mod domain {
        use super::*;

        #[derive(Clone, Serialize)]
        pub struct UserCreated {
            pub email: String,
        }

        impl Event for UserCreated {
            const ID: EventId = ids::USER_CREATED;
        }

        #[derive(Debug, Serialize, Deserialize)]
        pub struct NotifyJob {
            pub email: String,
        }

        #[async_trait]
        impl Job for NotifyJob {
            const ID: JobId = ids::NOTIFY_JOB;

            async fn handle(&self, context: JobContext) -> Result<()> {
                let log = context.app().resolve::<Mutex<Vec<String>>>()?;
                log.lock()
                    .unwrap()
                    .push(format!("processed:{}", self.email));
                Ok(())
            }
        }
    }

    pub mod providers {
        use super::*;

        #[derive(Clone)]
        pub struct AppServiceProvider {
            pub log: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.singleton_arc(self.log.clone())?;
                registrar.listen_event::<domain::UserCreated, _>(dispatch_job(
                    |event: &domain::UserCreated| domain::NotifyJob {
                        email: event.email.clone(),
                    },
                ))?;
                registrar.register_job::<domain::NotifyJob>()?;
                Ok(())
            }
        }
    }

    pub mod realtime {
        use super::*;

        pub fn register(registrar: &mut WebSocketRegistrar) -> Result<()> {
            registrar.channel(
                ids::CHAT_CHANNEL,
                |context: WebSocketContext, payload: serde_json::Value| async move {
                    context.publish(ids::ECHO_EVENT, payload).await
                },
            )?;
            Ok(())
        }
    }
}

fn base_builder(log: Arc<Mutex<Vec<String>>>) -> AppBuilder {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(app::providers::AppServiceProvider { log })
}

fn main() -> Result<()> {
    if let Ok(mode) = std::env::var("FOUNDRY_RUN_PHASE2_EXAMPLE") {
        let log = Arc::new(Mutex::new(Vec::new()));
        match mode.as_str() {
            "worker" => base_builder(log).run_worker()?,
            _ => base_builder(log)
                .register_websocket_routes(app::realtime::register)
                .run_websocket()?,
        }
    }

    Ok(())
}
