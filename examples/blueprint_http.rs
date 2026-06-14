use async_trait::async_trait;
use foundry::prelude::*;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        pub const HELLO_COMMAND: CommandId = CommandId::new("hello");
        pub const HEARTBEAT_SCHEDULE: ScheduleId = ScheduleId::new("heartbeat");
        pub const MOBILE_RULE: ValidationRuleId = ValidationRuleId::new("mobile");
    }

    pub mod providers {
        use super::*;

        pub struct AppServiceProvider;

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.singleton::<String>("foundry".to_string())?;
                Ok(())
            }
        }
    }

    pub mod portals {
        use super::*;

        pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route("/", get(index));
            Ok(())
        }

        async fn index(State(app): State<AppContext>) -> impl IntoResponse {
            let name = app
                .resolve::<String>()
                .map(|value| value.to_string())
                .unwrap_or_else(|_| "foundry".to_string());

            Json(serde_json::json!({
                "framework": name,
                "status": "ok",
            }))
        }
    }

    pub mod commands {
        use super::*;

        pub fn register(registry: &mut CommandRegistry) -> Result<()> {
            registry.command(
                ids::HELLO_COMMAND,
                Command::new("hello").about("Hello from Foundry"),
                |_invocation: CommandInvocation| async {
                    println!("hello from foundry");
                    Ok(())
                },
            )?;
            Ok(())
        }
    }

    pub mod schedules {
        use super::*;

        pub fn register(registry: &mut ScheduleRegistry) -> Result<()> {
            registry.cron(
                ids::HEARTBEAT_SCHEDULE,
                CronExpression::parse("*/1 * * * * *")?,
                |_invocation| async { Ok(()) },
            )?;
            Ok(())
        }
    }

    pub mod validation {
        use super::*;

        pub struct MobileRule;

        #[async_trait]
        impl ValidationRule for MobileRule {
            async fn validate(
                &self,
                _context: &RuleContext,
                value: &str,
            ) -> std::result::Result<(), ValidationError> {
                if value.starts_with('+') && value[1..].chars().all(|ch| ch.is_ascii_digit()) {
                    Ok(())
                } else {
                    Err(ValidationError::new("mobile", "invalid mobile number"))
                }
            }
        }
    }
}

fn main() -> Result<()> {
    if std::env::var("FOUNDRY_RUN_EXAMPLE").is_ok() {
        App::builder()
            .load_env()
            .load_config_dir("config")
            .register_provider(app::providers::AppServiceProvider)
            .register_routes(app::portals::router)
            .register_commands(app::commands::register)
            .register_schedule(app::schedules::register)
            .register_validation_rule(app::ids::MOBILE_RULE, app::validation::MobileRule)
            .run_http()?;
    }

    Ok(())
}
