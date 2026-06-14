use async_trait::async_trait;
use foundry::prelude::*;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        #[derive(Clone, Copy)]
        pub enum AuthGuard {
            Api,
        }

        impl From<AuthGuard> for GuardId {
            fn from(value: AuthGuard) -> Self {
                match value {
                    AuthGuard::Api => GuardId::new("api"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum PolicyKey {
            IsAdmin,
        }

        impl From<PolicyKey> for PolicyId {
            fn from(value: PolicyKey) -> Self {
                match value {
                    PolicyKey::IsAdmin => PolicyId::new("is_admin"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum RoleKey {
            Admin,
        }

        impl From<RoleKey> for RoleId {
            fn from(value: RoleKey) -> Self {
                match value {
                    RoleKey::Admin => RoleId::new("admin"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum Ability {
            ReportsView,
            WsChat,
        }

        impl From<Ability> for PermissionId {
            fn from(value: Ability) -> Self {
                match value {
                    Ability::ReportsView => PermissionId::new("reports:view"),
                    Ability::WsChat => PermissionId::new("ws:chat"),
                }
            }
        }

        pub const MOBILE_RULE: ValidationRuleId = ValidationRuleId::new("mobile");
        pub const PING_COMMAND: CommandId = CommandId::new("ping");
        pub const HEARTBEAT_SCHEDULE: ScheduleId = ScheduleId::new("heartbeat");
        pub const USER_CREATED: EventId = EventId::new("user.created");
        pub const AUDIT_JOB: JobId = JobId::new("audit.job");
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
        pub struct AuditJob {
            pub marker: String,
        }

        #[async_trait]
        impl Job for AuditJob {
            const ID: JobId = ids::AUDIT_JOB;

            async fn handle(&self, _context: JobContext) -> Result<()> {
                Ok(())
            }
        }
    }

    pub mod providers {
        use super::*;

        #[derive(Clone)]
        pub struct AppServiceProvider;

        pub struct AdminPolicy;

        #[async_trait]
        impl Policy for AdminPolicy {
            async fn evaluate(&self, actor: &Actor, _app: &AppContext) -> Result<bool> {
                Ok(actor.has_role(ids::RoleKey::Admin))
            }
        }

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.register_guard(
                    ids::AuthGuard::Api,
                    StaticBearerAuthenticator::new().token(
                        "admin-token",
                        Actor::new("admin-1", ids::AuthGuard::Api)
                            .with_roles([ids::RoleKey::Admin])
                            .with_permissions([ids::Ability::ReportsView, ids::Ability::WsChat]),
                    ),
                )?;
                registrar.register_policy(ids::PolicyKey::IsAdmin, AdminPolicy)?;
                registrar.listen_event::<domain::UserCreated, _>(dispatch_job(
                    |_event: &domain::UserCreated| domain::AuditJob {
                        marker: "created".to_string(),
                    },
                ))?;
                registrar.register_job::<domain::AuditJob>()?;
                Ok(())
            }
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
                if value.starts_with('+') {
                    Ok(())
                } else {
                    Err(ValidationError::new("mobile", "invalid mobile"))
                }
            }
        }
    }

    pub mod portals {
        use super::*;

        #[derive(Debug, Deserialize)]
        pub struct CreateUser {
            pub phone: String,
        }

        #[async_trait]
        impl RequestValidator for CreateUser {
            async fn validate(&self, validator: &mut Validator) -> Result<()> {
                validator
                    .field("phone", self.phone.clone())
                    .required()
                    .rule(ids::MOBILE_RULE)
                    .apply()
                    .await
            }
        }

        #[async_trait]
        impl foundry::validation::FromMultipart for CreateUser {
            async fn from_multipart(
                multipart: &mut axum::extract::Multipart,
            ) -> foundry::foundation::Result<Self> {
                let mut phone = None;
                while let Some(field) = multipart.next_field().await.map_err(|e| {
                    foundry::foundation::Error::message(format!("multipart error: {e}"))
                })? {
                    if field.name() == Some("phone") {
                        phone = Some(field.text().await.map_err(|e| {
                            foundry::foundation::Error::message(format!("field error: {e}"))
                        })?);
                    }
                }
                Ok(Self {
                    phone: phone.unwrap_or_default(),
                })
            }
        }

        pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route_with_options(
                "/users",
                post(create_user),
                HttpRouteOptions::new()
                    .guard(ids::AuthGuard::Api)
                    .permission(ids::Ability::ReportsView),
            );
            Ok(())
        }

        async fn create_user(
            _request_id: RequestId,
            _actor: CurrentActor,
            Validated(_payload): Validated<CreateUser>,
        ) -> impl IntoResponse {
            StatusCode::CREATED
        }
    }

    pub mod commands {
        use super::*;

        pub fn register(registry: &mut CommandRegistry) -> Result<()> {
            registry.command(
                ids::PING_COMMAND,
                Command::new("ping"),
                |_invocation: CommandInvocation| async move { Ok(()) },
            )?;
            Ok(())
        }
    }

    pub mod schedules {
        use super::*;

        pub fn register(registry: &mut ScheduleRegistry) -> Result<()> {
            registry.cron(
                ids::HEARTBEAT_SCHEDULE,
                CronExpression::parse("*/5 * * * * *")?,
                |_invocation| async move { Ok(()) },
            )?;
            Ok(())
        }
    }

    pub mod realtime {
        use super::*;

        pub fn register(registrar: &mut WebSocketRegistrar) -> Result<()> {
            registrar.channel_with_options(
                ids::CHAT_CHANNEL,
                |context: WebSocketContext, payload: serde_json::Value| async move {
                    context.publish(ids::ECHO_EVENT, payload).await
                },
                WebSocketChannelOptions::new()
                    .guard(ids::AuthGuard::Api)
                    .permission(ids::Ability::WsChat),
            )?;
            Ok(())
        }
    }
}

fn main() -> Result<()> {
    let _builder = App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(app::providers::AppServiceProvider)
        .register_routes(app::portals::router)
        .register_commands(app::commands::register)
        .register_schedule(app::schedules::register)
        .register_websocket_routes(app::realtime::register)
        .register_validation_rule(app::ids::MOBILE_RULE, app::validation::MobileRule);

    Ok(())
}
