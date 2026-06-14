# WebSocket Observability Dashboard Routes — Design Spec

## Context

Foundry already ships a set of read-only observability endpoints under `/_foundry/*`:

| Route                    | Purpose                              |
| ------------------------ | ------------------------------------ |
| `GET /_foundry/health`     | liveness probe                       |
| `GET /_foundry/ready`      | readiness checks                     |
| `GET /_foundry/runtime`    | diagnostics snapshot                 |
| `GET /_foundry/metrics`    | Prometheus text format               |
| `GET /_foundry/jobs/stats` | `job_history` status counts          |
| `GET /_foundry/jobs/failed`| last 50 failed/retried jobs          |
| `GET /_foundry/sql`        | recent slow queries                  |
| `GET /_foundry/openapi.json` | OpenAPI spec                       |

The WebSocket subsystem is absent from this surface even though Foundry already collects meaningful state for it:

- Global WS counters on `RuntimeDiagnostics` (opened/closed, active connections, subs, inbound/outbound messages) — reachable via `/_foundry/runtime` and `/_foundry/metrics`, but only as global totals with no per-channel breakdown.
- Redis presence sets at `ws:presence:<channel>` (populated by the WS kernel for presence-enabled channels).
- Redis replay history at `ws:history:<channel>` (last 50 published `ServerMessage`s).
- A `WebSocketRegistrar` containing every registered channel and its options (`presence`, `replay_count`, `access`, `allow_client_events`).

Operators have no way to query any of this per-channel. This spec adds read-only JSON endpoints under `/_foundry/ws/*` that expose the existing state, plus lightweight per-channel counters that today don't exist in aggregate form.

## Goals

- Surface WS channel registry, presence membership, recent history, and per-channel counters as JSON endpoints.
- Match the existing `/_foundry/*` pattern: same `AccessScope` gate, same `internal_error_response` shape, same config tree.
- Keep the instrumentation cost on the hot path trivial (one atomic bump per event per channel).
- Redact message payloads by default; gate full-payload history behind an explicit opt-in flag.

## Non-goals

- Admin mutations: broadcast, force-disconnect, history purge. Existing `WebSocketPublisher::publish` and `disconnect_user` are suitable building blocks, but a state-changing surface deserves its own auth/audit design pass.
- Bundled HTML admin page. All endpoints return JSON; operators can build whatever UI they want on top.
- Per-connection / per-node connection registry. Presence covers "who is here" for channels that matter; a node-local connection list is confusing in multi-instance deployments and is better deferred.
- Cross-node aggregation. Per-channel counters remain per-process, identical semantics to the existing global counters.

## Endpoints

All endpoints are mounted under `observability.base_path` (default `/_foundry`). They reuse `ObservabilityOptions::access` (same guard / permission gate as the rest of the dashboard).

### `GET /_foundry/ws/channels`

Registry dump.

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

No I/O — reads the in-process registry snapshot stored on `AppContext` at bootstrap.

### `GET /_foundry/ws/presence/:channel`

Live presence members from Redis.

```json
{
  "channel": "chat",
  "count": 3,
  "members": [
    { "actor_id": "user_1", "joined_at": 1713456789 }
  ]
}
```

- Returns `404` via the framework's standard `Error::not_found` path ("channel not registered") if the channel is not in the registry.
- Returns `404` ("presence not enabled for channel") if the channel is registered but has `presence: false`.
- Backed by `RuntimeBackend::smembers(presence_key(&channel))`.

### `GET /_foundry/ws/history/:channel?limit=N`

Recent messages from the replay buffer.

- `limit` defaults to 50; values outside `[1, 50]` are clamped silently.
- Backed by `RuntimeBackend::lrange(&format!("ws:history:{channel}"), 0, limit - 1)`.

Default response (payloads redacted). `payload_size_bytes` is the byte length of the serialized JSON payload:

```json
{
  "channel": "chat",
  "messages": [
    { "channel": "chat", "event": "message", "room": null, "payload_size_bytes": 245 }
  ]
}
```

With `observability.websocket.include_payloads = true`:

```json
{
  "channel": "chat",
  "messages": [
    { "channel": "chat", "event": "message", "room": null, "payload": { "...": "..." } }
  ]
}
```

`404` if the channel is not registered.

### `GET /_foundry/ws/stats`

Global totals plus per-channel counters.

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

Channels that are registered but have seen no traffic appear with zero counters. Channels never seen in the registry do not appear (we pre-seed the map from the registrar at bootstrap).

## Implementation

### Config (`src/config/mod.rs`, `publish.rs`, `env_publish.rs`)

Extend `ObservabilityConfig`:

```rust
pub struct ObservabilityConfig {
    // ...existing fields...
    pub websocket: WebSocketObservabilityConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WebSocketObservabilityConfig {
    /// Include full `ServerMessage.payload` in `/ws/history/:channel`.
    /// Defaults to `false` — metadata-only (`payload_size_bytes` in place of `payload`).
    pub include_payloads: bool,
}
```

Corresponding entries in `config/publish.rs` and `config/env_publish.rs` (e.g. `OBSERVABILITY__WEBSOCKET__INCLUDE_PAYLOADS`).

### Per-channel diagnostics (`src/logging/diagnostics.rs`)

Add a per-channel counter struct:

```rust
#[derive(Default)]
struct PerChannelWebSocketCounters {
    subscriptions_total: AtomicU64,
    unsubscribes_total: AtomicU64,
    active_subscriptions: AtomicU64,
    inbound_messages_total: AtomicU64,
    outbound_messages_total: AtomicU64,
}
```

Extend `WebSocketCounters`:

```rust
struct WebSocketCounters {
    // ...existing global atomics...
    per_channel: DashMap<ChannelId, Arc<PerChannelWebSocketCounters>>,
}
```

(`DashMap` is already used elsewhere in the crate; if not, `RwLock<HashMap>` is an acceptable fallback.)

Add `*_on(&ChannelId)` variants of the four existing record methods:

- `record_websocket_subscription_opened_on(channel)`
- `record_websocket_subscription_closed_on(channel)`
- `record_websocket_inbound_message_on(channel)`
- `record_websocket_outbound_message_on(channel)`

Each one increments both the existing global counter **and** the per-channel counter (looked up / inserted via `entry`). The existing global-only methods stay for call sites without a channel context (if any).

Extend the public snapshot type:

```rust
pub struct WebSocketRuntimeSnapshot {
    // ...existing global fields...
    pub channels: Vec<PerChannelWebSocketSnapshot>,
}

pub struct PerChannelWebSocketSnapshot {
    pub id: ChannelId,
    pub subscriptions_total: u64,
    pub unsubscribes_total: u64,
    pub active_subscriptions: u64,
    pub inbound_messages_total: u64,
    pub outbound_messages_total: u64,
}
```

At bootstrap, the kernel pre-seeds the per-channel map from the registrar so idle-but-registered channels show up in `/ws/stats` with zeros.

### Kernel wiring (`src/kernel/websocket.rs`)

Every existing call site that currently records a WS event already has a `ChannelId` in scope:

- Subscribe / unsubscribe handlers → pass `&channel` to the `_on` variants.
- Inbound `ClientMessage` dispatch → pass `&message.channel`.
- `WebSocketPublisher::publish_message` → use `message.channel` for the outbound bump.

Replace each `record_websocket_*` call with its `_on` variant. The `_on` variant internally bumps both the global and per-channel counters in a single call, so existing global totals keep working unchanged. The legacy no-channel methods can remain for any code path that genuinely lacks a channel, or be removed if no such path exists — the implementation plan will decide based on grep results.

### Channel registry exposure

Currently `WebSocketRegistrar::into_channels` is consumed by the kernel and not retained anywhere accessible from `AppContext`. Add a cloneable projection:

```rust
// src/websocket/mod.rs
#[derive(Debug, Clone, Serialize)]
pub struct WebSocketChannelDescriptor {
    pub id: ChannelId,
    pub presence: bool,
    pub replay_count: u32,
    pub allow_client_events: bool,
    pub requires_auth: bool,
    pub guard: Option<GuardId>,
    pub permissions: Vec<PermissionId>,
}

impl From<&RegisteredChannel> for WebSocketChannelDescriptor { /* ... */ }
```

Store `Arc<Vec<WebSocketChannelDescriptor>>` on `AppContext` at bootstrap (same pattern as route metadata for `/_foundry/openapi.json`). Apps that don't run the WS kernel get an empty slice — the handlers still work and simply return empty registries / 404s. Accessor:

```rust
impl AppContext {
    pub fn websocket_channels(&self) -> &[WebSocketChannelDescriptor] { /* ... */ }
}
```

Handlers look up the channel by id through this slice for registration / presence / replay-count checks.

### Routes + handlers (`src/logging/observability.rs`)

Extend `register_observability_routes`:

```rust
registrar.route_with_options(
    &join_route(&config.base_path, "ws/channels"),
    get(ws_channels),
    route_options.clone(),
);
registrar.route_with_options(
    &join_route(&config.base_path, "ws/presence/:channel"),
    get(ws_presence),
    route_options.clone(),
);
registrar.route_with_options(
    &join_route(&config.base_path, "ws/history/:channel"),
    get(ws_history),
    route_options.clone(),
);
registrar.route_with_options(
    &join_route(&config.base_path, "ws/stats"),
    get(ws_stats),
    route_options.clone(),
);
```

Handlers:

- `ws_channels(State(app))` → serialize `app.websocket_channels()`.
- `ws_presence(State(app), Path(channel))` → registry lookup → `RuntimeBackend::smembers(presence_key(&channel))` → deserialize to `PresenceInfo`.
- `ws_history(State(app), Path(channel), Query(params))` → registry lookup → `RuntimeBackend::lrange` → deserialize to `ServerMessage` → transform to metadata-only or full-payload shape based on `config.observability.websocket.include_payloads`.
- `ws_stats(State(app))` → read `diagnostics.snapshot().websocket` which now includes the per-channel vec.

Add `RuntimeBackend::lrange(key, start, stop)` if not already present (peer with existing `smembers`, `scard`, `lpush_capped`). Redis LRANGE returns a `Vec<String>`.

### Error handling

- Unregistered channel → `404` via `Error::not_found("channel not registered")`.
- Presence query on a `presence: false` channel → `404` via `Error::not_found("presence not enabled for channel")`.
- Redis unavailable → `500` via existing `internal_error_response(error)` (same path used by `jobs_stats` / `jobs_failed`).
- `limit` out of `[1, 50]` on `/ws/history` → clamp silently.

Response shape for errors follows the framework's standard `Error` → HTTP mapping; handlers do not hand-roll error JSON.

## Testing

New file `tests/websocket_observability_acceptance.rs`. Existing test infra (`TestApp`, `TestClient`, `TestResponse`) applies as-is.

Coverage:

1. `/ws/channels` — register a mix of channels (presence on/off, varying guards and permissions, `allow_client_events` true/false). Assert the JSON shape and field values round-trip correctly.
2. `/ws/presence/:channel` —
   - Member listing after simulated joins.
   - `404` on channel without `presence: true`.
   - `404` on unregistered channel.
3. `/ws/history/:channel` —
   - Seed the replay buffer via `WebSocketPublisher::publish`.
   - Assert default response is metadata-only (no `payload` key; `payload_size_bytes` present and matches).
   - Flip `observability.websocket.include_payloads = true`, assert payloads included.
   - `limit` clamping (request 999 → returns at most buffer size).
4. `/ws/stats` —
   - Registered-but-idle channels appear with zero per-channel counters.
   - Drive inbound and outbound traffic; assert per-channel counters increment and global totals match the sum when only one channel is active.

## Documentation

- **`docs/guides/websocket.md`** — append an "Observability" section covering the four endpoints, example `curl` calls, the `include_payloads` config flag, and a note that admin mutations are intentionally out of scope.
- **`CHANGELOG.md`** — user-visible entry under an `Added` heading: "Dashboard routes for WebSocket observability (`/_foundry/ws/channels`, `/_foundry/ws/presence/:channel`, `/_foundry/ws/history/:channel`, `/_foundry/ws/stats`)".
- **Rustdoc** — document the new public types (`WebSocketObservabilityConfig`, `WebSocketChannelDescriptor`, `PerChannelWebSocketSnapshot`) with short examples. `/ws/history` payload-redaction behavior must be documented on `WebSocketObservabilityConfig::include_payloads`.
- **`blueprints/15-framework-gaps.md`** — update the dashboard routes list at the `/_foundry/` section to include the four new routes.

## Out of scope (explicit)

- Broadcast / disconnect / clear-history mutation endpoints.
- Bundled HTML admin UI.
- Per-connection list endpoint.
- Cross-node counter aggregation.
- Redacting presence actor_ids or stats channel_ids.
