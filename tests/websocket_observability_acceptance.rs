use axum::http::StatusCode;
use foundry::notifications::{
    Notifiable, Notification, NOTIFICATION_BROADCAST_CHANNEL, NOTIFICATION_BROADCAST_EVENT,
    NOTIFY_BROADCAST,
};
use foundry::support::{ChannelEventId, ChannelId, GuardId, PermissionId};
use foundry::testing::TestApp;
use foundry::websocket::{WebSocketChannelOptions, WebSocketRegistrar};
use foundry::App;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct WebSocketChannelsContract {
    channels: Vec<WebSocketChannelConfigContract>,
}

#[derive(Debug, Deserialize)]
struct WebSocketChannelConfigContract {
    id: String,
    presence: bool,
    replay_count: u32,
    allow_client_events: bool,
    client_events: Vec<String>,
    server_events: Vec<String>,
    requires_auth: bool,
    guard: Option<String>,
    permissions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WebSocketPresenceContract {
    channel: String,
    count: usize,
    members: Vec<WebSocketPresenceMemberContract>,
}

#[derive(Debug, Deserialize)]
struct WebSocketPresenceMemberContract {
    actor_id: String,
    joined_at: i64,
}

#[derive(Debug, Deserialize)]
struct WebSocketHistoryContract {
    channel: String,
    messages: Vec<WebSocketHistoryMessageContract>,
}

#[derive(Debug, Deserialize)]
struct WebSocketHistoryMessageContract {
    channel: String,
    event: String,
    room: Option<String>,
    payload: Option<Value>,
    payload_size_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WebSocketStatsContract {
    global: WebSocketGlobalStatsContract,
    channels: Vec<WebSocketChannelStatsContract>,
}

struct BroadcastUser {
    id: &'static str,
}

impl Notifiable for BroadcastUser {
    fn notification_id(&self) -> String {
        self.id.to_string()
    }
}

struct BroadcastNotice;

impl Notification for BroadcastNotice {
    fn notification_type(&self) -> &str {
        "test.broadcast"
    }

    fn via(&self) -> Vec<foundry::support::NotificationChannelId> {
        vec![NOTIFY_BROADCAST]
    }

    fn to_broadcast(&self) -> Option<Value> {
        Some(serde_json::json!({ "message": "hello" }))
    }
}

fn register_notification_broadcast_channel(
    registrar: &mut WebSocketRegistrar,
) -> foundry::Result<()> {
    registrar.channel_with_options(
        NOTIFICATION_BROADCAST_CHANNEL,
        |_ctx, _payload| async { Ok(()) },
        WebSocketChannelOptions::new().server_event(NOTIFICATION_BROADCAST_EVENT),
    )?;
    Ok(())
}

async fn notification_history(app: &TestApp) -> WebSocketHistoryContract {
    let response = app
        .client()
        .get(&format!(
            "/_foundry/ws/history/{}",
            NOTIFICATION_BROADCAST_CHANNEL
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().unwrap()
}

fn assert_notification_broadcast_message(body: &WebSocketHistoryContract, room: &str) {
    assert_eq!(body.channel, NOTIFICATION_BROADCAST_CHANNEL.as_str());
    assert_eq!(body.messages.len(), 1);
    assert_eq!(
        body.messages[0].channel,
        NOTIFICATION_BROADCAST_CHANNEL.as_str()
    );
    assert_eq!(
        body.messages[0].event,
        NOTIFICATION_BROADCAST_EVENT.as_str()
    );
    assert_eq!(body.messages[0].room.as_deref(), Some(room));
    assert_eq!(
        body.messages[0].payload.as_ref().unwrap()["notification_type"],
        "test.broadcast"
    );
    assert_eq!(
        body.messages[0].payload.as_ref().unwrap()["data"]["message"],
        "hello"
    );
}

#[derive(Debug, Deserialize)]
struct WebSocketGlobalStatsContract {
    active_connections: u64,
    active_subscriptions: u64,
    subscriptions_total: u64,
    unsubscribes_total: u64,
    inbound_messages_total: u64,
    outbound_messages_total: u64,
    opened_total: u64,
    closed_total: u64,
}

#[derive(Debug, Deserialize)]
struct WebSocketChannelStatsContract {
    id: String,
    subscriptions_total: u64,
    unsubscribes_total: u64,
    active_subscriptions: u64,
    inbound_messages_total: u64,
    outbound_messages_total: u64,
}

#[tokio::test]
async fn ws_presence_endpoint_returns_members_for_presence_channel() {
    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|r| {
            r.channel_with_options(
                ChannelId::new("team"),
                |_ctx, _payload| async { Ok(()) },
                WebSocketChannelOptions::new().presence(true),
            )?;
            r.channel(ChannelId::new("public"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    app.seed_presence(&ChannelId::new("team"), "user_1", 1_713_000_000)
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/presence/team")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: WebSocketPresenceContract = response.json().unwrap();
    assert_eq!(body.channel, "team");
    assert_eq!(body.count, 1);
    assert_eq!(body.members.len(), 1);
    assert_eq!(body.members[0].actor_id, "user_1");
    assert_eq!(body.members[0].joined_at, 1_713_000_000);
}

#[tokio::test]
async fn ws_presence_endpoint_returns_404_for_non_presence_channel() {
    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("public"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/presence/public")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let body: Value = response.json().unwrap();
    assert_eq!(body["message"], "presence not enabled for channel");
    assert_eq!(body["status"], 404);
    assert!(body.get("error").is_none());
}

#[tokio::test]
async fn ws_presence_endpoint_returns_404_for_unregistered_channel() {
    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|_r| Ok(()))
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/presence/ghost")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let body: Value = response.json().unwrap();
    assert_eq!(body["message"], "channel not registered");
    assert_eq!(body["status"], 404);
    assert!(body.get("error").is_none());
}

#[tokio::test]
async fn ws_channels_endpoint_lists_registered_channels() {
    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|r| {
            r.channel_with_options(
                ChannelId::new("chat"),
                |_ctx, _payload| async { Ok(()) },
                WebSocketChannelOptions::new()
                    .presence(true)
                    .replay(10)
                    .client_event(ChannelEventId::new("typing"))
                    .server_event(ChannelEventId::new("message"))
                    .guard(GuardId::new("api"))
                    .permissions([PermissionId::new("chat:read")]),
            )?;
            r.channel(ChannelId::new("public"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/channels")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: WebSocketChannelsContract = response.json().unwrap();
    assert_eq!(body.channels.len(), 2);

    let chat = body
        .channels
        .iter()
        .find(|channel| channel.id == "chat")
        .expect("chat present");
    assert!(chat.presence);
    assert_eq!(chat.replay_count, 10);
    assert!(chat.allow_client_events);
    assert_eq!(chat.client_events, vec!["typing"]);
    assert_eq!(chat.server_events, vec!["message"]);
    assert!(chat.requires_auth);
    assert_eq!(chat.guard.as_deref(), Some("api"));
    assert_eq!(chat.permissions, vec!["chat:read"]);

    let public = body
        .channels
        .iter()
        .find(|channel| channel.id == "public")
        .expect("public present");
    assert!(!public.presence);
    assert!(public.client_events.is_empty());
    assert!(public.server_events.is_empty());
    assert!(!public.requires_auth);
}

#[tokio::test]
async fn ws_history_redacts_payloads_by_default() {
    use foundry::support::ChannelEventId;

    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("history-redact"), |_ctx, _payload| async {
                Ok(())
            })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    let publisher = app.app().websocket().unwrap();
    publisher
        .publish(
            ChannelId::new("history-redact"),
            ChannelEventId::new("created"),
            None,
            serde_json::json!({ "secret": "hello world" }),
        )
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/history/history-redact")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: WebSocketHistoryContract = response.json().unwrap();
    assert_eq!(body.channel, "history-redact");
    assert_eq!(body.messages.len(), 1);
    let message = &body.messages[0];
    assert_eq!(message.channel, "history-redact");
    assert_eq!(message.event, "created");
    assert_eq!(message.room, None);
    assert!(
        message.payload.is_none(),
        "payload must be redacted by default"
    );
    assert!(message.payload_size_bytes.unwrap() > 0);
}

#[tokio::test]
async fn ws_history_returns_payloads_when_flag_is_set() {
    use foundry::support::ChannelEventId;

    // Write a temp config dir with include_payloads = true.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("00-observability.toml"),
        r#"
[observability.websocket]
include_payloads = true
"#,
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(tmp.path())
        .enable_observability()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("history-full"), |_ctx, _payload| async {
                Ok(())
            })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    let publisher = app.app().websocket().unwrap();
    publisher
        .publish(
            ChannelId::new("history-full"),
            ChannelEventId::new("created"),
            None,
            serde_json::json!({ "secret": "hello world" }),
        )
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/history/history-full")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: WebSocketHistoryContract = response.json().unwrap();
    assert_eq!(body.channel, "history-full");
    assert_eq!(
        body.messages[0].payload.as_ref().unwrap()["secret"],
        "hello world"
    );
    assert_eq!(body.messages[0].payload_size_bytes, None);
}

#[tokio::test]
async fn broadcast_notifications_publish_to_canonical_channel_room() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("00-observability.toml"),
        r#"
[observability.websocket]
include_payloads = true
"#,
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(tmp.path())
        .enable_observability()
        .register_websocket_routes(register_notification_broadcast_channel)
        .build()
        .await
        .unwrap();

    app.app()
        .notify(&BroadcastUser { id: "user_42" }, &BroadcastNotice)
        .await
        .unwrap();

    let body = notification_history(&app).await;
    assert_notification_broadcast_message(&body, "user_42");
}

#[tokio::test]
async fn queued_broadcast_notifications_publish_to_canonical_channel_room() {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = format!(
        "queued-notification-broadcast-{}",
        tmp.path().file_name().unwrap().to_string_lossy()
    );
    std::fs::write(
        tmp.path().join("00-runtime.toml"),
        format!(
            r#"
[redis]
namespace = "{namespace}"

[jobs]
poll_interval_ms = 10

[observability.websocket]
include_payloads = true
"#
        ),
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(tmp.path())
        .enable_observability()
        .register_websocket_routes(register_notification_broadcast_channel)
        .build()
        .await
        .unwrap();
    let worker = App::builder()
        .load_config_dir(tmp.path())
        .register_websocket_routes(register_notification_broadcast_channel)
        .build_worker_kernel()
        .await
        .unwrap();

    app.app()
        .notify_queued(&BroadcastUser { id: "user_queued" }, &BroadcastNotice)
        .await
        .unwrap();

    assert!(worker.run_once().await.unwrap());

    let body = notification_history(&app).await;
    assert_notification_broadcast_message(&body, "user_queued");
}

#[tokio::test]
async fn ws_history_returns_404_for_unregistered_channel() {
    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|_r| Ok(()))
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/history/ghost")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let body: Value = response.json().unwrap();
    assert_eq!(body["message"], "channel not registered");
    assert_eq!(body["status"], 404);
    assert!(body.get("error").is_none());
}

#[tokio::test]
async fn ws_history_clamps_limit_to_buffer_size() {
    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("events"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/history/events?limit=999")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn ws_stats_exposes_global_and_per_channel_counters() {
    let app = TestApp::builder()
        .enable_observability()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("alpha"), |_ctx, _payload| async { Ok(()) })?;
            r.channel(ChannelId::new("idle"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    // Drive traffic via the diagnostics API directly.
    let diagnostics = app.app().diagnostics().unwrap();
    diagnostics.record_websocket_subscription_opened_on(&ChannelId::new("alpha"));
    diagnostics.record_websocket_subscription_closed_on(&ChannelId::new("alpha"));
    diagnostics.record_websocket_inbound_message_on(&ChannelId::new("alpha"));
    diagnostics.record_websocket_outbound_message_on(&ChannelId::new("alpha"));
    diagnostics.record_websocket_outbound_message_on(&ChannelId::new("alpha"));

    let response = app.client().get("/_foundry/ws/stats").send().await.unwrap();
    assert_eq!(response.status(), 200);
    let body: WebSocketStatsContract = response.json().unwrap();

    assert_eq!(body.global.active_connections, 0);
    assert_eq!(body.global.subscriptions_total, 1);
    assert_eq!(body.global.unsubscribes_total, 1);
    assert_eq!(body.global.active_subscriptions, 0);
    assert_eq!(body.global.inbound_messages_total, 1);
    assert_eq!(body.global.outbound_messages_total, 2);
    assert_eq!(body.global.opened_total, 0);
    assert_eq!(body.global.closed_total, 0);

    assert_eq!(
        body.channels.len(),
        2,
        "registered-but-idle channels appear too"
    );

    let alpha = body
        .channels
        .iter()
        .find(|channel| channel.id == "alpha")
        .unwrap();
    assert_eq!(alpha.subscriptions_total, 1);
    assert_eq!(alpha.unsubscribes_total, 1);
    assert_eq!(alpha.active_subscriptions, 0);
    assert_eq!(alpha.inbound_messages_total, 1);
    assert_eq!(alpha.outbound_messages_total, 2);

    let idle = body
        .channels
        .iter()
        .find(|channel| channel.id == "idle")
        .unwrap();
    assert_eq!(idle.subscriptions_total, 0);
    assert_eq!(idle.unsubscribes_total, 0);
    assert_eq!(idle.outbound_messages_total, 0);
}

#[tokio::test]
async fn publish_sets_history_ttl_by_default() {
    use foundry::support::ChannelEventId;

    let app = TestApp::builder()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("ttl-default"), |_ctx, _payload| async {
                Ok(())
            })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    assert_eq!(
        app.history_ttl(&ChannelId::new("ttl-default"))
            .await
            .unwrap(),
        None,
        "no TTL before first publish",
    );

    app.app()
        .websocket()
        .unwrap()
        .publish(
            ChannelId::new("ttl-default"),
            ChannelEventId::new("created"),
            None,
            serde_json::json!({}),
        )
        .await
        .unwrap();

    assert_eq!(
        app.history_ttl(&ChannelId::new("ttl-default"))
            .await
            .unwrap(),
        Some(604_800),
        "publish applies the default 7-day history TTL",
    );
}

#[tokio::test]
async fn publish_skips_ttl_when_configured_to_zero() {
    use foundry::support::ChannelEventId;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("00-websocket.toml"),
        r#"
[websocket]
history_ttl_seconds = 0
"#,
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(tmp.path())
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("ttl-disabled"), |_ctx, _payload| async {
                Ok(())
            })?;
            Ok(())
        })
        .build()
        .await
        .unwrap();

    app.app()
        .websocket()
        .unwrap()
        .publish(
            ChannelId::new("ttl-disabled"),
            ChannelEventId::new("created"),
            None,
            serde_json::json!({}),
        )
        .await
        .unwrap();

    assert_eq!(
        app.history_ttl(&ChannelId::new("ttl-disabled"))
            .await
            .unwrap(),
        None,
        "history_ttl_seconds = 0 disables expire()",
    );
}
