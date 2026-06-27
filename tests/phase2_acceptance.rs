use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use foundry::prelude::*;
use futures_util::{SinkExt, StreamExt};
use tempfile::tempdir;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        pub const USER_CREATED: EventId = EventId::new("user.created");
        pub const AUDIT_JOB: JobId = JobId::new("audit.job");
        pub const CHAT_CHANNEL: ChannelId = ChannelId::new("chat");
        pub const CLIENT_EVENTS_CHANNEL: ChannelId = ChannelId::new("client_events");
        pub const OPEN_CLIENT_EVENTS_CHANNEL: ChannelId = ChannelId::new("open_client_events");
        pub const FAIL_CHANNEL: ChannelId = ChannelId::new("failures");
        pub const PRESENCE_CHANNEL: ChannelId = ChannelId::new("presence");
        pub const JOIN_PANIC_CHANNEL: ChannelId = ChannelId::new("join_panic");
        pub const LEAVE_PANIC_CHANNEL: ChannelId = ChannelId::new("leave_panic");
        pub const ECHO_EVENT: ChannelEventId = ChannelEventId::new("echo");
        pub const HTTP_NOTICE_EVENT: ChannelEventId = ChannelEventId::new("http_notice");
        pub const TYPING_EVENT: ChannelEventId = ChannelEventId::new("typing");
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

            async fn handle(&self, context: JobContext) -> Result<()> {
                let log = context.app().resolve::<Mutex<Vec<String>>>()?;
                log.lock().unwrap().push(format!("job:{}", self.marker));
                Ok(())
            }

            fn backoff(&self, _attempt: u32) -> Duration {
                Duration::from_millis(10)
            }
        }
    }

    pub mod providers {
        use super::*;

        #[derive(Clone)]
        pub struct AppServiceProvider {
            pub log: Arc<Mutex<Vec<String>>>,
            pub spawn_worker: bool,
        }

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.singleton_arc(self.log.clone())?;
                registrar.listen_event::<domain::UserCreated, _>(dispatch_job(
                    |event: &domain::UserCreated| domain::AuditJob {
                        marker: format!("event:{}", event.email),
                    },
                ))?;
                registrar.register_job::<domain::AuditJob>()?;
                Ok(())
            }

            async fn boot(&self, app: &AppContext) -> Result<()> {
                self.log.lock().unwrap().push("provider:boot".to_string());
                if self.spawn_worker {
                    spawn_worker(app.clone())?;
                }
                Ok(())
            }
        }
    }

    pub mod http {
        use super::*;

        pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route("/dispatch", post(dispatch_job_and_publish));
            registrar.route("/events", post(dispatch_event));
            registrar.route("/health", get(health));
            Ok(())
        }

        async fn dispatch_job_and_publish(State(app): State<AppContext>) -> impl IntoResponse {
            app.jobs()
                .unwrap()
                .dispatch(domain::AuditJob {
                    marker: "http".to_string(),
                })
                .await
                .unwrap();
            app.websocket()
                .unwrap()
                .publish(
                    ids::CHAT_CHANNEL,
                    ids::HTTP_NOTICE_EVENT,
                    None,
                    serde_json::json!({ "source": "http" }),
                )
                .await
                .unwrap();
            StatusCode::ACCEPTED
        }

        async fn dispatch_event(State(app): State<AppContext>) -> impl IntoResponse {
            app.events()
                .unwrap()
                .dispatch(domain::UserCreated {
                    email: "foundry@example.com".to_string(),
                })
                .await
                .unwrap();
            StatusCode::ACCEPTED
        }

        async fn health(State(app): State<AppContext>) -> impl IntoResponse {
            let log = app.resolve::<Mutex<Vec<String>>>().unwrap();
            Json(serde_json::json!({
                "entries": log.lock().unwrap().clone(),
            }))
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
                    .server_events([ids::ECHO_EVENT, ids::HTTP_NOTICE_EVENT]),
            )?;
            registrar.channel_with_options(
                ids::CLIENT_EVENTS_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move { Ok(()) },
                WebSocketChannelOptions::new().client_event(ids::TYPING_EVENT),
            )?;
            registrar.channel_with_options(
                ids::OPEN_CLIENT_EVENTS_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move { Ok(()) },
                WebSocketChannelOptions::new().allow_client_events(true),
            )?;
            registrar.channel(
                ids::FAIL_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move {
                    Err(Error::message("handler failed"))
                },
            )?;
            registrar.channel_with_options(
                ids::PRESENCE_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move { Ok(()) },
                WebSocketChannelOptions::new()
                    .presence(true)
                    .on_leave(|context| async move {
                        let log = context.app().resolve::<Mutex<Vec<String>>>()?;
                        let room = context.room().unwrap_or("all").to_string();
                        log.lock().unwrap().push(format!("presence:left:{room}"));
                        Ok(())
                    }),
            )?;
            registrar.channel_with_options(
                ids::JOIN_PANIC_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move { Ok(()) },
                WebSocketChannelOptions::new().on_join(|_context| async {
                    if std::hint::black_box(true) {
                        panic!("join hook exploded");
                    }
                    Ok(())
                }),
            )?;
            registrar.channel_with_options(
                ids::LEAVE_PANIC_CHANNEL,
                |_context: WebSocketContext, _payload: serde_json::Value| async move { Ok(()) },
                WebSocketChannelOptions::new().on_leave(|_context| async {
                    if std::hint::black_box(true) {
                        panic!("leave hook exploded");
                    }
                    Ok(())
                }),
            )?;
            Ok(())
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn write_phase2_config(dir: &Path, server_port: u16, websocket_port: u16, namespace: &str) {
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

            [jobs]
            queue = "default"
            max_retries = 3
            poll_interval_ms = 20
        "#
        ),
    )
    .unwrap();
}

fn build_http_app(
    config_dir: &Path,
    log: Arc<Mutex<Vec<String>>>,
    spawn_worker: bool,
) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::AppServiceProvider { log, spawn_worker })
        .register_routes(app::http::router)
}

fn build_websocket_app(
    config_dir: &Path,
    log: Arc<Mutex<Vec<String>>>,
    spawn_worker: bool,
) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::AppServiceProvider { log, spawn_worker })
        .register_websocket_routes(app::realtime::register)
}

async fn wait_for_http_ready(base_url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..40 {
        if client
            .get(format!("{base_url}/health"))
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

async fn wait_for_log(log: &Arc<Mutex<Vec<String>>>, expected: &str) {
    for _ in 0..40 {
        if log.lock().unwrap().iter().any(|entry| entry == expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("log entry `{expected}` not observed");
}

async fn connect_websocket(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    for _ in 0..40 {
        if let Ok((socket, _)) = connect_async(url).await {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("websocket server did not become ready");
}

type TestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send_ws_json(socket: &mut TestSocket, value: serde_json::Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn next_ws_message(socket: &mut TestSocket) -> ServerMessage {
    let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("timed out waiting for websocket frame")
        .unwrap()
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

async fn expect_no_ws_message(socket: &mut TestSocket) {
    let next = tokio::time::timeout(Duration::from_millis(200), socket.next()).await;
    assert!(next.is_err(), "unexpected websocket frame: {next:?}");
}

#[tokio::test]
async fn websocket_kernel_handles_subscribe_message_and_unsubscribe() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-ws-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let mut socket = connect_websocket(&format!("ws://127.0.0.1:{websocket_port}/ws")).await;
    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::CHAT_CHANNEL,
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

    let subscribed = socket.next().await.unwrap().unwrap();
    let subscribed: ServerMessage = serde_json::from_str(subscribed.to_text().unwrap()).unwrap();
    assert_eq!(subscribed.event, SUBSCRIBED_EVENT);

    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Message,
                channel: app::ids::CHAT_CHANNEL,
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

    let echoed = socket.next().await.unwrap().unwrap();
    let echoed: ServerMessage = serde_json::from_str(echoed.to_text().unwrap()).unwrap();
    assert_eq!(echoed.event, app::ids::ECHO_EVENT);
    assert_eq!(echoed.payload["body"], "hello");

    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Unsubscribe,
                channel: app::ids::CHAT_CHANNEL,
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

    let unsubscribed = socket.next().await.unwrap().unwrap();
    let unsubscribed: ServerMessage =
        serde_json::from_str(unsubscribed.to_text().unwrap()).unwrap();
    assert_eq!(unsubscribed.event, UNSUBSCRIBED_EVENT);

    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Message,
                channel: app::ids::CHAT_CHANNEL,
                room: None,
                payload: Some(serde_json::json!({ "body": "ignored" })),
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

    let error = socket.next().await.unwrap().unwrap();
    let error: ServerMessage = serde_json::from_str(error.to_text().unwrap()).unwrap();
    assert_eq!(error.channel, SYSTEM_CHANNEL);
    assert_eq!(error.event, ERROR_EVENT);
    assert!(error.payload["message"]
        .as_str()
        .unwrap()
        .contains("not subscribed"));

    server.abort();
}

#[tokio::test]
async fn websocket_protocol_accepts_raw_snake_case_and_legacy_pascal_case_actions() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-raw-protocol-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let mut socket = connect_websocket(&format!("ws://127.0.0.1:{websocket_port}/ws")).await;
    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "subscribe", "channel": "chat" }),
    )
    .await;
    let subscribed = next_ws_message(&mut socket).await;
    assert_eq!(subscribed.event, SUBSCRIBED_EVENT);

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "Unsubscribe", "channel": "chat" }),
    )
    .await;
    let unsubscribed = next_ws_message(&mut socket).await;
    assert_eq!(unsubscribed.event, UNSUBSCRIBED_EVENT);

    server.abort();
}

#[tokio::test]
async fn websocket_protocol_reports_malformed_json_and_unknown_channels() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-protocol-errors-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let mut socket = connect_websocket(&format!("ws://127.0.0.1:{websocket_port}/ws")).await;
    socket.send(Message::Text("not json".into())).await.unwrap();
    let malformed = next_ws_message(&mut socket).await;
    assert_eq!(malformed.channel, SYSTEM_CHANNEL);
    assert_eq!(malformed.event, ERROR_EVENT);

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "subscribe", "channel": "ghost" }),
    )
    .await;
    let unknown = next_ws_message(&mut socket).await;
    assert_eq!(unknown.channel, SYSTEM_CHANNEL);
    assert_eq!(unknown.event, ERROR_EVENT);
    assert!(unknown.payload["message"]
        .as_str()
        .unwrap()
        .contains("not registered"));

    server.abort();
}

#[tokio::test]
async fn websocket_messages_require_subscription_and_ack_success_or_error() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-ack-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let mut socket = connect_websocket(&format!("ws://127.0.0.1:{websocket_port}/ws")).await;
    send_ws_json(
        &mut socket,
        serde_json::json!({
            "action": "message",
            "channel": "chat",
            "payload": { "body": "blocked" }
        }),
    )
    .await;
    let blocked = next_ws_message(&mut socket).await;
    assert_eq!(blocked.event, ERROR_EVENT);
    assert!(blocked.payload["message"]
        .as_str()
        .unwrap()
        .contains("not subscribed"));

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "subscribe", "channel": "chat" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut socket).await.event, SUBSCRIBED_EVENT);

    send_ws_json(
        &mut socket,
        serde_json::json!({
            "action": "message",
            "channel": "chat",
            "payload": { "body": "hello" },
            "ack_id": "ack-ok"
        }),
    )
    .await;
    let first = next_ws_message(&mut socket).await;
    let second = next_ws_message(&mut socket).await;
    let frames = [first, second];
    assert!(frames
        .iter()
        .any(|message| message.event == app::ids::ECHO_EVENT));
    let ack = frames
        .iter()
        .find(|message| message.event == ACK_EVENT)
        .expect("ack frame");
    assert_eq!(ack.payload["ack_id"], "ack-ok");
    assert_eq!(ack.payload["status"], "ok");

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "subscribe", "channel": "failures" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut socket).await.event, SUBSCRIBED_EVENT);
    send_ws_json(
        &mut socket,
        serde_json::json!({
            "action": "message",
            "channel": "failures",
            "ack_id": "ack-error"
        }),
    )
    .await;
    let failed_ack = next_ws_message(&mut socket).await;
    assert_eq!(failed_ack.event, ACK_EVENT);
    assert_eq!(failed_ack.payload["ack_id"], "ack-error");
    assert_eq!(failed_ack.payload["status"], "error");
    assert_eq!(failed_ack.payload["error"], "handler failed");

    server.abort();
}

#[tokio::test]
async fn websocket_room_routing_distinguishes_channel_wide_and_room_broadcasts() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-rooms-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let kernel = build_websocket_app(config_dir.path(), log.clone(), false)
        .build_websocket_kernel()
        .await
        .unwrap();
    let app = kernel.app().clone();
    let server = tokio::spawn(async move { kernel.serve().await.unwrap() });

    let url = format!("ws://127.0.0.1:{websocket_port}/ws");
    let mut all = connect_websocket(&url).await;
    let mut room = connect_websocket(&url).await;
    send_ws_json(
        &mut all,
        serde_json::json!({ "action": "subscribe", "channel": "chat" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut all).await.event, SUBSCRIBED_EVENT);
    send_ws_json(
        &mut room,
        serde_json::json!({ "action": "subscribe", "channel": "chat", "room": "room:42" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut room).await.event, SUBSCRIBED_EVENT);

    app.websocket()
        .unwrap()
        .publish(
            app::ids::CHAT_CHANNEL,
            app::ids::HTTP_NOTICE_EVENT,
            None,
            serde_json::json!({ "scope": "all" }),
        )
        .await
        .unwrap();
    assert_eq!(next_ws_message(&mut all).await.payload["scope"], "all");
    assert_eq!(next_ws_message(&mut room).await.payload["scope"], "all");

    app.websocket()
        .unwrap()
        .publish(
            app::ids::CHAT_CHANNEL,
            app::ids::HTTP_NOTICE_EVENT,
            Some("room:42"),
            serde_json::json!({ "scope": "room" }),
        )
        .await
        .unwrap();
    let room_frame = next_ws_message(&mut room).await;
    assert_eq!(room_frame.room.as_deref(), Some("room:42"));
    assert_eq!(room_frame.payload["scope"], "room");
    expect_no_ws_message(&mut all).await;

    server.abort();
}

#[tokio::test]
async fn websocket_publisher_rejects_undocumented_server_events() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-server-events-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let kernel = build_websocket_app(config_dir.path(), log, false)
        .build_websocket_kernel()
        .await
        .unwrap();
    let publisher = kernel.app().websocket().unwrap();

    publisher
        .publish(
            app::ids::FAIL_CHANNEL,
            ChannelEventId::new("undocumented"),
            None,
            serde_json::json!({ "compatible": true }),
        )
        .await
        .unwrap();

    let error = publisher
        .publish(
            app::ids::CHAT_CHANNEL,
            ChannelEventId::new("undocumented"),
            None,
            serde_json::json!({ "compatible": false }),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("websocket channel `chat` does not document server event `undocumented`"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn websocket_client_events_require_subscription_and_relay_to_matching_subscribers() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-client-events-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let url = format!("ws://127.0.0.1:{websocket_port}/ws");
    let mut sender = connect_websocket(&url).await;
    let mut receiver = connect_websocket(&url).await;

    send_ws_json(
        &mut sender,
        serde_json::json!({
            "action": "client_event",
            "channel": "client_events",
            "event": "typing"
        }),
    )
    .await;
    let blocked = next_ws_message(&mut sender).await;
    assert_eq!(blocked.event, ERROR_EVENT);

    for socket in [&mut sender, &mut receiver] {
        send_ws_json(
            socket,
            serde_json::json!({ "action": "subscribe", "channel": "client_events" }),
        )
        .await;
        assert_eq!(next_ws_message(socket).await.event, SUBSCRIBED_EVENT);
    }

    send_ws_json(
        &mut sender,
        serde_json::json!({
            "action": "client_event",
            "channel": "client_events",
            "event": "status",
            "payload": { "user": "one" }
        }),
    )
    .await;
    let rejected = next_ws_message(&mut sender).await;
    assert_eq!(rejected.event, ERROR_EVENT);
    assert!(rejected.payload["message"]
        .as_str()
        .unwrap()
        .contains("client event `status` is not allowed"));
    expect_no_ws_message(&mut receiver).await;

    send_ws_json(
        &mut sender,
        serde_json::json!({
            "action": "client_event",
            "channel": "client_events",
            "event": "typing",
            "payload": { "user": "one" }
        }),
    )
    .await;
    let relayed = next_ws_message(&mut receiver).await;
    assert_eq!(relayed.event, app::ids::TYPING_EVENT);
    assert_eq!(relayed.payload["user"], "one");
    expect_no_ws_message(&mut sender).await;

    send_ws_json(
        &mut sender,
        serde_json::json!({ "action": "subscribe", "channel": "open_client_events" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut sender).await.event, SUBSCRIBED_EVENT);

    send_ws_json(
        &mut sender,
        serde_json::json!({
            "action": "client_event",
            "channel": "open_client_events",
            "event": "subscribed",
            "payload": { "user": "one" }
        }),
    )
    .await;
    let reserved = next_ws_message(&mut sender).await;
    assert_eq!(reserved.event, ERROR_EVENT);
    assert!(reserved.payload["message"]
        .as_str()
        .unwrap()
        .contains("reserved Foundry protocol event"));

    server.abort();
}

#[tokio::test]
async fn websocket_disconnect_runs_presence_leave_and_lifecycle_hooks() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-presence-close-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let url = format!("ws://127.0.0.1:{websocket_port}/ws");
    let mut observer = connect_websocket(&url).await;
    let mut closing = connect_websocket(&url).await;
    send_ws_json(
        &mut observer,
        serde_json::json!({ "action": "subscribe", "channel": "presence", "room": "team" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut observer).await.event, SUBSCRIBED_EVENT);
    send_ws_json(
        &mut closing,
        serde_json::json!({ "action": "subscribe", "channel": "presence", "room": "team" }),
    )
    .await;
    assert_eq!(
        next_ws_message(&mut observer).await.event,
        PRESENCE_JOIN_EVENT
    );
    assert_eq!(next_ws_message(&mut closing).await.event, SUBSCRIBED_EVENT);

    closing.close(None).await.unwrap();
    let leave = next_ws_message(&mut observer).await;
    assert_eq!(leave.event, PRESENCE_LEAVE_EVENT);
    assert_eq!(leave.room.as_deref(), Some("team"));
    wait_for_log(&log, "presence:left:team").await;

    server.abort();
}

#[tokio::test]
async fn websocket_lifecycle_hook_panics_do_not_break_subscription_flow() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-lifecycle-panic-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let url = format!("ws://127.0.0.1:{websocket_port}/ws");
    let mut socket = connect_websocket(&url).await;

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "subscribe", "channel": "join_panic" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut socket).await.event, SUBSCRIBED_EVENT);

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "subscribe", "channel": "leave_panic" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut socket).await.event, SUBSCRIBED_EVENT);

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "unsubscribe", "channel": "leave_panic" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut socket).await.event, UNSUBSCRIBED_EVENT);

    send_ws_json(
        &mut socket,
        serde_json::json!({ "action": "subscribe", "channel": "chat" }),
    )
    .await;
    assert_eq!(next_ws_message(&mut socket).await.event, SUBSCRIBED_EVENT);

    server.abort();
}

#[tokio::test]
async fn http_handler_dispatches_job_and_publishes_to_websocket() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-http-ws-{server_port}-{websocket_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let http_server = tokio::spawn({
        let builder = build_http_app(config_dir.path(), log.clone(), true);
        async move { builder.run_http_async().await.unwrap() }
    });
    let websocket_server = tokio::spawn({
        let builder = build_websocket_app(config_dir.path(), log.clone(), false);
        async move { builder.run_websocket_async().await.unwrap() }
    });

    let base_url = format!("http://127.0.0.1:{server_port}");
    wait_for_http_ready(&base_url).await;

    let mut socket = connect_websocket(&format!("ws://127.0.0.1:{websocket_port}/ws")).await;
    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: app::ids::CHAT_CHANNEL,
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
    let _ = socket.next().await.unwrap().unwrap();

    reqwest::Client::new()
        .post(format!("{base_url}/dispatch"))
        .send()
        .await
        .unwrap();

    let pushed = socket.next().await.unwrap().unwrap();
    let pushed: ServerMessage = serde_json::from_str(pushed.to_text().unwrap()).unwrap();
    assert_eq!(pushed.event, app::ids::HTTP_NOTICE_EVENT);
    assert_eq!(pushed.payload["source"], "http");

    wait_for_log(&log, "job:http").await;

    http_server.abort();
    websocket_server.abort();
}

#[tokio::test]
async fn provider_registered_event_listener_dispatches_a_queued_job() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_phase2_config(
        config_dir.path(),
        server_port,
        websocket_port,
        &format!("phase2-events-{server_port}"),
    );

    let log = Arc::new(Mutex::new(Vec::new()));
    let http_server = tokio::spawn({
        let builder = build_http_app(config_dir.path(), log.clone(), true);
        async move { builder.run_http_async().await.unwrap() }
    });

    let base_url = format!("http://127.0.0.1:{server_port}");
    wait_for_http_ready(&base_url).await;

    reqwest::Client::new()
        .post(format!("{base_url}/events"))
        .send()
        .await
        .unwrap();

    wait_for_log(&log, "job:event:foundry@example.com").await;

    http_server.abort();
}
