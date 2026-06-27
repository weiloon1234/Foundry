use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::{AccessScope, Actor, Authenticatable};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{
    catch_future_panic, catch_sync_panic, panic_payload_message, RuntimeDiagnostics,
};
use crate::openapi::{ApiSchema, SchemaRef};
use crate::support::runtime::RuntimeBackend;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub actor_id: String,
    pub channel: ChannelId,
    pub joined_at: i64,
}

pub const SYSTEM_CHANNEL: ChannelId = ChannelId::new("system");
pub const ERROR_EVENT: ChannelEventId = ChannelEventId::new("error");
pub const SUBSCRIBED_EVENT: ChannelEventId = ChannelEventId::new("subscribed");
pub const UNSUBSCRIBED_EVENT: ChannelEventId = ChannelEventId::new("unsubscribed");
pub const PRESENCE_JOIN_EVENT: ChannelEventId = ChannelEventId::new("presence:join");
pub const PRESENCE_LEAVE_EVENT: ChannelEventId = ChannelEventId::new("presence:leave");
pub const ACK_EVENT: ChannelEventId = ChannelEventId::new("ack");

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientAction {
    Subscribe,
    Unsubscribe,
    Message,
    ClientEvent,
}

impl<'de> Deserialize<'de> for ClientAction {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "subscribe" | "Subscribe" => Ok(Self::Subscribe),
            "unsubscribe" | "Unsubscribe" => Ok(Self::Unsubscribe),
            "message" | "Message" => Ok(Self::Message),
            "client_event" | "ClientEvent" => Ok(Self::ClientEvent),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &[
                    "subscribe",
                    "unsubscribe",
                    "message",
                    "client_event",
                    "Subscribe",
                    "Unsubscribe",
                    "Message",
                    "ClientEvent",
                ],
            )),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ClientMessage {
    pub action: ClientAction,
    pub channel: ChannelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ChannelEventId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ServerMessage {
    pub channel: ChannelId,
    pub event: ChannelEventId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    pub payload: serde_json::Value,
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

#[derive(Clone)]
pub struct WebSocketPublisher {
    backend: RuntimeBackend,
    diagnostics: Arc<RuntimeDiagnostics>,
    history_ttl_seconds: u64,
    history_buffer_size: usize,
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
        }
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketEventDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone)]
pub(crate) struct WebSocketChannelEventContract {
    pub direction: WebSocketEventDirection,
    pub event: ChannelEventId,
    pub payload: Option<SchemaRef>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WebSocketChannelEventDescriptor {
    pub event: ChannelEventId,
    pub payload: Option<&'static str>,
}

#[derive(Clone, Default)]
pub struct WebSocketChannelOptions {
    pub access: AccessScope,
    pub presence: bool,
    pub(crate) authorize: Option<AuthorizeCallback>,
    pub(crate) allow_client_events: bool,
    pub(crate) on_join: Option<LifecycleCallback>,
    pub(crate) on_leave: Option<LifecycleCallback>,
    pub(crate) replay_count: u32,
    pub(crate) event_contracts: Vec<WebSocketChannelEventContract>,
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

    /// Declare a client-to-server event and its payload schema for frontend contracts.
    pub fn incoming_event<I, T>(mut self, event: I) -> Self
    where
        I: Into<ChannelEventId>,
        T: ApiSchema,
    {
        self.event_contracts.push(WebSocketChannelEventContract {
            direction: WebSocketEventDirection::Incoming,
            event: event.into(),
            payload: Some(SchemaRef::of::<T>()),
        });
        self
    }

    /// Declare a client-to-server event that does not send a payload.
    pub fn incoming_event_without_payload<I>(mut self, event: I) -> Self
    where
        I: Into<ChannelEventId>,
    {
        self.event_contracts.push(WebSocketChannelEventContract {
            direction: WebSocketEventDirection::Incoming,
            event: event.into(),
            payload: None,
        });
        self
    }

    /// Declare a server-to-client event and its payload schema for frontend contracts.
    pub fn outgoing_event<I, T>(mut self, event: I) -> Self
    where
        I: Into<ChannelEventId>,
        T: ApiSchema,
    {
        self.event_contracts.push(WebSocketChannelEventContract {
            direction: WebSocketEventDirection::Outgoing,
            event: event.into(),
            payload: Some(SchemaRef::of::<T>()),
        });
        self
    }

    /// Declare a server-to-client event that does not send a payload.
    pub fn outgoing_event_without_payload<I>(mut self, event: I) -> Self
    where
        I: Into<ChannelEventId>,
    {
        self.event_contracts.push(WebSocketChannelEventContract {
            direction: WebSocketEventDirection::Outgoing,
            event: event.into(),
            payload: None,
        });
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
#[derive(Debug, Clone, Serialize)]
pub struct WebSocketChannelDescriptor {
    pub id: ChannelId,
    pub presence: bool,
    pub replay_count: u32,
    pub allow_client_events: bool,
    pub requires_auth: bool,
    pub guard: Option<GuardId>,
    pub permissions: Vec<PermissionId>,
    pub incoming: Vec<WebSocketChannelEventDescriptor>,
    pub outgoing: Vec<WebSocketChannelEventDescriptor>,
}

impl From<&RegisteredChannel> for WebSocketChannelDescriptor {
    fn from(channel: &RegisteredChannel) -> Self {
        let event_descriptor =
            |contract: &WebSocketChannelEventContract| WebSocketChannelEventDescriptor {
                event: contract.event.clone(),
                payload: contract.payload.as_ref().map(|schema| schema.name),
            };

        Self {
            id: channel.id.clone(),
            presence: channel.options.presence,
            replay_count: channel.options.replay_count,
            allow_client_events: channel.options.allow_client_events,
            requires_auth: channel.options.requires_auth(),
            guard: channel.options.guard_id().cloned(),
            permissions: channel.options.permissions_set().into_iter().collect(),
            incoming: channel
                .options
                .event_contracts
                .iter()
                .filter(|contract| contract.direction == WebSocketEventDirection::Incoming)
                .map(event_descriptor)
                .collect(),
            outgoing: channel
                .options
                .event_contracts
                .iter()
                .filter(|contract| contract.direction == WebSocketEventDirection::Outgoing)
                .map(event_descriptor)
                .collect(),
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
        ChannelEventId, ChannelId, GuardId, PermissionId, WebSocketChannelOptions,
        WebSocketChannelRegistry, WebSocketRegistrar, WebSocketRouteRegistrar,
    };
    use crate::auth::Actor;
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
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
                    .allow_client_events(true)
                    .guard(GuardId::new("api"))
                    .permissions([PermissionId::new("chat:read")])
                    .incoming_event::<_, String>(ChannelEventId::new("send"))
                    .incoming_event_without_payload(ChannelEventId::new("typing"))
                    .outgoing_event::<_, String>(ChannelEventId::new("message"))
                    .outgoing_event_without_payload(ChannelEventId::new("pong")),
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
        assert_eq!(descriptor.permissions, vec![PermissionId::new("chat:read")]);
        assert_eq!(descriptor.incoming.len(), 2);
        assert_eq!(descriptor.incoming[0].event, ChannelEventId::new("send"));
        assert_eq!(descriptor.incoming[0].payload, Some("String"));
        assert_eq!(descriptor.incoming[1].event, ChannelEventId::new("typing"));
        assert_eq!(descriptor.incoming[1].payload, None);
        assert_eq!(descriptor.outgoing.len(), 2);
        assert_eq!(descriptor.outgoing[0].event, ChannelEventId::new("message"));
        assert_eq!(descriptor.outgoing[0].payload, Some("String"));
        assert_eq!(descriptor.outgoing[1].event, ChannelEventId::new("pong"));
        assert_eq!(descriptor.outgoing[1].payload, None);

        assert!(registry.find(&ChannelId::new("chat")).is_some());
        assert!(registry.find(&ChannelId::new("missing")).is_none());
    }
}
