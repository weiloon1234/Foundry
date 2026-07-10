use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use foundry::prelude::*;
use futures_util::{SinkExt, StreamExt};
use tempfile::tempdir;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        #[derive(Clone, Copy)]
        pub enum AuthGuard {
            Api,
            Admin,
        }

        impl From<AuthGuard> for GuardId {
            fn from(value: AuthGuard) -> Self {
                match value {
                    AuthGuard::Api => GuardId::new("api"),
                    AuthGuard::Admin => GuardId::new("admin"),
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

        pub const SECURE_CHAT_CHANNEL: ChannelId = ChannelId::new("secure_chat");
        pub const SECURE_PRESENCE_CHANNEL: ChannelId = ChannelId::new("secure_presence");
        pub const ADMIN_ONLY_CHANNEL: ChannelId = ChannelId::new("admin_only");
        pub const ECHO_EVENT: ChannelEventId = ChannelEventId::new("echo");
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
                        .token("guest-token", Actor::new("guest-1", ids::AuthGuard::Api))
                        .token(
                            "mfa-pending-token",
                            Actor::new("pending-1", ids::AuthGuard::Api).with_permissions([
                                ids::Ability::ReportsView.into(),
                                ids::Ability::WsChat.into(),
                                PermissionId::new(foundry::auth::token::MFA_PENDING_ABILITY),
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
                registrar.register_guard(
                    ids::AuthGuard::Admin,
                    StaticBearerAuthenticator::new().token(
                        "api-token-for-admin-guard",
                        Actor::new("viewer-1", ids::AuthGuard::Api),
                    ),
                )?;
                registrar.register_policy(ids::PolicyKey::IsAdmin, AdminPolicy)?;
                Ok(())
            }
        }
    }

    pub mod http {
        use super::*;

        pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route("/public", get(public));
            registrar.route_with_options(
                "/me",
                get(current_user),
                HttpRouteOptions::new()
                    .guard(ids::AuthGuard::Api)
                    .permission(ids::Ability::ReportsView),
            );
            Ok(())
        }

        async fn public(request_id: RequestId, actor: OptionalActor) -> impl IntoResponse {
            Json(serde_json::json!({
                "request_id": request_id.to_string(),
                "actor_id": actor.as_ref().map(|actor| actor.id.clone()),
            }))
        }

        async fn current_user(
            State(app): State<AppContext>,
            request_id: RequestId,
            actor: CurrentActor,
            optional: OptionalActor,
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
                "optional_actor_id": optional.as_ref().map(|actor| actor.id.clone()),
                "is_admin": is_admin,
            }))
        }
    }

    pub mod realtime {
        use super::*;

        pub fn register(registrar: &mut WebSocketRegistrar) -> Result<()> {
            registrar.channel_with_options(
                ids::SECURE_CHAT_CHANNEL,
                |context: WebSocketContext, payload: serde_json::Value| async move {
                    context
                        .publish(
                            ids::ECHO_EVENT,
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
            registrar.channel_with_options(
                ids::SECURE_PRESENCE_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move { Ok(()) },
                WebSocketChannelOptions::new()
                    .presence(true)
                    .guard(ids::AuthGuard::Api)
                    .permission(ids::Ability::WsChat),
            )?;
            registrar.channel_with_options(
                ids::ADMIN_ONLY_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move { Ok(()) },
                WebSocketChannelOptions::new().guard(ids::AuthGuard::Admin),
            )?;
            Ok(())
        }
    }
}

#[derive(Clone)]
struct RevocableBearerAuthenticator {
    actor: Arc<RwLock<Option<Actor>>>,
}

#[async_trait]
impl BearerAuthenticator for RevocableBearerAuthenticator {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>> {
        if token != "revocable-token" {
            return Ok(None);
        }
        Ok(self.actor.read().unwrap().clone())
    }
}

struct RevocableAuthProvider {
    actor: Arc<RwLock<Option<Actor>>>,
}

#[async_trait]
impl ServiceProvider for RevocableAuthProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_guard(
            app::ids::AuthGuard::Api,
            RevocableBearerAuthenticator {
                actor: self.actor.clone(),
            },
        )?;
        registrar.register_guard(app::ids::AuthGuard::Admin, StaticBearerAuthenticator::new())?;
        Ok(())
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn write_auth_config(dir: &Path, server_port: u16, websocket_port: u16, namespace: &str) {
    fs::write(
        dir.join("00-runtime.toml"),
        format!(
            r#"
            [server]
            host = "127.0.0.1"
            port = {server_port}

            [websocket]
            host = "127.0.0.1"
            port = {websocket_port}
            path = "/ws"

            [redis]
            namespace = "{namespace}"

            [auth]
            default_guard = "api"
            bearer_prefix = "Bearer"
        "#
        ),
    )
    .unwrap();
}

fn build_http_app(config_dir: &Path) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::AppServiceProvider)
        .register_routes(app::http::router)
}

fn build_websocket_app(config_dir: &Path) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::AppServiceProvider)
        .register_websocket_routes(app::realtime::register)
}

async fn wait_for_http_ready(base_url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..40 {
        if client
            .get(format!("{base_url}/public"))
            .send()
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("http server did not become ready");
}

async fn connect_websocket_with_token(
    url: &str,
    token: Option<&str>,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    for _ in 0..40 {
        let request = if let Some(token) = token {
            let mut request = url.into_client_request().unwrap();
            request
                .headers_mut()
                .insert("Authorization", format!("Bearer {token}").parse().unwrap());
            request
        } else {
            url.into_client_request().unwrap()
        };

        if let Ok((socket, _)) = connect_async(request).await {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("websocket server did not become ready");
}

async fn next_websocket_message(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> ServerMessage {
    let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("timed out waiting for websocket frame")
        .unwrap()
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

async fn assert_websocket_closes_without_event(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    forbidden_event: &ChannelEventId,
) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(payload))) => {
                    let message: ServerMessage = serde_json::from_str(&payload).unwrap();
                    assert_ne!(&message.event, forbidden_event);
                }
                Some(Ok(Message::Ping(payload))) => {
                    socket.send(Message::Pong(payload)).await.unwrap();
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            }
        }
    })
    .await
    .expect("websocket stayed open after cached credentials became invalid");
}

#[tokio::test]
async fn guarded_http_routes_enforce_auth_and_echo_request_id() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_auth_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("auth-http-{server_port}"),
    );

    let server = tokio::spawn({
        let builder = build_http_app(config_dir.path());
        async move { builder.run_http_async().await.unwrap() }
    });

    let base_url = format!("http://127.0.0.1:{server_port}");
    wait_for_http_ready(&base_url).await;
    let client = reqwest::Client::new();

    let public = client
        .get(format!("{base_url}/public"))
        .header("x-request-id", "custom-request-id")
        .send()
        .await
        .unwrap();
    assert_eq!(public.status(), reqwest::StatusCode::OK);
    assert_eq!(
        public.headers().get("x-request-id").unwrap(),
        "custom-request-id"
    );
    let public_payload: serde_json::Value = public.json().await.unwrap();
    assert_eq!(public_payload["request_id"], "custom-request-id");
    assert!(public_payload["actor_id"].is_null());

    let unauthorized = client.get(format!("{base_url}/me")).send().await.unwrap();
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
    let unauthorized_payload: serde_json::Value = unauthorized.json().await.unwrap();
    assert_eq!(
        unauthorized_payload["message"],
        "The Authorization header is missing."
    );
    assert_eq!(
        unauthorized_payload["error_code"],
        "missing_authorization_header"
    );
    assert_eq!(
        unauthorized_payload["message_key"],
        "auth.missing_authorization_header"
    );
    assert!(unauthorized_payload.get("code").is_none());

    let forbidden = client
        .get(format!("{base_url}/me"))
        .bearer_auth("guest-token")
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), reqwest::StatusCode::FORBIDDEN);
    let forbidden_payload: serde_json::Value = forbidden.json().await.unwrap();
    assert_eq!(
        forbidden_payload["message"],
        "You do not have permission to perform this action."
    );
    assert_eq!(
        forbidden_payload["error_code"],
        "missing_required_permission"
    );
    assert_eq!(
        forbidden_payload["message_key"],
        "auth.missing_required_permission"
    );
    assert!(forbidden_payload.get("code").is_none());

    let viewer = client
        .get(format!("{base_url}/me"))
        .bearer_auth("viewer-token")
        .send()
        .await
        .unwrap();
    assert_eq!(viewer.status(), reqwest::StatusCode::OK);
    let viewer_request_id = viewer
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let viewer_payload: serde_json::Value = viewer.json().await.unwrap();
    assert_eq!(viewer_payload["actor_id"], "viewer-1");
    assert_eq!(viewer_payload["optional_actor_id"], "viewer-1");
    assert_eq!(viewer_payload["is_admin"], false);
    assert_eq!(viewer_payload["request_id"], viewer_request_id);

    let admin = client
        .get(format!("{base_url}/me"))
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(admin.status(), reqwest::StatusCode::OK);
    let admin_payload: serde_json::Value = admin.json().await.unwrap();
    assert_eq!(admin_payload["actor_id"], "admin-1");
    assert_eq!(admin_payload["is_admin"], true);

    server.abort();
}

#[tokio::test]
async fn guarded_websocket_channels_require_auth_and_permissions() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_auth_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("auth-ws-{websocket_port}"),
    );

    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path());
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let url = format!("ws://127.0.0.1:{websocket_port}/ws");

    let mut anonymous = connect_websocket_with_token(&url, None).await;
    anonymous
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::SECURE_CHAT_CHANNEL,
                room: None,
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let anonymous_error = anonymous.next().await.unwrap().unwrap();
    let anonymous_error: ServerMessage =
        serde_json::from_str(anonymous_error.to_text().unwrap()).unwrap();
    assert_eq!(anonymous_error.channel, SYSTEM_CHANNEL);
    assert_eq!(anonymous_error.event, ERROR_EVENT);
    assert_eq!(
        anonymous_error.payload["message"],
        "Authentication credentials are required."
    );
    assert_eq!(
        anonymous_error.payload["error_code"],
        "missing_auth_credentials"
    );
    assert_eq!(
        anonymous_error.payload["message_key"],
        "auth.missing_auth_credentials"
    );
    assert!(anonymous_error.payload.get("code").is_none());

    let mut guest = connect_websocket_with_token(&url, Some("guest-token")).await;
    guest
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::SECURE_CHAT_CHANNEL,
                room: None,
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let guest_error = guest.next().await.unwrap().unwrap();
    let guest_error: ServerMessage = serde_json::from_str(guest_error.to_text().unwrap()).unwrap();
    assert_eq!(guest_error.event, ERROR_EVENT);
    assert_eq!(
        guest_error.payload["message"],
        "You do not have permission to perform this action."
    );
    assert_eq!(
        guest_error.payload["error_code"],
        "missing_required_permission"
    );
    assert_eq!(
        guest_error.payload["message_key"],
        "auth.missing_required_permission"
    );
    assert!(guest_error.payload.get("code").is_none());

    let mut mfa_pending = connect_websocket_with_token(&url, Some("mfa-pending-token")).await;
    mfa_pending
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::SECURE_CHAT_CHANNEL,
                room: None,
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let mfa_pending_error = next_websocket_message(&mut mfa_pending).await;
    assert_eq!(mfa_pending_error.channel, SYSTEM_CHANNEL);
    assert_eq!(mfa_pending_error.event, ERROR_EVENT);
    assert_eq!(
        mfa_pending_error.payload["message"],
        "Multi-factor authentication verification is required."
    );

    let mut viewer = connect_websocket_with_token(&url, Some("viewer-token")).await;
    viewer
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::SECURE_CHAT_CHANNEL,
                room: None,
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let subscribed = viewer.next().await.unwrap().unwrap();
    let subscribed: ServerMessage = serde_json::from_str(subscribed.to_text().unwrap()).unwrap();
    assert_eq!(subscribed.event, SUBSCRIBED_EVENT);

    viewer
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Message,
                channel: app::ids::SECURE_CHAT_CHANNEL,
                room: None,
                payload: Some(serde_json::json!({ "body": "hello" })),
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let echoed = viewer.next().await.unwrap().unwrap();
    let echoed: ServerMessage = serde_json::from_str(echoed.to_text().unwrap()).unwrap();
    assert_eq!(echoed.event, app::ids::ECHO_EVENT);
    assert_eq!(echoed.payload["actor_id"], "viewer-1");
    assert_eq!(echoed.payload["body"], "hello");

    server.abort();
}

#[tokio::test]
async fn guarded_websocket_revalidates_cached_credentials_before_actions_and_broadcasts() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_auth_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("auth-ws-revalidation-{websocket_port}"),
    );
    fs::write(
        config_dir.path().join("01-websocket-revalidation.toml"),
        "[websocket]\nauth_revalidation_interval_seconds = 1\n",
    )
    .unwrap();

    let actor = Arc::new(RwLock::new(Some(
        Actor::new("revocable-1", app::ids::AuthGuard::Api)
            .with_permissions([app::ids::Ability::WsChat]),
    )));
    let kernel = App::builder()
        .load_config_dir(config_dir.path())
        .register_provider(RevocableAuthProvider {
            actor: actor.clone(),
        })
        .register_websocket_routes(app::realtime::register)
        .build_websocket_kernel()
        .await
        .unwrap();
    let app_context = kernel.app().clone();
    let server = tokio::spawn(async move { kernel.serve().await.unwrap() });
    let url = format!("ws://127.0.0.1:{websocket_port}/ws");

    let subscribe = || ClientMessage {
        action: ClientAction::Subscribe,
        channel: app::ids::SECURE_CHAT_CHANNEL,
        room: None,
        payload: None,
        event: None,
        ack_id: None,
    };

    let mut permission_revoked = connect_websocket_with_token(&url, Some("revocable-token")).await;
    permission_revoked
        .send(Message::Text(
            serde_json::to_string(&subscribe()).unwrap().into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        next_websocket_message(&mut permission_revoked).await.event,
        SUBSCRIBED_EVENT
    );

    *actor.write().unwrap() = Some(Actor::new("revocable-1", app::ids::AuthGuard::Api));
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let protected_event = ChannelEventId::new("protected-after-permission-revocation");
    app_context
        .websocket()
        .unwrap()
        .publish(
            app::ids::SECURE_CHAT_CHANNEL,
            protected_event.clone(),
            None,
            serde_json::json!({"secret": true}),
        )
        .await
        .unwrap();
    assert_websocket_closes_without_event(&mut permission_revoked, &protected_event).await;

    *actor.write().unwrap() = Some(
        Actor::new("revocable-1", app::ids::AuthGuard::Api)
            .with_permissions([app::ids::Ability::WsChat]),
    );
    let mut token_revoked = connect_websocket_with_token(&url, Some("revocable-token")).await;
    token_revoked
        .send(Message::Text(
            serde_json::to_string(&subscribe()).unwrap().into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        next_websocket_message(&mut token_revoked).await.event,
        SUBSCRIBED_EVENT
    );

    *actor.write().unwrap() = None;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let _ = token_revoked
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Message,
                channel: app::ids::SECURE_CHAT_CHANNEL,
                room: None,
                payload: Some(serde_json::json!({"body": "must not echo"})),
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await;
    assert_websocket_closes_without_event(&mut token_revoked, &app::ids::ECHO_EVENT).await;

    server.abort();
}

#[tokio::test]
async fn websocket_guard_only_channel_rejects_actor_from_different_guard() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_auth_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("auth-ws-guard-mismatch-{websocket_port}"),
    );

    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path());
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let url = format!("ws://127.0.0.1:{websocket_port}/ws");
    let mut socket = connect_websocket_with_token(&url, Some("api-token-for-admin-guard")).await;
    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::ADMIN_ONLY_CHANNEL,
                room: None,
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

    let error = next_websocket_message(&mut socket).await;
    assert_eq!(error.channel, SYSTEM_CHANNEL);
    assert_eq!(error.event, ERROR_EVENT);
    assert_eq!(error.payload["message"], "The bearer token is invalid.");
    assert_eq!(error.payload["error_code"], "invalid_bearer_token");
    assert_eq!(error.payload["message_key"], "auth.invalid_bearer_token");

    server.abort();
}

#[tokio::test]
async fn disconnect_user_closes_authenticated_websocket_connections_and_broadcasts_presence_leave()
{
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_auth_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("auth-ws-disconnect-{websocket_port}"),
    );

    let kernel = build_websocket_app(config_dir.path())
        .build_websocket_kernel()
        .await
        .unwrap();
    let app = kernel.app().clone();
    let server = tokio::spawn(async move { kernel.serve().await.unwrap() });

    let url = format!("ws://127.0.0.1:{websocket_port}/ws");
    let mut observer = connect_websocket_with_token(&url, Some("admin-token")).await;
    let mut target = connect_websocket_with_token(&url, Some("viewer-token")).await;

    observer
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::SECURE_PRESENCE_CHANNEL,
                room: Some("team".to_string()),
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        next_websocket_message(&mut observer).await.event,
        SUBSCRIBED_EVENT
    );

    target
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::SECURE_PRESENCE_CHANNEL,
                room: Some("team".to_string()),
                payload: None,
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

    let observer_join = next_websocket_message(&mut observer).await;
    assert_eq!(observer_join.event, PRESENCE_JOIN_EVENT);
    assert_eq!(observer_join.payload["actor_id"], "viewer-1");
    assert_eq!(
        next_websocket_message(&mut target).await.event,
        SUBSCRIBED_EVENT
    );

    app.websocket()
        .unwrap()
        .disconnect_user("viewer-1")
        .await
        .unwrap();

    let leave = next_websocket_message(&mut observer).await;
    assert_eq!(leave.event, PRESENCE_LEAVE_EVENT);
    assert_eq!(leave.payload["actor_id"], "viewer-1");

    let _ = tokio::time::timeout(Duration::from_secs(2), target.next())
        .await
        .expect("target websocket did not close");

    server.abort();
}
