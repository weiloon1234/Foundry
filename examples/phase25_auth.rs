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

        #[derive(Clone, Copy)]
        pub enum RealtimeChannel {
            SecureChat,
        }

        impl From<RealtimeChannel> for ChannelId {
            fn from(value: RealtimeChannel) -> Self {
                match value {
                    RealtimeChannel::SecureChat => ChannelId::new("secure_chat"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum RealtimeEvent {
            Echo,
        }

        impl From<RealtimeEvent> for ChannelEventId {
            fn from(value: RealtimeEvent) -> Self {
                match value {
                    RealtimeEvent::Echo => ChannelEventId::new("echo"),
                }
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
                    StaticBearerAuthenticator::new()
                        .token(
                            "viewer-token",
                            Actor::new("viewer-1", ids::AuthGuard::Api).with_permissions([
                                ids::Ability::ReportsView,
                                ids::Ability::WsChat,
                            ]),
                        )
                        .token(
                            "admin-token",
                            Actor::new("admin-1", ids::AuthGuard::Api)
                                .with_roles([ids::RoleKey::Admin])
                                .with_permissions([
                                    ids::Ability::ReportsView,
                                    ids::Ability::WsChat,
                                ]),
                        ),
                )?;
                registrar.register_policy(ids::PolicyKey::IsAdmin, AdminPolicy)?;
                Ok(())
            }
        }
    }

    pub mod http {
        use super::*;

        pub fn register(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route_with_options(
                "/me",
                get(current_user),
                HttpRouteOptions::new()
                    .guard(ids::AuthGuard::Api)
                    .permission(ids::Ability::ReportsView),
            );
            Ok(())
        }

        async fn current_user(
            State(app): State<AppContext>,
            request_id: RequestId,
            actor: CurrentActor,
        ) -> impl IntoResponse {
            let is_admin = app
                .authorizer()
                .unwrap()
                .allows_policy(&actor, ids::PolicyKey::IsAdmin)
                .await
                .unwrap();

            Json(serde_json::json!({
                "request_id": request_id.to_string(),
                "actor_id": actor.id,
                "guard": actor.guard.as_str(),
                "is_admin": is_admin,
            }))
        }
    }

    pub mod realtime {
        use super::*;

        pub fn register(registrar: &mut WebSocketRegistrar) -> Result<()> {
            registrar.channel_with_options(
                ids::RealtimeChannel::SecureChat,
                |context: WebSocketContext, payload: serde_json::Value| async move {
                    context
                        .publish(
                            ids::RealtimeEvent::Echo,
                            serde_json::json!({
                                "actor_id": context.actor().map(|actor| actor.id.clone()),
                                "body": payload["body"].clone(),
                            }),
                        )
                        .await
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
    if std::env::var("FOUNDRY_RUN_PHASE25_EXAMPLE").is_ok() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(Error::other)?;

        runtime.block_on(async {
            let http = App::builder()
                .load_env()
                .load_config_dir("config")
                .register_provider(app::providers::AppServiceProvider)
                .register_routes(app::http::register)
                .build_http_kernel()
                .await?;

            let websocket = App::builder()
                .load_env()
                .load_config_dir("config")
                .register_provider(app::providers::AppServiceProvider)
                .register_websocket_routes(app::realtime::register)
                .build_websocket_kernel()
                .await?;

            tokio::try_join!(http.serve(), websocket.serve())?;
            Result::<()>::Ok(())
        })?;
    }

    Ok(())
}
