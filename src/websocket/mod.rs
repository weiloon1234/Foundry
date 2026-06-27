use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

use crate::auth::{AccessScope, Actor, Authenticatable};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{
    catch_future_panic, catch_sync_panic, panic_payload_message, RuntimeDiagnostics,
};
use crate::support::runtime::RuntimeBackend;
use crate::support::sync::{read_unpoisoned, write_unpoisoned};
use crate::support::{ChannelEventId, ChannelId, GuardId, PermissionId};

pub(crate) fn presence_key(channel: &ChannelId) -> String {
    format!("ws:presence:{}", channel.as_str())
}

pub(crate) fn presence_member_value(actor_id: &str, channel: &ChannelId, joined_at: i64) -> String {
    serde_json::to_string(&PresenceInfo {
        actor_id: actor_id.to_string(),
        channel: channel.clone(),
        joined_at,
    })
    .unwrap_or_default()
}

pub type WebSocketRouteRegistrar = Arc<dyn Fn(&mut WebSocketRegistrar) -> Result<()> + Send + Sync>;

pub(crate) fn build_registrar(
    registrars: &[WebSocketRouteRegistrar],
) -> Result<WebSocketRegistrar> {
    let mut registrar = WebSocketRegistrar::new();
    for route in registrars {
        match catch_sync_panic(|| route(&mut registrar)) {
            Ok(result) => result?,
            Err(panic) => return Err(websocket_registrar_panic_error(panic)),
        }
    }
    Ok(registrar)
}

fn websocket_registrar_panic_error(panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.websocket",
        panic = %message,
        "websocket registrar panicked"
    );
    Error::message(format!("websocket registrar panicked: {message}"))
}

#[derive(
    Debug, Clone, Serialize, Deserialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct PresenceInfo {
    pub actor_id: String,
    pub channel: ChannelId,
    #[ts(type = "number")]
    pub joined_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, foundry_macros::AppEnum)]
pub enum WebSocketAckStatus {
    Ok,
    Error,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct WebSocketAckPayload {
    pub ack_id: String,
    pub status: WebSocketAckStatus,
    pub error: Option<String>,
}

impl WebSocketAckPayload {
    pub fn ok(ack_id: impl Into<String>) -> Self {
        Self {
            ack_id: ack_id.into(),
            status: WebSocketAckStatus::Ok,
            error: None,
        }
    }

    pub fn error(ack_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            ack_id: ack_id.into(),
            status: WebSocketAckStatus::Error,
            error: Some(error.into()),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct WebSocketPresenceJoinPayload {
    pub actor_id: String,
    #[ts(type = "number")]
    pub joined_at: i64,
}

impl WebSocketPresenceJoinPayload {
    pub fn new(actor_id: impl Into<String>, joined_at: i64) -> Self {
        Self {
            actor_id: actor_id.into(),
            joined_at,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct WebSocketPresenceLeavePayload {
    pub actor_id: String,
}

impl WebSocketPresenceLeavePayload {
    pub fn new(actor_id: impl Into<String>) -> Self {
        Self {
            actor_id: actor_id.into(),
        }
    }
}

pub const SYSTEM_CHANNEL: ChannelId = ChannelId::new("system");
pub const ERROR_EVENT: ChannelEventId = ChannelEventId::new("error");
pub const SUBSCRIBED_EVENT: ChannelEventId = ChannelEventId::new("subscribed");
pub const UNSUBSCRIBED_EVENT: ChannelEventId = ChannelEventId::new("unsubscribed");
pub const PRESENCE_JOIN_EVENT: ChannelEventId = ChannelEventId::new("presence:join");
pub const PRESENCE_LEAVE_EVENT: ChannelEventId = ChannelEventId::new("presence:leave");
pub const ACK_EVENT: ChannelEventId = ChannelEventId::new("ack");

pub(crate) fn is_reserved_channel_protocol_event(event: &ChannelEventId) -> bool {
    event == &SUBSCRIBED_EVENT
        || event == &UNSUBSCRIBED_EVENT
        || event == &PRESENCE_JOIN_EVENT
        || event == &PRESENCE_LEAVE_EVENT
}

pub(crate) fn reserved_channel_protocol_event_message(
    kind: &str,
    event: &ChannelEventId,
) -> String {
    format!(
        "websocket {kind} event `{event}` is a reserved Foundry protocol event; use a domain-specific event id"
    )
}

fn ensure_app_channel_event(kind: &str, event: &ChannelEventId) -> Result<()> {
    if is_reserved_channel_protocol_event(event) {
        return Err(Error::message(reserved_channel_protocol_event_message(
            kind, event,
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, foundry_macros::AppEnum)]
pub enum ClientAction {
    #[foundry(aliases = ["Subscribe"])]
    Subscribe,
    #[foundry(aliases = ["Unsubscribe"])]
    Unsubscribe,
    #[foundry(aliases = ["Message"])]
    Message,
    #[foundry(aliases = ["ClientEvent"])]
    ClientEvent,
}

#[derive(
    Debug, Clone, Deserialize, PartialEq, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct ClientMessage {
    pub action: ClientAction,
    pub channel: ChannelId,
    #[serde(default)]
    #[ts(optional)]
    pub room: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub payload: Option<serde_json::Value>,
    #[serde(default)]
    #[ts(optional)]
    pub event: Option<ChannelEventId>,
    #[serde(default)]
    #[ts(optional)]
    pub ack_id: Option<String>,
}

impl Serialize for ClientMessage {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut len = 2;
        len += usize::from(self.room.is_some());
        len += usize::from(self.payload.is_some());
        len += usize::from(self.event.is_some());
        len += usize::from(self.ack_id.is_some());

        let mut state = serializer.serialize_struct("ClientMessage", len)?;
        state.serialize_field("action", &self.action)?;
        state.serialize_field("channel", &self.channel)?;
        if let Some(room) = &self.room {
            state.serialize_field("room", room)?;
        }
        if let Some(payload) = &self.payload {
            state.serialize_field("payload", payload)?;
        }
        if let Some(event) = &self.event {
            state.serialize_field("event", event)?;
        }
        if let Some(ack_id) = &self.ack_id {
            state.serialize_field("ack_id", ack_id)?;
        }
        state.end()
    }
}

#[derive(
    Debug, Clone, Deserialize, PartialEq, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct ServerMessage {
    pub channel: ChannelId,
    pub event: ChannelEventId,
    #[serde(default)]
    #[ts(optional)]
    pub room: Option<String>,
    pub payload: serde_json::Value,
}

impl Serialize for ServerMessage {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut len = 3;
        len += usize::from(self.room.is_some());

        let mut state = serializer.serialize_struct("ServerMessage", len)?;
        state.serialize_field("channel", &self.channel)?;
        state.serialize_field("event", &self.event)?;
        if let Some(room) = &self.room {
            state.serialize_field("room", room)?;
        }
        state.serialize_field("payload", &self.payload)?;
        state.end()
    }
}

#[derive(Clone)]
pub struct WebSocketContext {
    app: AppContext,
    connection_id: u64,
    actor: Option<Actor>,
    channel: ChannelId,
    room: Option<String>,
}

impl WebSocketContext {
    pub(crate) fn new(
        app: AppContext,
        connection_id: u64,
        actor: Option<Actor>,
        channel: ChannelId,
        room: Option<String>,
    ) -> Self {
        Self {
            app,
            connection_id,
            actor,
            channel,
            room,
        }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn connection_id(&self) -> u64 {
        self.connection_id
    }

    pub fn actor(&self) -> Option<&Actor> {
        self.actor.as_ref()
    }

    /// Resolve the authenticated actor to its backing model.
    ///
    /// Returns `Ok(None)` if no actor is present on this connection.
    pub async fn resolve_actor<M: Authenticatable>(&self) -> Result<Option<M>> {
        match &self.actor {
            Some(actor) => actor.resolve::<M>(&self.app).await,
            None => Ok(None),
        }
    }

    pub fn channel(&self) -> &ChannelId {
        &self.channel
    }

    pub fn room(&self) -> Option<&str> {
        self.room.as_deref()
    }

    pub async fn publish<I>(&self, event: I, payload: impl Serialize) -> Result<()>
    where
        I: Into<ChannelEventId>,
    {
        self.app
            .websocket()?
            .publish(self.channel.clone(), event, self.room(), payload)
            .await
    }

    /// Return all presence members for the current channel.
    pub async fn presence_members(&self) -> Result<Vec<PresenceInfo>> {
        let backend = RuntimeBackend::from_config(self.app.config())?;
        let key = presence_key(&self.channel);
        let members = backend.smembers(&key).await?;
        let mut infos = Vec::with_capacity(members.len());
        for raw in members {
            if let Ok(info) = serde_json::from_str::<PresenceInfo>(&raw) {
                infos.push(info);
            }
        }
        Ok(infos)
    }

    /// Return the number of presence members for the current channel.
    pub async fn presence_count(&self) -> Result<usize> {
        let backend = RuntimeBackend::from_config(self.app.config())?;
        let key = presence_key(&self.channel);
        backend.scard(&key).await
    }
}

#[async_trait]
pub trait ChannelHandler: Send + Sync + 'static {
    async fn handle(&self, context: WebSocketContext, payload: serde_json::Value) -> Result<()>;
}

#[async_trait]
impl<F, Fut> ChannelHandler for F
where
    F: Fn(WebSocketContext, serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    async fn handle(&self, context: WebSocketContext, payload: serde_json::Value) -> Result<()> {
        (self)(context, payload).await
    }
}

pub struct WebSocketPublisher {
    backend: RuntimeBackend,
    diagnostics: Arc<RuntimeDiagnostics>,
    history_ttl_seconds: u64,
    history_buffer_size: usize,
    channel_registry: RwLock<Option<Arc<WebSocketChannelRegistry>>>,
}

impl Clone for WebSocketPublisher {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            diagnostics: self.diagnostics.clone(),
            history_ttl_seconds: self.history_ttl_seconds,
            history_buffer_size: self.history_buffer_size,
            channel_registry: RwLock::new(
                read_unpoisoned(
                    &self.channel_registry,
                    "websocket publisher channel registry",
                )
                .clone(),
            ),
        }
    }
}

impl WebSocketPublisher {
    pub(crate) fn new(
        backend: RuntimeBackend,
        diagnostics: Arc<RuntimeDiagnostics>,
        history_ttl_seconds: u64,
        history_buffer_size: usize,
    ) -> Self {
        Self {
            backend,
            diagnostics,
            history_ttl_seconds,
            history_buffer_size: history_buffer_size.max(1),
            channel_registry: RwLock::new(None),
        }
    }

    pub(crate) fn attach_channel_registry(&self, registry: Arc<WebSocketChannelRegistry>) {
        *write_unpoisoned(
            &self.channel_registry,
            "websocket publisher channel registry",
        ) = Some(registry);
    }

    pub async fn publish<C, E>(
        &self,
        channel: C,
        event: E,
        room: Option<&str>,
        payload: impl Serialize,
    ) -> Result<()>
    where
        C: Into<ChannelId>,
        E: Into<ChannelEventId>,
    {
        self.publish_message(ServerMessage {
            channel: channel.into(),
            event: event.into(),
            room: room.map(ToOwned::to_owned),
            payload: serde_json::to_value(payload).map_err(Error::other)?,
        })
        .await
    }

    pub async fn publish_message(&self, message: ServerMessage) -> Result<()> {
        self.validate_publish_event(&message)?;

        let payload = serde_json::to_string(&message).map_err(Error::other)?;
        self.diagnostics
            .record_websocket_outbound_message_on(&message.channel);
        self.backend
            .publish_ws(message.channel.as_str(), &payload)
            .await?;

        // Buffer for replay so new subscribers can catch up on recent messages.
        let history_key = format!("ws:history:{}", message.channel);
        let _ = self
            .backend
            .lpush_capped(&history_key, &payload, self.history_buffer_size)
            .await;
        if self.history_ttl_seconds > 0 {
            let _ = self
                .backend
                .expire(&history_key, self.history_ttl_seconds)
                .await;
        }

        Ok(())
    }

    fn validate_publish_event(&self, message: &ServerMessage) -> Result<()> {
        ensure_app_channel_event("server", &message.event)?;

        let registry = read_unpoisoned(
            &self.channel_registry,
            "websocket publisher channel registry",
        )
        .clone();
        let Some(registry) = registry else {
            return Ok(());
        };
        let Some(descriptor) = registry.find(&message.channel) else {
            return Ok(());
        };

        if descriptor.server_events.is_empty()
            || descriptor
                .server_events
                .iter()
                .any(|event| event == &message.event)
        {
            return Ok(());
        }

        Err(Error::message(format!(
            "websocket channel `{}` does not document server event `{}`",
            message.channel, message.event
        )))
    }

    /// Force disconnect all connections for a specific user (across all instances).
    pub async fn disconnect_user(&self, actor_id: &str) -> Result<()> {
        let command = serde_json::json!({
            "type": "disconnect_user",
            "actor_id": actor_id,
        });
        self.backend
            .publish_ws("__system:disconnect", &command.to_string())
            .await
    }
}

pub struct WebSocketRegistrar {
    channels: HashMap<ChannelId, RegisteredChannel>,
}

/// Type for channel lifecycle callbacks (on_join / on_leave).
pub type LifecycleCallback =
    Arc<dyn Fn(WebSocketContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;

/// Type for dynamic per-subscription authorization callbacks.
pub type AuthorizeCallback = Arc<
    dyn Fn(
            WebSocketContext,
            ChannelId,
            Option<String>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone, Default)]
pub struct WebSocketChannelOptions {
    pub access: AccessScope,
    pub presence: bool,
    pub(crate) authorize: Option<AuthorizeCallback>,
    pub(crate) allow_client_events: bool,
    pub(crate) client_events: BTreeSet<ChannelEventId>,
    pub(crate) server_events: BTreeSet<ChannelEventId>,
    pub(crate) on_join: Option<LifecycleCallback>,
    pub(crate) on_leave: Option<LifecycleCallback>,
    pub(crate) replay_count: u32,
}

impl WebSocketChannelOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn presence(mut self, enabled: bool) -> Self {
        self.presence = enabled;
        self
    }

    pub fn guard<I>(mut self, guard: I) -> Self
    where
        I: Into<GuardId>,
    {
        self.access = self.access.with_guard(guard);
        self
    }

    pub fn permission<I>(mut self, permission: I) -> Self
    where
        I: Into<PermissionId>,
    {
        self.access = self.access.with_permission(permission);
        self
    }

    pub fn permissions<I, P>(mut self, permissions: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PermissionId>,
    {
        self.access = self.access.with_permissions(permissions);
        self
    }

    /// Add a dynamic authorization callback for subscription requests.
    ///
    /// Called after guard/permission checks. Return `Ok(())` to allow,
    /// `Err(...)` to reject.
    ///
    /// ```ignore
    /// WebSocketChannelOptions::new()
    ///     .guard(AuthGuard::Api)
    ///     .authorize(|ctx, channel, room| async move {
    ///         let actor = ctx.actor().ok_or(Error::unauthorized("auth required"))?;
    ///         // Custom logic...
    ///         Ok(())
    ///     })
    /// ```
    pub fn authorize<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(WebSocketContext, ChannelId, Option<String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.authorize = Some(wrap_authorize_callback(f));
        self
    }

    /// Allow clients to send events that are relayed to other subscribers.
    pub fn allow_client_events(mut self, enabled: bool) -> Self {
        self.allow_client_events = enabled;
        self
    }

    /// Allow clients to relay a specific event id on this channel.
    ///
    /// Registering one or more client events narrows accepted client-event
    /// frames to that allowlist and enables client-event relay for the channel.
    pub fn client_event<I>(mut self, event: I) -> Self
    where
        I: Into<ChannelEventId>,
    {
        self.allow_client_events = true;
        self.client_events.insert(event.into());
        self
    }

    /// Allow clients to relay the provided event ids on this channel.
    ///
    /// If no client events are registered, `allow_client_events(true)` keeps the
    /// legacy open event-id behavior for compatibility.
    pub fn client_events<I, E>(mut self, events: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: Into<ChannelEventId>,
    {
        self.allow_client_events = true;
        self.client_events
            .extend(events.into_iter().map(Into::into));
        self
    }

    /// Document that the backend may publish a specific event id on this channel.
    pub fn server_event<I>(mut self, event: I) -> Self
    where
        I: Into<ChannelEventId>,
    {
        self.server_events.insert(event.into());
        self
    }

    /// Document that the backend may publish the provided event ids on this channel.
    pub fn server_events<I, E>(mut self, events: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: Into<ChannelEventId>,
    {
        self.server_events
            .extend(events.into_iter().map(Into::into));
        self
    }

    /// Register a callback invoked when a client subscribes to this channel.
    pub fn on_join<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(WebSocketContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on_join = Some(wrap_lifecycle_callback("on_join", f));
        self
    }

    /// Register a callback invoked when a client unsubscribes from this channel.
    pub fn on_leave<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(WebSocketContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on_leave = Some(wrap_lifecycle_callback("on_leave", f));
        self
    }

    /// Enable message replay for new subscribers on this channel.
    ///
    /// When a client subscribes, the last `count` messages are sent before
    /// the `SUBSCRIBED` event so the client can catch up on recent activity.
    /// Set to `0` (the default) to disable replay.
    pub fn replay(mut self, count: u32) -> Self {
        self.replay_count = count;
        self
    }

    pub(crate) fn requires_auth(&self) -> bool {
        self.access.requires_auth()
    }

    pub(crate) fn guard_id(&self) -> Option<&GuardId> {
        self.access.guard()
    }

    pub(crate) fn permissions_set(&self) -> std::collections::BTreeSet<PermissionId> {
        self.access.permissions()
    }
}

fn wrap_authorize_callback<F, Fut>(f: F) -> AuthorizeCallback
where
    F: Fn(WebSocketContext, ChannelId, Option<String>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    Arc::new(move |ctx, channel, room| {
        match catch_sync_panic(|| f(ctx, channel.clone(), room.clone())) {
            Ok(future) => Box::pin(async move {
                match catch_future_panic(future).await {
                    Ok(result) => result,
                    Err(panic) => Err(websocket_authorizer_panic_error(panic)),
                }
            }),
            Err(panic) => Box::pin(async move { Err(websocket_authorizer_panic_error(panic)) }),
        }
    })
}

fn wrap_lifecycle_callback<F, Fut>(hook: &'static str, f: F) -> LifecycleCallback
where
    F: Fn(WebSocketContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    Arc::new(move |ctx| match catch_sync_panic(|| f(ctx)) {
        Ok(future) => Box::pin(async move {
            match catch_future_panic(future).await {
                Ok(result) => result,
                Err(panic) => Err(websocket_lifecycle_panic_error(hook, panic)),
            }
        }),
        Err(panic) => Box::pin(async move { Err(websocket_lifecycle_panic_error(hook, panic)) }),
    })
}

fn websocket_lifecycle_panic_error(
    hook: &'static str,
    panic: Box<dyn std::any::Any + Send>,
) -> Error {
    let message = panic_payload_message(panic);
    Error::message(format!("websocket {hook} hook panicked: {message}"))
}

fn websocket_authorizer_panic_error(panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.websocket",
        panic = %message,
        "websocket authorizer panicked"
    );
    Error::message(format!("websocket authorizer panicked: {message}"))
}

impl Default for WebSocketRegistrar {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketRegistrar {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }

    pub fn channel<I, H>(&mut self, id: I, handler: H) -> Result<&mut Self>
    where
        I: Into<ChannelId>,
        H: ChannelHandler,
    {
        self.channel_with_options(id, handler, WebSocketChannelOptions::default())
    }

    pub fn channel_with_options<I, H>(
        &mut self,
        id: I,
        handler: H,
        options: WebSocketChannelOptions,
    ) -> Result<&mut Self>
    where
        I: Into<ChannelId>,
        H: ChannelHandler,
    {
        let id = id.into();
        if self.channels.contains_key(&id) {
            return Err(Error::message(format!(
                "websocket channel `{id}` already registered"
            )));
        }
        for event in &options.client_events {
            ensure_app_channel_event("client", event)?;
        }
        for event in &options.server_events {
            ensure_app_channel_event("server", event)?;
        }

        self.channels.insert(
            id.clone(),
            RegisteredChannel {
                id,
                options,
                handler: Arc::new(handler),
            },
        );
        Ok(self)
    }

    pub(crate) fn into_channels(self) -> Vec<RegisteredChannel> {
        self.channels.into_values().collect()
    }
}

#[derive(Clone)]
pub(crate) struct RegisteredChannel {
    pub id: ChannelId,
    pub options: WebSocketChannelOptions,
    pub handler: Arc<dyn ChannelHandler>,
}

/// Public projection of a registered WebSocket channel's configuration.
///
/// Emitted by the `/_foundry/ws/channels` dashboard endpoint and returned
/// from [`AppContext::websocket_channels`](crate::foundation::AppContext::websocket_channels).
#[derive(Debug, Clone, Serialize, ts_rs::TS, foundry_macros::TS)]
pub struct WebSocketChannelDescriptor {
    pub id: ChannelId,
    pub presence: bool,
    #[ts(type = "number")]
    pub replay_count: u32,
    pub allow_client_events: bool,
    pub client_events: Vec<ChannelEventId>,
    pub server_events: Vec<ChannelEventId>,
    pub requires_auth: bool,
    pub guard: Option<GuardId>,
    pub permissions: Vec<PermissionId>,
}

impl crate::openapi::ApiSchema for WebSocketChannelDescriptor {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "presence": { "type": "boolean" },
                "replay_count": { "type": "integer" },
                "allow_client_events": { "type": "boolean" },
                "client_events": {
                    "type": "array",
                    "items": { "type": "string" },
                },
                "server_events": {
                    "type": "array",
                    "items": { "type": "string" },
                },
                "requires_auth": { "type": "boolean" },
                "guard": { "type": "string", "nullable": true },
                "permissions": {
                    "type": "array",
                    "items": { "type": "string" },
                },
            },
            "required": [
                "id",
                "presence",
                "replay_count",
                "allow_client_events",
                "client_events",
                "server_events",
                "requires_auth",
                "guard",
                "permissions",
            ],
        })
    }

    fn schema_name() -> &'static str {
        "WebSocketChannelDescriptor"
    }
}

impl From<&RegisteredChannel> for WebSocketChannelDescriptor {
    fn from(channel: &RegisteredChannel) -> Self {
        Self {
            id: channel.id.clone(),
            presence: channel.options.presence,
            replay_count: channel.options.replay_count,
            allow_client_events: channel.options.allow_client_events,
            client_events: channel.options.client_events.iter().cloned().collect(),
            server_events: channel.options.server_events.iter().cloned().collect(),
            requires_auth: channel.options.requires_auth(),
            guard: channel.options.guard_id().cloned(),
            permissions: channel.options.permissions_set().into_iter().collect(),
        }
    }
}

/// Shared registry of `RegisteredChannel` entries, stored in the `AppContext`
/// container so both the WebSocket kernel and dashboard handlers read from the
/// same source of truth.
#[derive(Clone, Default)]
pub struct WebSocketChannelRegistry {
    channels: Arc<Vec<RegisteredChannel>>,
}

impl std::fmt::Debug for WebSocketChannelRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketChannelRegistry")
            .field("channel_count", &self.channels.len())
            .finish()
    }
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
        self.channels.iter().find(|c| c.id == *id).map(Into::into)
    }

    pub(crate) fn registered_channels(&self) -> &[RegisteredChannel] {
        &self.channels
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;
    use std::sync::Arc;

    use super::{
        ChannelEventId, ChannelId, ClientAction, ClientMessage, GuardId, PermissionId,
        WebSocketChannelOptions, WebSocketChannelRegistry, WebSocketRegistrar,
        WebSocketRouteRegistrar, PRESENCE_JOIN_EVENT, SUBSCRIBED_EVENT,
    };
    use crate::auth::Actor;
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::logging::{ReadinessRegistryBuilder, RuntimeBackendKind, RuntimeDiagnostics};
    use crate::support::runtime::RuntimeBackend;
    use crate::validation::RuleRegistry;

    fn websocket_context() -> super::WebSocketContext {
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();
        super::WebSocketContext::new(
            app,
            1,
            Some(Actor::new("user-1", GuardId::new("api"))),
            ChannelId::new("chat"),
            None,
        )
    }

    #[test]
    fn rejects_duplicate_channel_registration() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(ChannelId::new("chat"), |_context, _payload| async {
                Ok(())
            })
            .unwrap();

        let error = registrar
            .channel(ChannelId::new("chat"), |_context, _payload| async {
                Ok(())
            })
            .err()
            .unwrap();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn rejects_reserved_protocol_events_in_channel_options() {
        let mut registrar = WebSocketRegistrar::new();
        let server_error = registrar
            .channel_with_options(
                ChannelId::new("reserved_server"),
                |_context, _payload| async { Ok(()) },
                WebSocketChannelOptions::new().server_event(SUBSCRIBED_EVENT),
            )
            .err()
            .unwrap();
        assert!(server_error
            .to_string()
            .contains("reserved Foundry protocol event"));

        let mut registrar = WebSocketRegistrar::new();
        let client_error = registrar
            .channel_with_options(
                ChannelId::new("reserved_client"),
                |_context, _payload| async { Ok(()) },
                WebSocketChannelOptions::new().client_event(PRESENCE_JOIN_EVENT),
            )
            .err()
            .unwrap();
        assert!(client_error
            .to_string()
            .contains("reserved Foundry protocol event"));
    }

    #[test]
    fn websocket_publisher_rejects_reserved_protocol_server_events() {
        let publisher = super::WebSocketPublisher::new(
            RuntimeBackend::memory("websocket-reserved-protocol-events"),
            Arc::new(RuntimeDiagnostics::new(
                RuntimeBackendKind::Memory,
                ReadinessRegistryBuilder::freeze_shared(ReadinessRegistryBuilder::shared()),
            )),
            0,
            1,
        );

        let error = publisher
            .validate_publish_event(&super::ServerMessage {
                channel: ChannelId::new("chat"),
                event: SUBSCRIBED_EVENT,
                room: None,
                payload: serde_json::Value::Null,
            })
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("reserved Foundry protocol event"));
    }

    #[test]
    fn websocket_registrar_panic_becomes_error() {
        let registrars: Vec<WebSocketRouteRegistrar> = vec![Arc::new(|_| {
            panic!("websocket registrar explode");
        })];

        let error = match super::build_registrar(&registrars) {
            Ok(_) => panic!("expected websocket registrar panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "websocket registrar panicked: websocket registrar explode"
        );
    }

    #[test]
    fn client_action_uses_canonical_keys_and_legacy_aliases() {
        assert_eq!(
            serde_json::to_string(&ClientAction::ClientEvent).unwrap(),
            "\"client_event\""
        );
        assert_eq!(
            serde_json::from_str::<ClientAction>("\"ClientEvent\"").unwrap(),
            ClientAction::ClientEvent
        );

        let message: ClientMessage =
            serde_json::from_value(serde_json::json!({ "action": "Subscribe", "channel": "chat" }))
                .unwrap();
        assert_eq!(message.action, ClientAction::Subscribe);
        assert_eq!(message.channel, ChannelId::new("chat"));

        let client_frame = serde_json::to_value(ClientMessage {
            action: ClientAction::Subscribe,
            channel: ChannelId::new("chat"),
            room: None,
            payload: None,
            event: None,
            ack_id: None,
        })
        .unwrap();
        assert_eq!(
            client_frame,
            serde_json::json!({ "action": "subscribe", "channel": "chat" })
        );

        let server_frame = serde_json::to_value(super::ServerMessage {
            channel: ChannelId::new("chat"),
            event: ChannelEventId::new("message"),
            room: None,
            payload: serde_json::json!({ "body": "hello" }),
        })
        .unwrap();
        assert_eq!(
            server_frame,
            serde_json::json!({
                "channel": "chat",
                "event": "message",
                "payload": { "body": "hello" },
            })
        );
    }

    #[tokio::test]
    async fn websocket_authorize_future_panic_becomes_error() {
        let options = WebSocketChannelOptions::new().authorize(|_ctx, _channel, _room| async {
            let should_panic = true;
            if should_panic {
                panic!("ws auth boom");
            }
            Ok(())
        });
        let authorize = options.authorize.unwrap();

        let error = authorize(websocket_context(), ChannelId::new("chat"), None)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("websocket authorizer panicked: ws auth boom"));
    }

    #[tokio::test]
    async fn websocket_authorize_factory_panic_becomes_error() {
        let options = WebSocketChannelOptions::new().authorize(|_ctx, _channel, _room| {
            if std::hint::black_box(true) {
                panic!("ws auth factory boom");
            }
            ready(Ok(()))
        });
        let authorize = options.authorize.unwrap();

        let error = authorize(websocket_context(), ChannelId::new("chat"), None)
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("websocket authorizer panicked: ws auth factory boom"));
    }

    #[tokio::test]
    async fn websocket_on_join_future_panic_becomes_error() {
        let options = WebSocketChannelOptions::new().on_join(|_ctx| async {
            let should_panic = true;
            if should_panic {
                panic!("join hook boom");
            }
            Ok(())
        });
        let on_join = options.on_join.unwrap();

        let error = on_join(websocket_context()).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("websocket on_join hook panicked: join hook boom"));
    }

    #[tokio::test]
    async fn websocket_on_leave_factory_panic_becomes_error() {
        let options = WebSocketChannelOptions::new().on_leave(|_ctx| {
            if std::hint::black_box(true) {
                panic!("leave hook factory boom");
            }
            ready(Ok(()))
        });
        let on_leave = options.on_leave.unwrap();

        let error = on_leave(websocket_context()).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("websocket on_leave hook panicked: leave hook factory boom"));
    }

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
                    .client_event(ChannelEventId::new("typing"))
                    .server_events([ChannelEventId::new("message"), ChannelEventId::new("ack")])
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
        assert_eq!(
            descriptor.client_events,
            vec![ChannelEventId::new("typing")]
        );
        assert_eq!(
            descriptor.server_events,
            vec![ChannelEventId::new("ack"), ChannelEventId::new("message")]
        );
        assert!(descriptor.requires_auth);
        assert_eq!(descriptor.guard.as_ref(), Some(&GuardId::new("api")));
        assert_eq!(descriptor.permissions, vec![PermissionId::new("chat:read")]);

        assert!(registry.find(&ChannelId::new("chat")).is_some());
        assert!(registry.find(&ChannelId::new("missing")).is_none());
    }
}
