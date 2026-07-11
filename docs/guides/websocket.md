# WebSocket Guide

Channel-based real-time communication with presence tracking, rooms, auth, and server-side broadcasting.

---

## Quick Start

```rust
const CHAT: ChannelId = ChannelId::new("chat");
const MESSAGE: ChannelEventId = ChannelEventId::new("message");

#[derive(serde::Deserialize, foundry::ApiSchema)]
struct ChatMessage { text: String }

fn ws_routes(r: &mut WebSocketRegistrar) -> Result<()> {
    r.typed_channel::<ChatMessage, _>(CHAT, |ctx, payload| async move {
        ctx.publish(MESSAGE, serde_json::json!({ "text": payload.text })).await
    })?;
    Ok(())
}
```

Register and run:

```rust
App::builder()
    .register_websocket_routes(ws_routes)
    .run_websocket()?;
```

Clients connect to `ws://host:3010/ws` and subscribe to channels by sending:

```json
{ "action": "subscribe", "channel": "chat" }
```

---

## Channels

### Basic Channel

```rust
r.typed_channel::<ChatMessage, _>(ChannelId::new("chat"), |ctx, payload| async move {
    ctx.publish(MESSAGE, serde_json::json!({ "text": payload.text })).await
})?;
```

`typed_channel::<Payload, _>` deserializes each inbound `message` payload before
the handler runs and records the payload schema in the realtime contract.
Invalid payloads receive a 422 acknowledgement. Use `raw_channel` when a
channel deliberately owns dynamic `serde_json::Value`; legacy `channel` remains
an alias for the raw form.

For per-user notification broadcasts, use `register_notification_websocket_channel` rather than a
public basic channel; it enforces guard and room ownership checks.

### Channel with Options

```rust
#[derive(serde::Deserialize, foundry::ApiSchema)]
struct OrderMessage { order_id: String }

r.typed_channel_with_options::<OrderMessage, _>(
    ChannelId::new("orders"),
    |ctx, payload| async move {
        tracing::info!(order_id = %payload.order_id, "received order message");
        Ok(())
    },
    WebSocketChannelOptions::new()
        .guard(Guard::User)                         // require auth
        .permission(Permission::OrdersView)          // require permission
        .presence(true)                              // track who's connected
        .allow_client_events(true)                   // clients can relay to other clients
        .replay(10)                                  // send last 10 messages to new subscribers
        .authorize(|ctx, channel, room| async move {
            // Dynamic auth — e.g., check if user owns this order
            Ok(())
        })
        .on_join(|ctx| async move {
            tracing::info!(user = ?ctx.actor(), "joined orders channel");
            Ok(())
        })
        .on_leave(|ctx| async move {
            tracing::info!(user = ?ctx.actor(), "left orders channel");
            Ok(())
        }),
)?;
```

### Channel Options

| Method | What it does |
|--------|-------------|
| `.guard(Guard::User)` | Require auth guard for subscription |
| `.permission(Permission::X)` | Require specific permission |
| `.permissions([...])` | Require all listed permissions |
| `.authorize(async fn)` | Custom async auth check after guard/permission |
| `.presence(true)` | Enable join/leave tracking |
| `.allow_client_events(true)` | Allow clients to relay events to other clients |
| `.replay(N)` | Buffer last N messages, send to new subscribers |
| `.on_join(async fn)` | Callback when a user subscribes |
| `.on_leave(async fn)` | Callback when a user unsubscribes |

`authorize`, `on_join`, and `on_leave` receive owned context values, so async closures can safely move the context, channel, and room into the future.

---

## Handling Messages

### ChannelHandler Trait

```rust
struct OrderHandler;

#[async_trait]
impl ChannelHandler for OrderHandler {
    async fn handle(&self, ctx: WebSocketContext, payload: Value) -> Result<()> {
        let user = ctx.resolve_actor::<User>().await?
            .ok_or_else(|| Error::message("user not found"))?;

        let order_id = payload.get("order_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing order_id"))?;

        // Process the message...

        // Broadcast response to all subscribers
        ctx.publish(ChannelEventId::new("order_updated"), json!({
            "order_id": order_id,
            "updated_by": user.name,
        })).await
    }
}
```

### WebSocketContext

Available in every handler:

```rust
ctx.app()                  // → &AppContext (full framework access)
ctx.connection_id()        // → u64 (unique per connection)
ctx.actor()                // → Option<&Actor> (authenticated user)
ctx.channel()              // → &ChannelId
ctx.room()                 // → Option<&str>

// Resolve to database model
let user = ctx.resolve_actor::<User>().await?;

// Publish to this channel
ctx.publish(EVENT_ID, json!({ "data": "value" })).await?;

// Presence
let members = ctx.presence_members().await?;  // current channel + room
let count = ctx.presence_count().await?;      // current channel + room
```

---

## Rooms

Rooms are subdivisions within a channel. A client subscribes to a channel + room combination:

```json
{ "action": "subscribe", "channel": "chat", "room": "room:42" }
```

A channel-wide publish (`room = None`) reaches every subscriber on that channel, including room subscribers. A room publish reaches only subscribers for that exact room; channel-wide subscribers do not receive room-specific messages unless they also subscribe to that room.

Publish to a specific room:

```rust
// Only subscribers in room "room:42" receive this
app.websocket()?.publish(
    ChannelId::new("chat"),
    ChannelEventId::new("message"),
    Some("room:42"),
    json!({ "text": "hello room 42" }),
).await?;
```

Publish to the whole channel (all rooms):

```rust
app.websocket()?.publish(
    ChannelId::new("chat"),
    ChannelEventId::new("announcement"),
    None,  // no room = broadcast to all
    json!({ "text": "server maintenance in 5 minutes" }),
).await?;
```

---

## Presence

Track who's connected to a channel in real time.

Enable on channel:

```rust
WebSocketChannelOptions::new().presence(true)
```

Query in handler:

```rust
async fn handle(&self, ctx: WebSocketContext, _payload: Value) -> Result<()> {
    let members = ctx.presence_members().await?;
    for member in &members {
        // member.actor_id, member.channel, member.room, member.joined_at
    }

    let online_count = ctx.presence_count().await?;

    // Broadcast current member list
    ctx.publish(ChannelEventId::new("presence_update"), json!({
        "members": members.iter().map(|m| &m.actor_id).collect::<Vec<_>>(),
        "count": online_count,
    })).await
}
```

**Automatic events** (sent by the framework, not your handler):

| Event | When | Payload |
|-------|------|---------|
| `presence:join` | User subscribes to channel | `{ "actor_id": "..." }` |
| `presence:leave` | User unsubscribes or disconnects | `{ "actor_id": "..." }` |

---

## Broadcasting from HTTP Handlers / Jobs

Publish WebSocket messages from anywhere — not just inside channel handlers:

```rust
// In an HTTP handler
async fn update_order(
    State(app): State<AppContext>,
    Path(order_id): Path<String>,
) -> Result<impl IntoResponse> {
    // ... update order in database ...

    // Broadcast to WebSocket subscribers
    app.websocket()?.publish(
        ChannelId::new("orders"),
        ChannelEventId::new("updated"),
        Some(&format!("order:{order_id}")),
        json!({ "order_id": order_id, "status": "shipped" }),
    ).await?;

    Ok(Json(json!({ "ok": true })))
}

// In a background job
impl Job for ProcessOrderJob {
    async fn handle(&self, ctx: JobContext) -> Result<()> {
        // ... process order ...

        ctx.app().websocket()?.publish(
            ChannelId::new("orders"),
            ChannelEventId::new("processed"),
            None,
            json!({ "order_id": self.order_id }),
        ).await
    }
}
```

### Force Disconnect

Kick an actor from one guard's WebSocket connections (e.g., after ban):

```rust
app.websocket()?
    .disconnect_actor(GuardId::new("web"), &user_id)
    .await?;
```

This works across distributed instances via Redis pub/sub and matches the exact
`(guard, actor ID)` pair. Equal actor IDs under another guard remain connected.

### Server Shutdown

Graceful shutdown rejects racing WebSocket handshakes with HTTP 503, sends each
live socket close code `1001` with reason `server shutdown`, then drains hub
registrations, actor tracking, subscriptions, presence state, and `on_leave`
hooks. The existing `app.background_shutdown_timeout_ms` bounds the complete
cleanup; `0` requests immediate cutoff.

---

## Client Protocol

Clients communicate via JSON frames over WebSocket:

Client actions are canonical `snake_case`: `subscribe`, `unsubscribe`, `message`, and `client_event`. Foundry temporarily accepts the older PascalCase spellings shown in early docs (`Subscribe`, `Unsubscribe`, `Message`, `ClientEvent`) for compatibility, but new clients should use `snake_case`.

### Subscribe

```json
{ "action": "subscribe", "channel": "chat" }
{ "action": "subscribe", "channel": "chat", "room": "room:42" }
```

Server responds:

```json
{ "channel": "chat", "event": "subscribed" }
```

### Unsubscribe

```json
{ "action": "unsubscribe", "channel": "chat" }
```

### Send Message

```json
{
    "action": "message",
    "channel": "chat",
    "payload": { "text": "hello" },
    "ack_id": "optional-client-id"
}
```

Clients must subscribe to the exact channel/room before sending `message` or `client_event` frames. A message for `{ "channel": "chat", "room": "room:42" }` requires a matching subscription to that room.

If `ack_id` is provided, server responds with:

```json
{
  "channel": "system",
  "event": "ack",
  "payload": { "ack_id": "optional-client-id", "status": "ok", "error": null }
}
```

### Client Events (peer-to-peer relay)

When `allow_client_events(true)` is set:

```json
{
    "action": "client_event",
    "channel": "chat",
    "event": "typing",
    "payload": { "user": "Alice" }
}
```

Relayed to all other subscribers (not back to sender).

---

## System Events

The framework automatically sends these events:

| Constant | Event | Description |
|----------|-------|-------------|
| `SUBSCRIBED_EVENT` | `subscribed` | Subscription confirmed |
| `UNSUBSCRIBED_EVENT` | `unsubscribed` | Unsubscription confirmed |
| `PRESENCE_JOIN_EVENT` | `presence:join` | User joined (presence channels) |
| `PRESENCE_LEAVE_EVENT` | `presence:leave` | User left (presence channels) |
| `ERROR_EVENT` | `error` | Error occurred |
| `ACK_EVENT` | `ack` | Message acknowledged |

`error` and `ack` are sent on the `system` channel. `subscribed`, `unsubscribed`, `presence:join`, and `presence:leave` are sent on the channel they describe and include the room when relevant.

---

## Config

```toml
# config/websocket.toml
[websocket]
host = "127.0.0.1"
port = 3010
path = "/ws"
heartbeat_interval_seconds = 30       # server pings client
heartbeat_timeout_seconds = 10        # disconnect if no pong
auth_revalidation_interval_seconds = 30 # max cached credential age; minimum 1 second
max_message_size_bytes = 1048576      # inbound message cap; 0 uses transport default
max_frame_size_bytes = 1048576        # inbound frame cap; 0 uses transport default
max_write_buffer_size_bytes = 1048576 # socket write buffer cap; 0 uses transport default
max_messages_per_second = 50          # per-connection flood protection
max_connections_global = 10000        # process-wide connections; 0 = unlimited
max_connections_per_ip = 100          # anonymous connections per resolved IP; 0 = unlimited
max_connections_per_user = 5          # multi-device limit
max_subscriptions_per_connection = 100 # active subscriptions per connection; 0 = unlimited
max_channel_length = 128              # client-supplied channel id bytes
max_room_length = 256                 # client-supplied room id bytes
max_event_length = 128                # client-supplied event id bytes
max_ack_id_length = 128               # client-supplied ack id bytes
outbound_buffer_size = 1024           # queued outbound frames before disconnect
query_token_enabled = true            # allow ?token=... browser bearer auth
query_token_name = "token"            # query parameter name for bearer auth
query_token_max_length = 4096         # decoded query-token bytes; 0 = unlimited
allowed_origins = []                  # exact Origin allow-list; empty allows same-origin in production/staging
history_buffer_size = 50              # recent messages retained per channel
history_ttl_seconds = 604800          # auto-reap idle history after 7 days
```

If `allowed_origins` is empty, Foundry allows same-origin browser handshakes in production/staging and remains permissive outside production-like environments. Same-origin checks compare scheme, host, and effective port (`80` for HTTP and `443` for HTTPS). Forwarded host and protocol headers are used only when the TCP peer matches `[http.trusted_proxy]`; configure that trust when TLS terminates at a reverse proxy. Handshakes without an `Origin` header remain available for non-browser clients. If `allowed_origins` is non-empty, browser handshakes must include an `Origin` header that exactly matches one configured value. Use this with session-cookie authentication to prevent cross-site WebSocket handshakes.

Foundry accepts bearer tokens in the query string by default because browser
WebSocket clients cannot set custom `Authorization` headers. The decoded token
is bounded by `query_token_max_length`, duplicate token params are rejected, and
the parameter name is configurable. Keep these WebSocket tokens short-lived:
Foundry avoids recording query strings in its own request diagnostics, but reverse
proxies and hosting platforms may log full URLs.

WebSocket client IP metadata follows the HTTP trusted-proxy config. Forwarded
IP headers are honored only when `[http.trusted_proxy]` is enabled and the TCP
peer matches a trusted CIDR; otherwise Foundry uses the socket peer IP.

Connection admission is reserved before the WebSocket upgrade. The global cap
protects process capacity, while the per-IP cap limits anonymous sockets using
the resolved client IP. Once a connection authenticates, it leaves the IP
bucket and is limited by the typed `(guard, actor ID)` user bucket, so identical
actor IDs in different guards do not collide.

Guarded connections revalidate bearer tokens and sessions at most every
`auth_revalidation_interval_seconds`. Revoked or expired credentials close the
socket before later protected broadcasts are delivered, and refreshed actors
replace cached roles and permissions. These background checks never extend a
sliding session TTL; only ordinary authenticated activity does.

---

## Event Integration

Automatically broadcast domain events to WebSocket:

```rust
// In ServiceProvider — listen for events and broadcast
registrar.listen_event::<OrderPlaced, _>(
    publish_websocket(|event: &OrderPlaced| ServerMessage {
        channel: ChannelId::new("orders"),
        event: ChannelEventId::new("placed"),
        room: None,
        payload: json!({ "order_id": event.order_id }),
    })
)?;
```

See [Background Processing Guide](background-processing.md) for event→websocket helpers.

---

## Observability

Foundry exposes read-only JSON endpoints under the observability base path (default `/_foundry`) for inspecting WebSocket state from ops tooling or custom admin apps. All endpoints honor the same `ObservabilityOptions` access scope as the rest of the dashboard — gate them behind a guard and permission for production use.

### Endpoints

| Route                                 | Purpose                                      |
| ------------------------------------- | -------------------------------------------- |
| `GET /_foundry/ws/channels`             | List all registered channels and their options |
| `GET /_foundry/ws/presence/:channel`    | Live presence members for a presence channel |
| `GET /_foundry/ws/history/:channel`     | Recent buffered messages (metadata only by default) |
| `GET /_foundry/ws/stats`                | Global + per-channel counters                |

#### Example: list registered channels

```bash
curl -s http://localhost:3000/_foundry/ws/channels | jq
```

```json
{
  "channels": [
    {
      "id": "chat",
      "presence": true,
      "replay_count": 10,
      "allow_client_events": false,
      "requires_auth": true,
      "guard": "api",
      "permissions": ["chat:read"]
    }
  ]
}
```

#### Example: inspect presence

```bash
curl -s http://localhost:3000/_foundry/ws/presence/chat | jq
```

```json
{
  "channel": "chat",
  "count": 3,
  "members": [
    { "actor_id": "user_1", "joined_at": 1713456789 }
  ]
}
```

#### Example: peek recent history (metadata only)

```bash
curl -s "http://localhost:3000/_foundry/ws/history/chat?limit=10" | jq
```

Each entry includes `{ channel, event, room, payload_size_bytes }`. The raw `payload` is **not** included by default.

History lists are capped by `websocket.history_buffer_size` (default 50). Each publish also refreshes a TTL on the history key (default 7 days, configured via `websocket.history_ttl_seconds`), so channels that go silent are auto-reaped by Redis — no manual cleanup scheduler needed. Set `history_ttl_seconds = 0` to disable and retain history indefinitely.

## Compatibility Notes

The current hardening pass tightened a few pre-1.0 semantics:

- `message` and `client_event` now require an active matching subscription.
- Channel-wide publishes reach all subscribers; room publishes reach only exact room subscribers.
- `on_leave` and `presence:leave` run on unsubscribe, socket close, heartbeat timeout, and force disconnect.
- Channel callbacks receive owned `WebSocketContext`, `ChannelId`, and room values so async closures can safely move data into futures.

### Including payloads in history

If you need to see message bodies (e.g., in staging or internal tooling), opt in via config:

```toml
[observability.websocket]
include_payloads = true
```

Or via environment:

```
OBSERVABILITY__WEBSOCKET__INCLUDE_PAYLOADS=true
```

When enabled, `/ws/history/:channel` returns the full `ServerMessage.payload` for each buffered entry. Use this with care — payloads may contain personal or sensitive data.

### Per-channel stats

`GET /_foundry/ws/stats` pairs the existing global counters with a per-channel breakdown:

```json
{
  "global": {
    "active_connections": 40,
    "active_subscriptions": 85,
    "inbound_messages_total": 12300,
    "outbound_messages_total": 45600
  },
  "channels": [
    {
      "id": "chat",
      "active_subscriptions": 20,
      "subscriptions_total": 200,
      "unsubscribes_total": 180,
      "inbound_messages_total": 5000,
      "outbound_messages_total": 20000
    }
  ]
}
```

Registered channels with no traffic appear with zero counters. Counters are per-process, matching the semantics of the existing global counters — aggregate across instances in your metrics backend if needed. Idle per-channel diagnostics are bounded by `observability.websocket_channel_retention`; active channels are not evicted.

The same series are also emitted in Prometheus format on `/_foundry/metrics`, labelled by `channel`:

```
foundry_websocket_subscriptions_total{channel="chat"} 200
foundry_websocket_channel_unsubscribes_total{channel="chat"} 180
foundry_websocket_active_subscriptions{channel="chat"} 20
foundry_websocket_channel_messages_total{channel="chat",direction="inbound"} 5000
foundry_websocket_channel_messages_total{channel="chat",direction="outbound"} 20000
```

### What these endpoints intentionally don't do

- **No admin actions.** Broadcast, force-disconnect, and history purge are deliberately not exposed. Build those into your app code where the authorization story is yours to own.
- **No bundled UI.** Every endpoint returns JSON; wire it into whatever dashboard you already run.
- **No per-connection list.** Per-node connection registries are confusing in multi-instance deployments. Use presence to see who is subscribed.
