# WebSocket Observability Dashboard Routes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four read-only JSON endpoints under `/_foundry/ws/*` exposing the WebSocket channel registry, presence members, replay history, and per-channel counters — matching the style of the existing `/_foundry/jobs/*` dashboard routes.

**Architecture:** Execute the existing `WebSocketRouteRegistrar` closures once during `AppBuilder::bootstrap()` to produce a `WebSocketChannelRegistry`, stash it in the DI container, then (a) have `WebSocketKernel` read channels from the container instead of re-running closures, and (b) add four Axum handlers in `src/logging/observability.rs` that consult the registry, `RuntimeBackend`, and `RuntimeDiagnostics`. Per-channel counters are added to `RuntimeDiagnostics` as a `DashMap<ChannelId, PerChannelWebSocketCounters>` keyed by channel; kernel/publisher call sites are swapped to `_on(&ChannelId)` variants.

**Tech Stack:** Rust 1.94, Axum 0.7, Tokio, `dashmap`, serde, existing `RuntimeBackend` Redis helpers (`smembers`, `scard`, `lrange`), existing `HttpRegistrar`.

**Reference spec:** `docs/superpowers/specs/2026-04-19-websocket-dashboard-routes-design.md`

---

## File Structure

**New files:**
- `tests/websocket_observability_acceptance.rs` — acceptance tests for all four endpoints.

**Modified files:**
- `src/config/mod.rs` — extend `ObservabilityConfig` with nested `WebSocketObservabilityConfig`.
- `src/config/publish.rs` — add commented lines for the new config key.
- `src/config/env_publish.rs` — add commented env override for `include_payloads`.
- `src/logging/diagnostics.rs` — add `PerChannelWebSocketCounters`, `PerChannelWebSocketSnapshot`; extend `WebSocketCounters` and `WebSocketRuntimeSnapshot`; add `_on` record methods.
- `src/logging/metrics.rs` — emit per-channel Prometheus series.
- `src/kernel/websocket.rs` — swap subscription/message record calls to `_on` variants; read channels from container instead of consuming closures.
- `src/websocket/mod.rs` — add `WebSocketChannelDescriptor` and `WebSocketChannelRegistry`; make `RegisteredChannel` projection available.
- `src/foundation/app.rs` — in `bootstrap()`, run WS route closures once, build `WebSocketChannelRegistry`, register in container; add `AppContext::websocket_channels()` accessor.
- `src/logging/observability.rs` — register four new routes with handlers.
- `docs/guides/websocket.md` — add "Observability" section.
- `CHANGELOG.md` — `Added` entry and `Changed` entry for snapshot type expansion.
- `blueprints/15-framework-gaps.md` — update the dashboard route list.

**Do not touch:**
- `tests/fixtures/blueprint_app/` and `tests/fixtures/plugin_*_app/` — they must stay green but need no source edits for this feature. Any breakage signals a real ripple; fix there, do not work around.

---

## Task 1: Add `WebSocketObservabilityConfig` to `ObservabilityConfig`

**Files:**
- Modify: `src/config/mod.rs` (struct `ObservabilityConfig` around line 374-392, and imports if needed)
- Modify: `src/config/publish.rs` (the commented observability block near line 155)
- Modify: `src/config/env_publish.rs` (around line 263)
- Test: `src/config/mod.rs` (inline test module)

- [ ] **Step 1: Write the failing test**

Append to the existing test module in `src/config/mod.rs` (find the block containing `fn loads_observability_overrides` around line 1019–1030; add a sibling test next to it):

```rust
#[test]
fn loads_websocket_observability_overrides() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("00-observability.toml"),
        r#"
[websocket]
include_payloads = true
"#,
    )
    .unwrap();
    let config = ConfigRepository::from_dir_with_defaults(directory.path(), vec![]).unwrap();
    let observability: ObservabilityConfig = config.observability().unwrap();
    assert!(observability.websocket.include_payloads);
}

#[test]
fn websocket_observability_defaults_to_redacted() {
    let observability = ObservabilityConfig::default();
    assert!(!observability.websocket.include_payloads);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib loads_websocket_observability_overrides websocket_observability_defaults_to_redacted`
Expected: FAIL — `websocket` field does not exist on `ObservabilityConfig`.

- [ ] **Step 3: Add the new config type and embed it**

In `src/config/mod.rs`, around line 374, replace the existing `ObservabilityConfig` block with:

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ObservabilityConfig {
    pub base_path: String,
    pub tracing_enabled: bool,
    pub otlp_endpoint: String,
    pub service_name: String,
    pub websocket: WebSocketObservabilityConfig,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            base_path: "/_foundry".to_string(),
            tracing_enabled: false,
            otlp_endpoint: "http://localhost:4317".to_string(),
            service_name: "foundry".to_string(),
            websocket: WebSocketObservabilityConfig::default(),
        }
    }
}

/// Observability options specific to the WebSocket dashboard endpoints.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct WebSocketObservabilityConfig {
    /// When `true`, `/_foundry/ws/history/:channel` includes full `ServerMessage.payload`
    /// for each buffered message. When `false` (the default), payloads are replaced
    /// with their serialized byte length under `payload_size_bytes`, so dashboard
    /// readers cannot see raw message contents.
    pub include_payloads: bool,
}
```

- [ ] **Step 4: Re-export `WebSocketObservabilityConfig`**

In the same file at line 741 (the `pub use` list for config types), add `WebSocketObservabilityConfig` next to `ObservabilityConfig`. Grep first to find the right `pub use` statement:

Run: `grep -n "ObservabilityConfig" src/config/mod.rs`
Expected to find a line like: `pub use ... ObservabilityConfig, RedisConfig ...`
Edit that line to include `WebSocketObservabilityConfig`.

- [ ] **Step 5: Extend the published config templates**

In `src/config/publish.rs` (line ~155), add below the existing observability block:

```
# [observability.websocket]
# include_payloads = false         # Include full payloads in /_foundry/ws/history/:channel
```

In `src/config/env_publish.rs` (line ~266, after the existing OBSERVABILITY__* lines), add:

```
# OBSERVABILITY__WEBSOCKET__INCLUDE_PAYLOADS=false
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib loads_websocket_observability_overrides websocket_observability_defaults_to_redacted`
Expected: PASS.

Run: `cargo build -p foundry`
Expected: compiles cleanly.

- [ ] **Step 7: Commit**

```bash
git add src/config/mod.rs src/config/publish.rs src/config/env_publish.rs
git commit -m "Add WebSocketObservabilityConfig for dashboard payload redaction"
```

---

## Task 2: Add `PerChannelWebSocketCounters` and extend `WebSocketCounters`

**Files:**
- Modify: `src/logging/diagnostics.rs:41-51, 115-140` (snapshot type and counters)

This task adds the per-channel storage. We'll wire increments in Task 3 and kernel call sites in Task 4.

- [ ] **Step 1: Write the failing test**

Append to the existing test module at the bottom of `src/logging/diagnostics.rs`:

```rust
#[test]
fn per_channel_counters_start_at_zero_and_increment() {
    use crate::support::ChannelId;

    let diagnostics = RuntimeDiagnostics::default();
    let chat = ChannelId::new("chat");

    diagnostics.record_websocket_subscription_opened_on(&chat);
    diagnostics.record_websocket_inbound_message_on(&chat);
    diagnostics.record_websocket_outbound_message_on(&chat);
    diagnostics.record_websocket_outbound_message_on(&chat);

    let snapshot = diagnostics.snapshot().websocket;
    let channel = snapshot
        .channels
        .iter()
        .find(|c| c.id == chat)
        .expect("channel snapshot missing");
    assert_eq!(channel.subscriptions_total, 1);
    assert_eq!(channel.active_subscriptions, 1);
    assert_eq!(channel.inbound_messages_total, 1);
    assert_eq!(channel.outbound_messages_total, 2);

    // Global totals must still increment alongside per-channel.
    assert_eq!(snapshot.subscriptions_total, 1);
    assert_eq!(snapshot.inbound_messages_total, 1);
    assert_eq!(snapshot.outbound_messages_total, 2);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib per_channel_counters_start_at_zero_and_increment`
Expected: FAIL — `record_websocket_subscription_opened_on` / `_on` variants do not exist, and `channels` field missing on `WebSocketRuntimeSnapshot`.

- [ ] **Step 3: Add `PerChannelWebSocketCounters` and extend snapshot type**

In `src/logging/diagnostics.rs`, add at the top of the file alongside the other imports:

```rust
use dashmap::DashMap;
```

Below the existing `WebSocketRuntimeSnapshot` struct (around line 41–51), add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerChannelWebSocketSnapshot {
    pub id: crate::support::ChannelId,
    pub subscriptions_total: u64,
    pub unsubscribes_total: u64,
    pub active_subscriptions: u64,
    pub inbound_messages_total: u64,
    pub outbound_messages_total: u64,
}
```

Extend `WebSocketRuntimeSnapshot` (same struct, around line 42) by adding the field:

```rust
pub channels: Vec<PerChannelWebSocketSnapshot>,
```

Below the existing `WebSocketCounters` struct (around line 115–125), add:

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

Extend `WebSocketCounters` to hold the per-channel map:

```rust
#[derive(Default)]
struct WebSocketCounters {
    opened_total: AtomicU64,
    closed_total: AtomicU64,
    active_connections: AtomicU64,
    subscriptions_total: AtomicU64,
    unsubscribes_total: AtomicU64,
    active_subscriptions: AtomicU64,
    inbound_messages_total: AtomicU64,
    outbound_messages_total: AtomicU64,
    per_channel: DashMap<crate::support::ChannelId, Arc<PerChannelWebSocketCounters>>,
}
```

Add `use std::sync::Arc;` at the top of the file if not already present.

Extend `WebSocketCounters::snapshot` to populate the new vector:

```rust
impl WebSocketCounters {
    fn snapshot(&self) -> WebSocketRuntimeSnapshot {
        let mut channels: Vec<PerChannelWebSocketSnapshot> = self
            .per_channel
            .iter()
            .map(|entry| {
                let counters = entry.value();
                PerChannelWebSocketSnapshot {
                    id: entry.key().clone(),
                    subscriptions_total: counters.subscriptions_total.load(Ordering::Relaxed),
                    unsubscribes_total: counters.unsubscribes_total.load(Ordering::Relaxed),
                    active_subscriptions: counters.active_subscriptions.load(Ordering::Relaxed),
                    inbound_messages_total: counters.inbound_messages_total.load(Ordering::Relaxed),
                    outbound_messages_total: counters.outbound_messages_total.load(Ordering::Relaxed),
                }
            })
            .collect();
        channels.sort_by(|a, b| a.id.cmp(&b.id));

        WebSocketRuntimeSnapshot {
            opened_total: self.opened_total.load(Ordering::Relaxed),
            closed_total: self.closed_total.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            subscriptions_total: self.subscriptions_total.load(Ordering::Relaxed),
            unsubscribes_total: self.unsubscribes_total.load(Ordering::Relaxed),
            active_subscriptions: self.active_subscriptions.load(Ordering::Relaxed),
            inbound_messages_total: self.inbound_messages_total.load(Ordering::Relaxed),
            outbound_messages_total: self.outbound_messages_total.load(Ordering::Relaxed),
            channels,
        }
    }
}
```

Add a helper method on `WebSocketCounters` for ensuring a channel entry exists:

```rust
impl WebSocketCounters {
    fn entry(&self, channel: &crate::support::ChannelId) -> Arc<PerChannelWebSocketCounters> {
        self.per_channel
            .entry(channel.clone())
            .or_insert_with(|| Arc::new(PerChannelWebSocketCounters::default()))
            .clone()
    }
}
```

- [ ] **Step 4: Check `dashmap` is already in the workspace**

Run: `grep -n '"dashmap"' Cargo.toml`
Expected: a version line. If absent, stop and ask the user before adding a new dependency (global CLAUDE.md says "don't install new packages without asking").

If present, no action needed.

- [ ] **Step 5: Run only the build to confirm the new types compile**

Run: `cargo build -p foundry`
Expected: compiles. (The diagnostics test from Step 1 will still fail because `_on` methods don't exist yet — that's fixed in Task 3.)

- [ ] **Step 6: Commit**

```bash
git add src/logging/diagnostics.rs
git commit -m "Add per-channel WebSocket counter storage and snapshot fields"
```

---

## Task 3: Add `_on(&ChannelId)` record methods on `RuntimeDiagnostics`

**Files:**
- Modify: `src/logging/diagnostics.rs` (impl block containing `record_websocket_*` around lines 305–345)

- [ ] **Step 1: Extend the impl with per-channel record methods**

Find the impl block around line 305 (method `record_websocket_connection`) and add four new methods alongside the existing `record_websocket_subscription_opened`, `_closed`, `_inbound_message`, `_outbound_message`:

```rust
pub fn record_websocket_subscription_opened_on(&self, channel: &crate::support::ChannelId) {
    self.websocket.subscriptions_total.fetch_add(1, Ordering::Relaxed);
    self.websocket.active_subscriptions.fetch_add(1, Ordering::Relaxed);
    let entry = self.websocket.entry(channel);
    entry.subscriptions_total.fetch_add(1, Ordering::Relaxed);
    entry.active_subscriptions.fetch_add(1, Ordering::Relaxed);
}

pub fn record_websocket_subscription_closed_on(&self, channel: &crate::support::ChannelId) {
    self.websocket.unsubscribes_total.fetch_add(1, Ordering::Relaxed);
    decrement_saturating(&self.websocket.active_subscriptions);
    let entry = self.websocket.entry(channel);
    entry.unsubscribes_total.fetch_add(1, Ordering::Relaxed);
    decrement_saturating(&entry.active_subscriptions);
}

pub fn record_websocket_inbound_message_on(&self, channel: &crate::support::ChannelId) {
    self.websocket.inbound_messages_total.fetch_add(1, Ordering::Relaxed);
    self.websocket.entry(channel).inbound_messages_total.fetch_add(1, Ordering::Relaxed);
}

pub fn record_websocket_outbound_message_on(&self, channel: &crate::support::ChannelId) {
    self.websocket.outbound_messages_total.fetch_add(1, Ordering::Relaxed);
    self.websocket.entry(channel).outbound_messages_total.fetch_add(1, Ordering::Relaxed);
}
```

Add a pre-seed helper (used later in bootstrap and by the WS kernel builder):

```rust
pub fn register_websocket_channel(&self, channel: &crate::support::ChannelId) {
    let _ = self.websocket.entry(channel);
}
```

(Leave the existing global-only `record_websocket_subscription_opened` / `_closed` / `_inbound_message` / `_outbound_message` methods unchanged. They become unused by Task 4 but stay public in case external code depends on them.)

- [ ] **Step 2: Run the test from Task 2**

Run: `cargo test --lib per_channel_counters_start_at_zero_and_increment`
Expected: PASS.

- [ ] **Step 3: Run the full diagnostics test module**

Run: `cargo test --lib logging::diagnostics::tests`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/logging/diagnostics.rs
git commit -m "Add per-channel WebSocket record methods on RuntimeDiagnostics"
```

---

## Task 4: Swap kernel and publisher call sites to `_on` variants

**Files:**
- Modify: `src/kernel/websocket.rs` (lines 477, 937, 959, 980, 1152)
- Modify: `src/websocket/mod.rs` (line 215)

Connection-level counters (`record_websocket_connection`) stay global-only because the opened/closed events do not yet have a channel context (connections own many subscriptions).

- [ ] **Step 1: Write the failing assertion (integration-level)**

Add this test to the existing test module at the bottom of `src/kernel/websocket.rs`:

```rust
#[tokio::test]
async fn per_channel_counters_track_subscriptions_and_messages() {
    use crate::logging::RuntimeDiagnostics;
    use crate::support::ChannelId;

    // This test is a signpost: it confirms the kernel wires per-channel counters.
    // Actual over-the-wire traffic is exercised in tests/websocket_observability_acceptance.rs
    // (Task 12). Here we only assert the public record API is reachable with a channel.

    let diagnostics = std::sync::Arc::new(RuntimeDiagnostics::default());
    let chat = ChannelId::new("chat");
    diagnostics.record_websocket_subscription_opened_on(&chat);
    diagnostics.record_websocket_inbound_message_on(&chat);
    let snapshot = diagnostics.snapshot().websocket;
    assert_eq!(snapshot.channels.len(), 1);
    assert_eq!(snapshot.channels[0].id, chat);
}
```

Run: `cargo test --lib kernel::websocket::tests::per_channel_counters_track_subscriptions_and_messages`
Expected: PASS (the diagnostics API exists from Task 3). This test guards against accidental API removal.

- [ ] **Step 2: Find and update each call site in `src/kernel/websocket.rs`**

The current file has these call sites (line numbers may shift slightly after earlier tasks — re-grep before editing):

Run: `grep -n "record_websocket_subscription_opened\|record_websocket_subscription_closed\|record_websocket_inbound_message\|record_websocket_outbound_message" src/kernel/websocket.rs`

Expected matches (approximate):
- `477: diagnostics.record_websocket_inbound_message();`
- `885: diagnostics.record_websocket_subscription_closed();`
- `937: diagnostics.record_websocket_subscription_opened();`
- `959: diagnostics.record_websocket_subscription_closed();`
- `980: diagnostics.record_websocket_outbound_message();`
- `1152: diagnostics.record_websocket_subscription_closed();`

At each of those call sites, the surrounding code already has the channel in scope (typically `&message.channel` or `&channel.id` or via a loop variable). Read ±5 lines around each to confirm the channel binding name, then replace:

```rust
// Before:
diagnostics.record_websocket_subscription_opened();

// After:
diagnostics.record_websocket_subscription_opened_on(&channel_id_var);
```

Repeat for each of the four `_opened` / `_closed` / `_inbound_message` call sites.

Cases where the channel is not obvious:
- Line 885 and 1152: these fire during connection teardown and iterate a set of subscribed channels. Replace with a loop that calls `_on` per channel.

Example (pseudo-pattern, adapt to actual surrounding code — do NOT copy verbatim without reading the current structure):

```rust
// Before (around line 1148-1156, connection teardown):
for _channel in subscribed_channels.iter() {
    diagnostics.record_websocket_subscription_closed();
}
diagnostics.record_websocket_connection(WebSocketConnectionState::Closed);

// After:
for channel in subscribed_channels.iter() {
    diagnostics.record_websocket_subscription_closed_on(channel);
}
diagnostics.record_websocket_connection(WebSocketConnectionState::Closed);
```

Read the existing loop variable names and preserve them; the critical change is `_on(&channel)`.

- [ ] **Step 3: Update the publisher call site**

In `src/websocket/mod.rs`, around line 215:

```rust
// Before:
self.diagnostics.record_websocket_outbound_message();

// After:
self.diagnostics
    .record_websocket_outbound_message_on(&message.channel);
```

Keep it before the `publish_ws` call so local instrumentation records even if Redis fan-out fails.

- [ ] **Step 4: Run the existing WS tests**

Run: `cargo test --test phase2_acceptance`
Expected: PASS (this is the main WS integration test file; existing assertions should still hold).

Run: `cargo test --lib kernel::websocket::tests`
Expected: PASS including the new Task 4 test.

- [ ] **Step 5: Commit**

```bash
git add src/kernel/websocket.rs src/websocket/mod.rs
git commit -m "Record per-channel WebSocket counters from kernel and publisher"
```

---

## Task 5: Emit per-channel Prometheus series in `/_foundry/metrics`

**Files:**
- Modify: `src/logging/metrics.rs` (WebSocket block around lines 96–134)

- [ ] **Step 1: Write the failing test**

Find the existing Prometheus test (around line 261–292 in `src/logging/metrics.rs`) and add a sibling test:

```rust
#[test]
fn format_prometheus_emits_per_channel_websocket_series() {
    use crate::logging::diagnostics::PerChannelWebSocketSnapshot;
    use crate::support::ChannelId;

    let mut snapshot = RuntimeSnapshot::default();
    snapshot.websocket.active_connections = 5;
    snapshot.websocket.channels = vec![
        PerChannelWebSocketSnapshot {
            id: ChannelId::new("chat"),
            subscriptions_total: 10,
            unsubscribes_total: 2,
            active_subscriptions: 8,
            inbound_messages_total: 100,
            outbound_messages_total: 300,
        },
    ];

    let output = format_prometheus(&snapshot);

    assert!(output.contains("foundry_websocket_active_connections 5"));
    assert!(
        output.contains("foundry_websocket_subscriptions_total{channel=\"chat\"} 10"),
        "missing subscriptions per-channel series:\n{output}"
    );
    assert!(output.contains("foundry_websocket_active_subscriptions{channel=\"chat\"} 8"));
    assert!(
        output.contains("foundry_websocket_messages_total{channel=\"chat\",direction=\"inbound\"} 100"),
    );
    assert!(
        output.contains("foundry_websocket_messages_total{channel=\"chat\",direction=\"outbound\"} 300"),
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib format_prometheus_emits_per_channel_websocket_series`
Expected: FAIL — per-channel series not emitted.

- [ ] **Step 3: Extend the WebSocket Prometheus block**

In `src/logging/metrics.rs` at the end of the existing WebSocket block (around line 134, just before the `// Scheduler counters` comment), append:

```rust
// Per-channel WebSocket series
write_help_type(
    &mut out,
    "foundry_websocket_subscriptions_total",
    "Total WebSocket subscriptions per channel",
    "counter",
);
write_help_type(
    &mut out,
    "foundry_websocket_active_subscriptions",
    "Currently active WebSocket subscriptions per channel",
    "gauge",
);
for channel in &snapshot.websocket.channels {
    let id = channel.id.as_str();
    let _ = writeln!(
        out,
        "foundry_websocket_subscriptions_total{{channel=\"{id}\"}} {}",
        channel.subscriptions_total
    );
    let _ = writeln!(
        out,
        "foundry_websocket_active_subscriptions{{channel=\"{id}\"}} {}",
        channel.active_subscriptions
    );
    let _ = writeln!(
        out,
        "foundry_websocket_messages_total{{channel=\"{id}\",direction=\"inbound\"}} {}",
        channel.inbound_messages_total
    );
    let _ = writeln!(
        out,
        "foundry_websocket_messages_total{{channel=\"{id}\",direction=\"outbound\"}} {}",
        channel.outbound_messages_total
    );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib format_prometheus_emits_per_channel_websocket_series`
Expected: PASS.

Run: `cargo test --lib logging::metrics`
Expected: PASS (existing tests still green).

- [ ] **Step 5: Commit**

```bash
git add src/logging/metrics.rs
git commit -m "Emit per-channel WebSocket series in Prometheus output"
```

---

## Task 6: Add `WebSocketChannelDescriptor` and `WebSocketChannelRegistry`

**Files:**
- Modify: `src/websocket/mod.rs` (append to file)

- [ ] **Step 1: Write the failing test**

Append to the existing test module at the bottom of `src/websocket/mod.rs`:

```rust
#[test]
fn descriptor_is_projected_from_registered_channel() {
    let mut registrar = WebSocketRegistrar::new();
    registrar
        .channel_with_options(
            ChannelId::new("chat"),
            |_ctx, _payload| async { Ok(()) },
            WebSocketChannelOptions::new()
                .presence(true)
                .replay(25)
                .allow_client_events(true)
                .guard(GuardId::new("api"))
                .permissions([PermissionId::new("chat:read")]),
        )
        .unwrap();

    let registry = WebSocketChannelRegistry::from_registrar(registrar);

    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 1);
    let descriptor = &descriptors[0];
    assert_eq!(descriptor.id, ChannelId::new("chat"));
    assert!(descriptor.presence);
    assert_eq!(descriptor.replay_count, 25);
    assert!(descriptor.allow_client_events);
    assert!(descriptor.requires_auth);
    assert_eq!(descriptor.guard.as_ref(), Some(&GuardId::new("api")));
    assert_eq!(
        descriptor.permissions,
        vec![PermissionId::new("chat:read")]
    );

    assert!(registry.find(&ChannelId::new("chat")).is_some());
    assert!(registry.find(&ChannelId::new("missing")).is_none());
}
```

Also update the imports at the top of the test module:

```rust
use super::{
    ChannelId, WebSocketChannelOptions, WebSocketChannelRegistry, WebSocketRegistrar,
};
use crate::support::{GuardId, PermissionId};
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib websocket::tests::descriptor_is_projected_from_registered_channel`
Expected: FAIL — `WebSocketChannelRegistry` / `WebSocketChannelDescriptor` do not exist.

- [ ] **Step 3: Add the descriptor and registry types**

In `src/websocket/mod.rs`, after the existing `RegisteredChannel` struct (around line 437–442), append:

```rust
/// Public projection of a registered WebSocket channel's configuration.
///
/// Emitted by the `/_foundry/ws/channels` dashboard endpoint and returned
/// from [`AppContext::websocket_channels`](crate::foundation::AppContext::websocket_channels).
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

impl From<&RegisteredChannel> for WebSocketChannelDescriptor {
    fn from(channel: &RegisteredChannel) -> Self {
        Self {
            id: channel.id.clone(),
            presence: channel.options.presence,
            replay_count: channel.options.replay_count,
            allow_client_events: channel.options.allow_client_events,
            requires_auth: channel.options.requires_auth(),
            guard: channel.options.guard_id().cloned(),
            permissions: channel.options.permissions_set().into_iter().collect(),
        }
    }
}

/// Shared registry of `RegisteredChannel` entries, stored in the `AppContext`
/// container so both the WebSocket kernel and dashboard handlers read from the
/// same source of truth.
#[derive(Debug, Clone, Default)]
pub struct WebSocketChannelRegistry {
    channels: Arc<Vec<RegisteredChannel>>,
}

impl WebSocketChannelRegistry {
    pub fn from_registrar(registrar: WebSocketRegistrar) -> Self {
        let mut channels = registrar.into_channels();
        channels.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            channels: Arc::new(channels),
        }
    }

    pub fn descriptors(&self) -> Vec<WebSocketChannelDescriptor> {
        self.channels.iter().map(Into::into).collect()
    }

    pub fn find(&self, id: &ChannelId) -> Option<WebSocketChannelDescriptor> {
        self.channels
            .iter()
            .find(|c| c.id == *id)
            .map(Into::into)
    }

    pub(crate) fn registered_channels(&self) -> &[RegisteredChannel] {
        &self.channels
    }
}
```

- [ ] **Step 4: Re-export the new public types**

In `src/prelude.rs` around line 103 (where `WebSocketChannelOptions, WebSocketContext, WebSocketPublisher, WebSocketRegistrar` is exported), add `WebSocketChannelDescriptor` and `WebSocketChannelRegistry`:

```rust
pub use crate::websocket::{
    // ...existing exports...
    WebSocketChannelDescriptor, WebSocketChannelOptions, WebSocketChannelRegistry,
    WebSocketContext, WebSocketPublisher, WebSocketRegistrar,
    // ...
};
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib websocket::tests::descriptor_is_projected_from_registered_channel`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/websocket/mod.rs src/prelude.rs
git commit -m "Add WebSocketChannelDescriptor and WebSocketChannelRegistry"
```

---

## Task 7: Build registry at bootstrap; refactor `WebSocketKernel` to read from container

**Files:**
- Modify: `src/foundation/app.rs` (bootstrap flow around lines 1033–1049 and service registration ~929–944)
- Modify: `src/kernel/websocket.rs` (struct `WebSocketKernel`, `new`, `build_router` lines 29–77)

- [ ] **Step 1: Write the failing test**

Append to the existing test module at the bottom of `src/foundation/app.rs` (or create one if absent). This test checks that after building the kernel, the registry is resolvable from the container:

```rust
#[tokio::test]
async fn bootstrap_registers_websocket_channel_registry() {
    use crate::support::ChannelId;
    use crate::websocket::WebSocketChannelRegistry;

    let builder = crate::App::builder().register_websocket_routes(|r| {
        r.channel(ChannelId::new("chat"), |_ctx, _payload| async { Ok(()) })?;
        Ok(())
    });

    let kernel = builder.build_websocket_kernel().await.expect("kernel builds");
    let registry = kernel
        .app()
        .container()
        .resolve::<WebSocketChannelRegistry>()
        .expect("registry registered during bootstrap");

    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].id, ChannelId::new("chat"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib bootstrap_registers_websocket_channel_registry`
Expected: FAIL — `WebSocketChannelRegistry` is not registered in the container.

- [ ] **Step 3: Build the registry in `bootstrap()` and register it**

In `src/foundation/app.rs`, locate the block that collects websocket routes (around line 1034–1035):

```rust
let mut boot_websocket_routes = prepared_plugins.websocket_routes;
boot_websocket_routes.extend(websocket_routes);
```

Immediately after that, build the registrar and register the resulting `WebSocketChannelRegistry` in the container. Also pre-seed per-channel diagnostic counters so idle channels appear in `/ws/stats`:

```rust
let mut ws_registrar = crate::websocket::WebSocketRegistrar::new();
for route in &boot_websocket_routes {
    route(&mut ws_registrar)?;
}
let ws_registry = crate::websocket::WebSocketChannelRegistry::from_registrar(ws_registrar);
for descriptor in ws_registry.descriptors() {
    diagnostics.register_websocket_channel(&descriptor.id);
}
app.container()
    .singleton_arc(std::sync::Arc::new(ws_registry.clone()))?;
```

(If `diagnostics` is not already in scope at this point, move the `let diagnostics = ...` line so it is. Read surrounding lines — line 941 registers it in the container after construction.)

Keep the `websocket_routes` field on `BootArtifacts` for now; Task 7 Step 4 removes its use in the kernel.

- [ ] **Step 4: Refactor `WebSocketKernel` to stop re-running closures**

In `src/kernel/websocket.rs`, change:

```rust
pub struct WebSocketKernel {
    app: AppContext,
    routes: Vec<WebSocketRouteRegistrar>,
}

impl WebSocketKernel {
    pub fn new(app: AppContext, routes: Vec<WebSocketRouteRegistrar>) -> Self {
        Self { app, routes }
    }
    // ...
}
```

to:

```rust
pub struct WebSocketKernel {
    app: AppContext,
}

impl WebSocketKernel {
    pub fn new(app: AppContext) -> Self {
        Self { app }
    }
    // ...
}
```

Rewrite `build_router` (lines 61–77) so it resolves the registry from the container instead of running closures:

```rust
async fn build_router(&self) -> Result<axum::Router> {
    let ws_config = self.app.config().websocket()?;
    let registry = self
        .app
        .container()
        .resolve::<crate::websocket::WebSocketChannelRegistry>()?;
    let registered_channels: Vec<crate::websocket::RegisteredChannel> =
        registry.registered_channels().to_vec();
    let backend = RuntimeBackend::from_config(self.app.config())?;
    let state =
        WebSocketServerState::new(self.app.clone(), registered_channels, backend, ws_config);
    state.start_pubsub().await?;

    Ok(axum::Router::new()
        .route(&state.ws_config.path, get(websocket_handler))
        .with_state(state))
}
```

Add `impl Clone for RegisteredChannel` if it is not already derivable. Check: `grep -n "^pub(crate) struct RegisteredChannel" src/websocket/mod.rs`. If `RegisteredChannel` is already `#[derive(Clone)]` (it is at line 437 of the current file), no action needed.

- [ ] **Step 5: Update the caller in `build_websocket_kernel`**

In `src/foundation/app.rs` around line 691–693:

```rust
// Before:
pub async fn build_websocket_kernel(self) -> Result<WebSocketKernel> {
    let boot = self.bootstrap().await?;
    Ok(WebSocketKernel::new(boot.app, boot.websocket_routes))
}

// After:
pub async fn build_websocket_kernel(self) -> Result<WebSocketKernel> {
    let boot = self.bootstrap().await?;
    Ok(WebSocketKernel::new(boot.app))
}
```

The `websocket_routes` field on `BootArtifacts` is now unused. Remove it from the struct (around line 1070) and from its initialization (around line 1045).

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --lib bootstrap_registers_websocket_channel_registry`
Expected: PASS.

Run: `cargo test --test phase2_acceptance`
Expected: PASS — existing end-to-end WebSocket tests still work because the registry holds the same `Vec<RegisteredChannel>` the kernel used before.

- [ ] **Step 7: Run blueprint and plugin fixture acceptance to catch ripple**

Run: `cargo test --test blueprint_fixture_acceptance --test plugin_fixture_acceptance`
Expected: PASS. CLAUDE.md requires both fixture families stay green after bootstrap changes.

- [ ] **Step 8: Commit**

```bash
git add src/foundation/app.rs src/kernel/websocket.rs
git commit -m "Build WebSocketChannelRegistry during bootstrap and resolve from container"
```

---

## Task 8: Add `AppContext::websocket_channels()` accessor

**Files:**
- Modify: `src/foundation/app.rs` (impl `AppContext` around line 118)

- [ ] **Step 1: Write the failing test**

Append to the test module in `src/foundation/app.rs`:

```rust
#[tokio::test]
async fn app_context_exposes_websocket_channels() {
    use crate::support::ChannelId;

    let builder = crate::App::builder().register_websocket_routes(|r| {
        r.channel(ChannelId::new("alerts"), |_ctx, _payload| async { Ok(()) })?;
        Ok(())
    });

    let kernel = builder.build_websocket_kernel().await.unwrap();
    let registry = kernel.app().websocket_channels().unwrap();

    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].id, ChannelId::new("alerts"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib app_context_exposes_websocket_channels`
Expected: FAIL — `websocket_channels` method does not exist.

- [ ] **Step 3: Add the accessor**

In `src/foundation/app.rs`, next to `pub fn websocket(&self)` (line 118), add:

```rust
pub fn websocket_channels(&self) -> Result<Arc<crate::websocket::WebSocketChannelRegistry>> {
    self.resolve::<crate::websocket::WebSocketChannelRegistry>()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib app_context_exposes_websocket_channels`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/foundation/app.rs
git commit -m "Expose WebSocketChannelRegistry on AppContext"
```

---

## Task 9: Add `/_foundry/ws/channels` handler and route

**Files:**
- Modify: `src/logging/observability.rs` (register_observability_routes around line 64–106, and handler functions)

- [ ] **Step 1: Write the failing test**

Create `tests/websocket_observability_acceptance.rs`:

```rust
use foundry::support::{ChannelId, GuardId, PermissionId};
use foundry::testing::TestApp;
use foundry::websocket::WebSocketChannelOptions;
use serde_json::Value;

#[tokio::test]
async fn ws_channels_endpoint_lists_registered_channels() {
    let app = TestApp::builder()
        .register_websocket_routes(|r| {
            r.channel_with_options(
                ChannelId::new("chat"),
                |_ctx, _payload| async { Ok(()) },
                WebSocketChannelOptions::new()
                    .presence(true)
                    .replay(10)
                    .allow_client_events(false)
                    .guard(GuardId::new("api"))
                    .permissions([PermissionId::new("chat:read")]),
            )?;
            r.channel(ChannelId::new("public"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await;

    let response = app.client().get("/_foundry/ws/channels").await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    let channels = body["channels"].as_array().expect("channels array");
    assert_eq!(channels.len(), 2);

    let chat = channels
        .iter()
        .find(|c| c["id"] == "chat")
        .expect("chat present");
    assert_eq!(chat["presence"], Value::Bool(true));
    assert_eq!(chat["replay_count"], 10);
    assert_eq!(chat["allow_client_events"], Value::Bool(false));
    assert_eq!(chat["requires_auth"], Value::Bool(true));
    assert_eq!(chat["guard"], "api");
    assert_eq!(chat["permissions"], Value::Array(vec!["chat:read".into()]));

    let public = channels
        .iter()
        .find(|c| c["id"] == "public")
        .expect("public present");
    assert_eq!(public["presence"], Value::Bool(false));
    assert_eq!(public["requires_auth"], Value::Bool(false));
}
```

**Note on `TestApp::builder`:** If `TestApp` does not expose a builder that accepts `register_websocket_routes`, check `src/testing/mod.rs` for the actual API. Patterns to check: `TestApp::new_with(|builder| ...)`, `TestApp::with_app_builder(...)`. Use whatever the existing WS acceptance tests in `tests/phase2_acceptance.rs` use. Mirror that pattern.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test websocket_observability_acceptance ws_channels_endpoint_lists_registered_channels`
Expected: FAIL — handler not registered (404) or test infra compile error.

- [ ] **Step 3: Register the route and write the handler**

In `src/logging/observability.rs`, inside `register_observability_routes` just before `Ok(())` (around line 104), add:

```rust
registrar.route_with_options(
    &join_route(&config.base_path, "ws/channels"),
    get(ws_channels),
    route_options.clone(),
);
```

Append the handler below `slow_queries` (around line 236):

```rust
async fn ws_channels(State(app): State<AppContext>) -> Response {
    let registry = match app.websocket_channels() {
        Ok(registry) => registry,
        Err(error) => return internal_error_response(error),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({ "channels": registry.descriptors() })),
    )
        .into_response()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test websocket_observability_acceptance ws_channels_endpoint_lists_registered_channels`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/logging/observability.rs tests/websocket_observability_acceptance.rs
git commit -m "Add GET /_foundry/ws/channels dashboard endpoint"
```

---

## Task 10: Add `/_foundry/ws/presence/:channel` handler and route

**Files:**
- Modify: `src/logging/observability.rs`
- Modify: `tests/websocket_observability_acceptance.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/websocket_observability_acceptance.rs`:

```rust
#[tokio::test]
async fn ws_presence_endpoint_returns_members_for_presence_channel() {
    use foundry::support::runtime::RuntimeBackend;
    use foundry::websocket::{presence_key, presence_member_value, PresenceInfo};
    use foundry::support::ChannelId;

    let app = TestApp::builder()
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
        .await;

    // Seed Redis presence directly.
    let backend = RuntimeBackend::from_config(app.context().config()).unwrap();
    let key = presence_key(&ChannelId::new("team"));
    backend
        .sadd(&key, &presence_member_value("user_1", &ChannelId::new("team"), 1_713_000_000))
        .await
        .unwrap();

    let response = app.client().get("/_foundry/ws/presence/team").await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["channel"], "team");
    assert_eq!(body["count"], 1);
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["actor_id"], "user_1");
}

#[tokio::test]
async fn ws_presence_endpoint_returns_404_for_non_presence_channel() {
    let app = TestApp::builder()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("public"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await;

    let response = app.client().get("/_foundry/ws/presence/public").await.unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn ws_presence_endpoint_returns_404_for_unregistered_channel() {
    let app = TestApp::builder().build().await;

    let response = app
        .client()
        .get("/_foundry/ws/presence/ghost")
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}
```

**Note:** If `RuntimeBackend::sadd` does not exist, either (a) add a thin helper mirroring `smembers`, or (b) seed via `app.context().websocket()?.publish(...)` after subscribing a fake actor. Prefer (b) if the publisher path auto-adds presence; otherwise add the helper.

Check: `grep -n "pub async fn sadd\|conn.sadd\|sadd(" src/support/runtime.rs`
If `sadd` is absent, add it following the `smembers` / `scard` pattern:

```rust
pub async fn sadd(&self, key: &str, value: &str) -> Result<()> {
    match &self.kind {
        RuntimeBackendKind::Redis { pool, prefix } => {
            let mut conn = pool.get().await.map_err(Error::other)?;
            let full_key = prefixed(prefix, key);
            let _: () = conn.sadd(full_key, value).await.map_err(Error::other)?;
        }
        RuntimeBackendKind::InMemory { state } => {
            let mut inner = state.lock().await;
            inner.sets.entry(key.to_string()).or_default().insert(value.to_string());
        }
    }
    Ok(())
}
```

(Mirror the exact field names used by the existing `smembers` method — open that function and copy the scaffolding.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test websocket_observability_acceptance ws_presence`
Expected: FAIL — route not registered.

- [ ] **Step 3: Register the route and write the handler**

In `src/logging/observability.rs` `register_observability_routes`, add below the `/ws/channels` registration:

```rust
registrar.route_with_options(
    &join_route(&config.base_path, "ws/presence/:channel"),
    get(ws_presence),
    route_options.clone(),
);
```

Append the handler:

```rust
async fn ws_presence(
    State(app): State<AppContext>,
    axum::extract::Path(channel): axum::extract::Path<crate::support::ChannelId>,
) -> Response {
    let registry = match app.websocket_channels() {
        Ok(registry) => registry,
        Err(error) => return internal_error_response(error),
    };
    let descriptor = match registry.find(&channel) {
        Some(descriptor) => descriptor,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "channel not registered" })),
            )
                .into_response();
        }
    };
    if !descriptor.presence {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "presence not enabled for channel" })),
        )
            .into_response();
    }

    let backend = match crate::support::runtime::RuntimeBackend::from_config(app.config()) {
        Ok(backend) => backend,
        Err(error) => return internal_error_response(error),
    };
    let members_raw = match backend
        .smembers(&crate::websocket::presence_key(&channel))
        .await
    {
        Ok(members) => members,
        Err(error) => return internal_error_response(error),
    };

    let members: Vec<serde_json::Value> = members_raw
        .iter()
        .filter_map(|raw| serde_json::from_str::<crate::websocket::PresenceInfo>(raw).ok())
        .map(|info| {
            serde_json::json!({
                "actor_id": info.actor_id,
                "joined_at": info.joined_at,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "channel": channel.as_str(),
            "count": members.len(),
            "members": members,
        })),
    )
        .into_response()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test websocket_observability_acceptance ws_presence`
Expected: PASS for all three tests.

- [ ] **Step 5: Commit**

```bash
git add src/logging/observability.rs tests/websocket_observability_acceptance.rs src/support/runtime.rs
git commit -m "Add GET /_foundry/ws/presence/:channel dashboard endpoint"
```

---

## Task 11: Add `/_foundry/ws/history/:channel` handler and route

**Files:**
- Modify: `src/logging/observability.rs`
- Modify: `tests/websocket_observability_acceptance.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/websocket_observability_acceptance.rs`:

```rust
#[tokio::test]
async fn ws_history_redacts_payloads_by_default() {
    let app = TestApp::builder()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("events"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await;

    // Publish some messages so they land in ws:history:events
    let publisher = app.context().websocket().unwrap();
    publisher
        .publish(
            ChannelId::new("events"),
            foundry::support::ChannelEventId::new("created"),
            None,
            serde_json::json!({ "secret": "hello world" }),
        )
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/history/events")
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message["channel"], "events");
    assert_eq!(message["event"], "created");
    assert!(message.get("payload").is_none(), "payload must be redacted by default");
    assert!(message["payload_size_bytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn ws_history_returns_payloads_when_flag_is_set() {
    let app = TestApp::builder()
        .config_override(|c| c.observability.websocket.include_payloads = true)
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("events"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await;

    let publisher = app.context().websocket().unwrap();
    publisher
        .publish(
            ChannelId::new("events"),
            foundry::support::ChannelEventId::new("created"),
            None,
            serde_json::json!({ "secret": "hello world" }),
        )
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/ws/history/events")
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["payload"]["secret"], "hello world");
}

#[tokio::test]
async fn ws_history_returns_404_for_unregistered_channel() {
    let app = TestApp::builder().build().await;
    let response = app.client().get("/_foundry/ws/history/ghost").await.unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn ws_history_clamps_limit_to_buffer_size() {
    let app = TestApp::builder()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("events"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await;

    let response = app
        .client()
        .get("/_foundry/ws/history/events?limit=999")
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    // No panic, no error — clamped silently.
}
```

**Note on `config_override`:** If `TestApp::builder` does not have a `config_override` method, use whatever mechanism existing observability tests use to tweak config (e.g., writing a temp config file). Check `tests/observability_acceptance.rs` for the pattern.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test websocket_observability_acceptance ws_history`
Expected: FAIL — route not registered.

- [ ] **Step 3: Register the route and write the handler**

In `src/logging/observability.rs` `register_observability_routes`, add:

```rust
registrar.route_with_options(
    &join_route(&config.base_path, "ws/history/:channel"),
    get(ws_history),
    route_options.clone(),
);
```

Append the handler:

```rust
#[derive(serde::Deserialize)]
struct WsHistoryQuery {
    limit: Option<i64>,
}

async fn ws_history(
    State(app): State<AppContext>,
    axum::extract::Path(channel): axum::extract::Path<crate::support::ChannelId>,
    axum::extract::Query(params): axum::extract::Query<WsHistoryQuery>,
) -> Response {
    const HISTORY_BUFFER_MAX: i64 = 50;

    let registry = match app.websocket_channels() {
        Ok(registry) => registry,
        Err(error) => return internal_error_response(error),
    };
    if registry.find(&channel).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "channel not registered" })),
        )
            .into_response();
    }

    let limit = params
        .limit
        .unwrap_or(HISTORY_BUFFER_MAX)
        .clamp(1, HISTORY_BUFFER_MAX);

    let backend = match crate::support::runtime::RuntimeBackend::from_config(app.config()) {
        Ok(backend) => backend,
        Err(error) => return internal_error_response(error),
    };

    let history_key = format!("ws:history:{}", channel.as_str());
    let entries = match backend.lrange(&history_key, 0, limit - 1).await {
        Ok(entries) => entries,
        Err(error) => return internal_error_response(error),
    };

    let include_payloads = match app.config().observability() {
        Ok(cfg) => cfg.websocket.include_payloads,
        Err(error) => return internal_error_response(error),
    };

    let messages: Vec<serde_json::Value> = entries
        .iter()
        .filter_map(|raw| {
            let message = serde_json::from_str::<crate::websocket::ServerMessage>(raw).ok()?;
            let base = serde_json::json!({
                "channel": message.channel.as_str(),
                "event": message.event.as_str(),
                "room": message.room,
            });
            let mut obj = base.as_object().cloned().unwrap_or_default();
            if include_payloads {
                obj.insert("payload".to_string(), message.payload);
            } else {
                let size = serde_json::to_vec(&message.payload)
                    .map(|v| v.len() as u64)
                    .unwrap_or(0);
                obj.insert("payload_size_bytes".to_string(), size.into());
            }
            Some(serde_json::Value::Object(obj))
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "channel": channel.as_str(),
            "messages": messages,
        })),
    )
        .into_response()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test websocket_observability_acceptance ws_history`
Expected: PASS for all four tests.

- [ ] **Step 5: Commit**

```bash
git add src/logging/observability.rs tests/websocket_observability_acceptance.rs
git commit -m "Add GET /_foundry/ws/history/:channel dashboard endpoint"
```

---

## Task 12: Add `/_foundry/ws/stats` handler and route

**Files:**
- Modify: `src/logging/observability.rs`
- Modify: `tests/websocket_observability_acceptance.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/websocket_observability_acceptance.rs`:

```rust
#[tokio::test]
async fn ws_stats_exposes_global_and_per_channel_counters() {
    let app = TestApp::builder()
        .register_websocket_routes(|r| {
            r.channel(ChannelId::new("alpha"), |_ctx, _payload| async { Ok(()) })?;
            r.channel(ChannelId::new("idle"), |_ctx, _payload| async { Ok(()) })?;
            Ok(())
        })
        .build()
        .await;

    // Drive some alpha traffic via the diagnostics API directly.
    let diagnostics = app.context().diagnostics().unwrap();
    diagnostics.record_websocket_subscription_opened_on(&ChannelId::new("alpha"));
    diagnostics.record_websocket_inbound_message_on(&ChannelId::new("alpha"));
    diagnostics.record_websocket_outbound_message_on(&ChannelId::new("alpha"));
    diagnostics.record_websocket_outbound_message_on(&ChannelId::new("alpha"));

    let response = app.client().get("/_foundry/ws/stats").await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();

    assert_eq!(body["global"]["active_subscriptions"], 1);
    assert_eq!(body["global"]["inbound_messages_total"], 1);
    assert_eq!(body["global"]["outbound_messages_total"], 2);

    let channels = body["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2, "registered-but-idle channels appear too");

    let alpha = channels.iter().find(|c| c["id"] == "alpha").unwrap();
    assert_eq!(alpha["subscriptions_total"], 1);
    assert_eq!(alpha["active_subscriptions"], 1);
    assert_eq!(alpha["inbound_messages_total"], 1);
    assert_eq!(alpha["outbound_messages_total"], 2);

    let idle = channels.iter().find(|c| c["id"] == "idle").unwrap();
    assert_eq!(idle["subscriptions_total"], 0);
    assert_eq!(idle["outbound_messages_total"], 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test websocket_observability_acceptance ws_stats_exposes_global_and_per_channel_counters`
Expected: FAIL — route not registered.

- [ ] **Step 3: Register the route and write the handler**

In `src/logging/observability.rs` `register_observability_routes`, add:

```rust
registrar.route_with_options(
    &join_route(&config.base_path, "ws/stats"),
    get(ws_stats),
    route_options,
);
```

(Note: this is the last registration, so consume `route_options` instead of cloning.)

Append the handler:

```rust
async fn ws_stats(State(app): State<AppContext>) -> Response {
    let diagnostics = match app.diagnostics() {
        Ok(d) => d,
        Err(error) => return internal_error_response(error),
    };
    let ws_snapshot = diagnostics.snapshot().websocket;

    let channels: Vec<serde_json::Value> = ws_snapshot
        .channels
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id.as_str(),
                "subscriptions_total": c.subscriptions_total,
                "unsubscribes_total": c.unsubscribes_total,
                "active_subscriptions": c.active_subscriptions,
                "inbound_messages_total": c.inbound_messages_total,
                "outbound_messages_total": c.outbound_messages_total,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "global": {
                "active_connections": ws_snapshot.active_connections,
                "active_subscriptions": ws_snapshot.active_subscriptions,
                "subscriptions_total": ws_snapshot.subscriptions_total,
                "unsubscribes_total": ws_snapshot.unsubscribes_total,
                "inbound_messages_total": ws_snapshot.inbound_messages_total,
                "outbound_messages_total": ws_snapshot.outbound_messages_total,
                "opened_total": ws_snapshot.opened_total,
                "closed_total": ws_snapshot.closed_total,
            },
            "channels": channels,
        })),
    )
        .into_response()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test websocket_observability_acceptance ws_stats_exposes_global_and_per_channel_counters`
Expected: PASS.

- [ ] **Step 5: Run the full acceptance file**

Run: `cargo test --test websocket_observability_acceptance`
Expected: PASS for every test added across Tasks 9–12.

- [ ] **Step 6: Commit**

```bash
git add src/logging/observability.rs tests/websocket_observability_acceptance.rs
git commit -m "Add GET /_foundry/ws/stats dashboard endpoint"
```

---

## Task 13: Update the WebSocket consumer guide

**Files:**
- Modify: `docs/guides/websocket.md`

- [ ] **Step 1: Append an "Observability" section**

At the end of `docs/guides/websocket.md`, append:

```markdown
---

## Observability

Foundry exposes read-only JSON endpoints under the observability base path (default `/_foundry`) for inspecting WebSocket state from ops tooling or custom admin apps. All endpoints honor the same `ObservabilityOptions` access scope as the rest of the dashboard — gate them behind a guard and permission for production use.

### Endpoints

| Route                                 | Purpose                                      |
| ------------------------------------- | -------------------------------------------- |
| `GET /_foundry/ws/channels`             | List all registered channels and their options |
| `GET /_foundry/ws/presence/:channel`    | Live presence members for a presence channel |
| `GET /_foundry/ws/history/:channel`     | Last up-to-50 buffered messages (metadata only by default) |
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

#### Example: peek recent history (metadata only)

```bash
curl -s "http://localhost:3000/_foundry/ws/history/chat?limit=10" | jq
```

Each message entry includes `{ channel, event, room, payload_size_bytes }`. The raw `payload` is **not** included by default.

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

Registered channels with no traffic appear with zero counters. Counters are per-process, matching the semantics of the existing global counters — aggregate across instances in your metrics backend if needed.

The same series are also emitted in Prometheus format on `/_foundry/metrics`, labelled by `channel`:

```
foundry_websocket_subscriptions_total{channel="chat"} 200
foundry_websocket_active_subscriptions{channel="chat"} 20
foundry_websocket_messages_total{channel="chat",direction="inbound"} 5000
foundry_websocket_messages_total{channel="chat",direction="outbound"} 20000
```

### What these endpoints intentionally don't do

- **No admin actions.** Broadcast, force-disconnect, and history purge are deliberately not exposed. Build those into your app code where the authorization story is yours to own.
- **No bundled UI.** Every endpoint returns JSON; wire it into whatever dashboard you already run.
- **No per-connection list.** Per-node connection registries are confusing in multi-instance deployments. Use presence to see who is subscribed.
```

- [ ] **Step 2: Commit**

```bash
git add docs/guides/websocket.md
git commit -m "Document WebSocket observability dashboard endpoints"
```

---

## Task 14: Update `CHANGELOG.md`

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append entries under `[Unreleased]`**

In `CHANGELOG.md`, under `## [Unreleased]`:

Under `### Added`, append:

```markdown
- WebSocket observability dashboard endpoints: `GET /_foundry/ws/channels`, `GET /_foundry/ws/presence/:channel`, `GET /_foundry/ws/history/:channel`, and `GET /_foundry/ws/stats`. History payloads are redacted by default; set `observability.websocket.include_payloads = true` to include them.
- Per-channel WebSocket Prometheus series on `/_foundry/metrics` (`foundry_websocket_subscriptions_total{channel=...}`, `foundry_websocket_active_subscriptions{channel=...}`, `foundry_websocket_messages_total{channel=...,direction=...}`).
- `AppContext::websocket_channels()` accessor returning the registered channel registry.
```

Under `### Changed`, append:

```markdown
- `WebSocketRuntimeSnapshot` now includes a `channels: Vec<PerChannelWebSocketSnapshot>` field in addition to the existing global counters.
- `WebSocketKernel::new` no longer takes a `Vec<WebSocketRouteRegistrar>`; registered channels are built once during `AppBuilder::bootstrap()` and resolved from the DI container. Direct callers of `WebSocketKernel::new` must drop the routes argument.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "Changelog: WebSocket dashboard endpoints and per-channel metrics"
```

---

## Task 15: Update the dashboard route list in the blueprint

**Files:**
- Modify: `blueprints/15-framework-gaps.md` (the route table around line 389–396)

- [ ] **Step 1: Extend the route list**

In `blueprints/15-framework-gaps.md`, find the block:

```
/_foundry/health          — liveness probe
/_foundry/ready           — readiness probe
/_foundry/runtime         — runtime snapshot
/_foundry/metrics         — Prometheus format
/_foundry/jobs/stats      — job statistics
/_foundry/jobs/failed     — failed jobs
/_foundry/openapi.json    — OpenAPI spec
/_foundry/sql             — slow query log (NEW)
```

Add below it:

```
/_foundry/ws/channels     — registered WebSocket channels
/_foundry/ws/presence/:channel — live presence members
/_foundry/ws/history/:channel  — recent buffered messages (metadata by default)
/_foundry/ws/stats        — global + per-channel WebSocket counters
```

- [ ] **Step 2: Commit**

```bash
git add blueprints/15-framework-gaps.md
git commit -m "Blueprint: list WebSocket dashboard endpoints"
```

---

## Task 16: Final verification

- [ ] **Step 1: Run the full verify pipeline**

Run: `make verify`
Expected: PASS (fmt + full test suite + clippy + fixture checks).

If `fixture-check` fails, open both `tests/fixtures/blueprint_app/` and `tests/fixtures/plugin_consumer_app/` and investigate — do not silence warnings. A likely failure mode is a fixture that calls `WebSocketKernel::new(app, routes)` directly; if so, update it to the new single-argument signature.

- [ ] **Step 2: Spot-check the new endpoints manually (optional)**

If a local Redis + Postgres environment is available, run one of the example apps that has WS registered and:

```bash
curl http://localhost:3000/_foundry/ws/channels
curl http://localhost:3000/_foundry/ws/stats
curl http://localhost:3000/_foundry/metrics | grep foundry_websocket_
```

Each should return non-empty, well-formed JSON (or Prometheus text) matching the shapes documented above.

- [ ] **Step 3: Nothing to commit**

`make verify` may reformat generated files; if so, commit the formatting changes with:

```bash
git add -A
git status
# Review the diff; if only format/generated, commit:
git commit -m "Format: apply cargo fmt from verify run"
```

If `git status` is clean, skip.

---

## Self-Review Notes

Spec coverage check:
- Endpoints 1–4 → Tasks 9, 10, 11, 12. ✓
- `WebSocketObservabilityConfig` → Task 1. ✓
- Per-channel counters → Tasks 2, 3, 4. ✓
- `WebSocketChannelDescriptor` + registry → Task 6. ✓
- `AppContext::websocket_channels()` → Task 8. ✓
- Prometheus per-channel series (spec-adjacent) → Task 5. ✓
- Docs (guide, CHANGELOG, blueprint) → Tasks 13, 14, 15. ✓
- Error handling → covered in handler code in Tasks 9–12.
- Out-of-scope items (admin mutations, HTML UI, connection list, cross-node aggregation) → not in any task. ✓

Placeholder scan: no TBDs, no "add appropriate error handling," every test has concrete code. The only instruction-style hints ("check surrounding code before editing") sit on top of concrete replace-blocks, not in place of them.

Type consistency: `record_websocket_*_on` signature is identical across Tasks 3, 4, 12 call sites. `WebSocketChannelRegistry::find(&ChannelId) -> Option<WebSocketChannelDescriptor>` matches usage in Tasks 10 and 11.
