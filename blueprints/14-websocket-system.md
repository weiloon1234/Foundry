# Rust WebSocket System Blueprint (Framework-Level)

## Overview

This document defines the full design of Foundry's **real-time WebSocket system** — covering what's built, what's missing, and phased improvements for production-grade real-time features.

Goal:

> Provide a channel-based real-time system with private/presence channels, auth callbacks, client events, rate limiting, heartbeat, message acknowledgment, and job-integrated broadcasting — with DX comparable to Laravel Echo/Reverb.

---

# Current State

**Status: Hardened pre-1.0 foundation — ready for framework-level contract testing**

### What's Built
- Channel-based pub/sub (Redis + memory backends)
- Rooms within channels
- WebSocketPublisher for broadcasting from HTTP handlers/jobs
- Presence channels (join/leave tracking via Redis sets)
- Auth guards per channel (token-based, cached per connection)
- Permission checks per channel
- Multi-instance via Redis pub/sub
- Comprehensive diagnostics (connection/subscription/message counts)
- Heartbeat/ping-pong with stale connection cleanup
- Per-connection rate limiting and per-user connection limits
- Dynamic subscription authorization callbacks
- Client event relay, message acknowledgments, and replay history
- Bounded outbound queues with slow-consumer disconnect
- Origin allow-list support for browser handshakes
- Lifecycle cleanup on unsubscribe, socket close, heartbeat timeout, and force disconnect

### What's Still Missing / Deferred

| Feature | Priority | Impact |
|---------|----------|--------|
| Private channel route helpers | **Medium** | Guarded channels + `.authorize(...)` cover this today, but first-class helpers would improve DX |
| Durable connection recovery | **Medium** | Current replay is bounded history, not session resume with sequence numbers |
| Binary frame support | **Low** | Text-only currently |

---

# Phase 1: Critical — Production Safety

## 1.1 Heartbeat / Ping-Pong

**Status:** Done. Server sends periodic Ping frames, responds to client Ping frames with Pong, and disconnects stale sockets.

### Config

```toml
[websocket]
heartbeat_interval_seconds = 30
heartbeat_timeout_seconds = 10
```

### Internal Design

- Server spawns a heartbeat task per connection
- Every `heartbeat_interval`, send a Ping frame
- If no Pong received within `heartbeat_timeout`, close the connection
- Track `last_pong_at` on ConnectionState

### Consumer DX

No consumer action needed — automatic.

---

## 1.2 Per-Connection Rate Limiting

**Status:** Done. Foundry tracks messages per connection in a one-second window and returns an error event when exceeded.

### Config

```toml
[websocket]
max_messages_per_second = 50
```

### Internal Design

- Per-connection atomic counter + window bucket
- In `process_client_message`, check counter before processing
- If exceeded, send error event and optionally disconnect
- Uses in-memory counter (no Redis needed — per-connection, per-instance)

---

## 1.3 Channel Authorization Callbacks

**Status:** Done. Static guard/permission checks run first, then optional dynamic authorization.

### Consumer DX

```rust
registrar.channel_with_options(
    TEAM_CHAT,
    handle_chat,
    WebSocketChannelOptions::new()
        .guard(AuthGuard::Api)
        .authorize(|ctx, channel, room| async move {
            // Dynamic check: is user a member of this team?
            let team_id = room.ok_or(Error::forbidden("room required"))?;
            let is_member = TeamMember::query()
                .where_(TeamMember::USER_ID.eq(&ctx.actor().unwrap().id))
                .where_(TeamMember::TEAM_ID.eq(team_id))
                .count(ctx.app()).await? > 0;
            if is_member { Ok(()) } else { Err(Error::forbidden("not a team member")) }
        }),
);
```

### Internal Design

- `WebSocketChannelOptions` stores `authorize: Option<AuthorizeCallback>`
- Callback receives owned `WebSocketContext`, `ChannelId`, and `Option<String>` room values
- Called after guard/permission checks, before subscription is confirmed
- If callback returns Err, Foundry sends `ERROR_EVENT` and rejects subscription

---

## 1.4 Private Channels

**Status:** Deferred. Use guarded channels plus `.authorize(...)` for user-scoped access today.

First-class private-channel helpers can be added later once route-pattern conventions are stable.

---

## 1.5 Max Connections Per User

### Config

```toml
[websocket]
max_connections_per_user = 5
```

### Internal Design

- Track `user_id → Set<connection_id>` in ConnectionHub
- On new connection auth, check count
- If exceeded, reject or disconnect oldest

---

## 1.6 Force Disconnect API

### Consumer DX

```rust
// From HTTP handler or job:
app.websocket()?.disconnect_user(&user_id).await?;
app.websocket()?.disconnect_connection(connection_id).await?;
```

### Internal Design

- `WebSocketPublisher::disconnect_user()` publishes a special disconnect command via pub/sub
- Each instance checks local connections and closes matching ones

---

# Phase 2: High — Interactive Features

## 2.1 Client Events

Allow clients to send events that are relayed to other subscribers (not server-originated).

### Protocol

```json
// Client sends:
{"action": "client_event", "channel": "chat", "event": "typing", "payload": {"user": "Alice"}}

// Server relays to all OTHER subscribers of "chat" (not back to sender)
```

### Consumer DX

```rust
registrar.channel_with_options(
    CHAT,
    handle_chat,
    WebSocketChannelOptions::new()
        .allow_client_events(true)  // enable relay
);
```

---

## 2.2 Presence Change Events

When a user joins or leaves a presence channel, broadcast the change to all subscribers.

### Auto-broadcast

```json
// On join, server sends to all subscribers:
{"channel": "chat", "event": "presence:join", "payload": {"actor_id": "user-1", "joined_at": 1234567890}}

// On leave:
{"channel": "chat", "event": "presence:leave", "payload": {"actor_id": "user-1"}}
```

---

## 2.3 Channel Lifecycle Hooks

```rust
registrar.channel_with_options(
    CHAT,
    handle_message,
    WebSocketChannelOptions::new()
        .on_join(|ctx| async move {
            ctx.publish("system", json!({"message": format!("{} joined", ctx.actor().unwrap().id)})).await
        })
        .on_leave(|ctx| async move {
            ctx.publish("system", json!({"message": format!("{} left", ctx.actor().unwrap().id)})).await
        }),
);
```

---

# Phase 3: Medium — Reliability

## 3.1 Message Acknowledgment

Client can request delivery confirmation for important messages.

### Protocol

```json
// Client sends with ack_id:
{"action": "message", "channel": "orders", "payload": {...}, "ack_id": "abc123"}

// Server responds after handler succeeds:
{"channel": "system", "event": "ack", "payload": {"ack_id": "abc123", "status": "ok"}}
```

---

## 3.2 Connection Recovery

On reconnect, client can resume from last received message.

### Design

- Server assigns a `session_id` on connection
- Messages include monotonic `sequence` numbers per channel
- Client sends `resume_from: {session_id, last_sequence}` on reconnect
- Server replays missed messages from a short Redis Stream buffer

---

# Implementation Order

| Phase | Features | Status |
|-------|----------|--------|
| 1.1 | Heartbeat/ping-pong | ✅ Done — WriterCommand enum, ping task, pong tracking, stale close |
| 1.2 | Per-connection rate limiting | ✅ Done — per-connection counter, 1s window, error on exceed |
| 1.3 | Channel authorization callbacks | ✅ Done — AuthorizeCallback, wired into Subscribe flow |
| 1.4 | Private channels | Deferred — use guarded channels + authorize callback |
| 1.5 | Max connections per user | ✅ Done — user→connection tracking, limit check on auth |
| 1.6 | Force disconnect API | ✅ Done — hub + pub/sub command handling |
| 2.1 | Client events | ✅ Done — ClientAction::ClientEvent, broadcast_except (relay to others) |
| 2.2 | Presence change events | ✅ Done — auto-broadcast presence:join / presence:leave |
| 2.3 | Channel lifecycle hooks | ✅ Done — .on_join() / .on_leave() callbacks |
| 3.1 | Message acknowledgment | ✅ Done — `ack_id` on ClientMessage, ACK_EVENT after handler |
| 3.2 | Connection recovery | Partial — bounded replay is done; durable session resume is deferred |

---

# Security Checklist

| Concern | Current | Target |
|---------|---------|--------|
| Auth per channel | ✅ Guard + permissions + authorization callbacks | Keep callback API stable |
| Token revocation | ⚠️ Cached, no re-validation | Add cache TTL or re-validate periodically |
| Subscription enforcement | ✅ Messages/client events require matching subscription | Keep covered by acceptance tests |
| Rate limiting | ✅ Per-connection message rate limit | Tune defaults from production usage |
| Connection limits | ✅ Max per user | Add oldest-connection eviction policy if needed |
| Force disconnect | ✅ Cross-instance `disconnect_user` | Add connection-level disconnect only if a public connection ID story is needed |
| Browser origins | ✅ Optional exact Origin allow-list | Document production deployment guidance |
| Backpressure | ✅ Bounded outbound buffer + disconnect | Expose metrics for slow-consumer disconnects |

---

# Config (Complete)

```toml
[websocket]
host = "127.0.0.1"
port = 3010
path = "/ws"

# Phase 1
heartbeat_interval_seconds = 30
heartbeat_timeout_seconds = 10
max_messages_per_second = 50
max_connections_per_user = 5
outbound_buffer_size = 1024
allowed_origins = []
history_buffer_size = 50
history_ttl_seconds = 604800
```

---

# Assumptions

- WebSocket runs on a separate port from HTTP (existing design)
- Redis pub/sub for multi-instance broadcasting (existing)
- Presence tracked via Redis sets (existing)
- Auth tokens validated once per connection, cached (existing)
- User/private channel access is expressed through guarded channels and `.authorize(...)`
- Client events are opt-in per channel (not default)
- Message acknowledgment is opt-in per message (not default)
- Current recovery is bounded replay from Redis lists; durable session resume remains deferred

---

# One-Line Goal

> A Foundry WebSocket channel should support private/presence/public modes with dynamic authorization, client events, lifecycle hooks, rate limiting, and heartbeat — all configurable per-channel with the same DX quality as the HTTP routing system.
