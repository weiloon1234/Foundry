use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;

use crate::auth::{Actor, AuthError, AuthErrorCode};
use crate::config::WebSocketConfig;
use crate::foundation::{AppContext, Error, ErrorResponse, Result};
use crate::logging::{
    catch_async_panic, catch_future_panic, panic_payload_message, AuthOutcome, RuntimeDiagnostics,
    WebSocketConnectionState,
};
use crate::support::runtime::RuntimeBackend;
use crate::support::sync::lock_unpoisoned;
use crate::support::{ChannelEventId, ChannelId, GuardId};
use crate::websocket::{
    is_reserved_channel_protocol_event, presence_key, presence_member_value,
    reserved_channel_protocol_event_message, ClientAction, ClientMessage, RegisteredChannel,
    ServerMessage, WebSocketAckPayload, WebSocketContext, WebSocketPresenceJoinPayload,
    WebSocketPresenceLeavePayload, ACK_EVENT, ERROR_EVENT, PRESENCE_JOIN_EVENT,
    PRESENCE_LEAVE_EVENT, SUBSCRIBED_EVENT, SYSTEM_CHANNEL, UNSUBSCRIBED_EVENT,
};

pub struct WebSocketKernel {
    app: AppContext,
}

impl WebSocketKernel {
    pub fn new(app: AppContext) -> Self {
        Self { app }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub async fn bind(self) -> Result<BoundWebSocketServer> {
        let websocket = self.app.config().websocket()?;
        let addr = format!("{}:{}", websocket.host, websocket.port);
        let listener = TcpListener::bind(addr).await.map_err(Error::other)?;
        let local_addr = listener.local_addr().map_err(Error::other)?;
        let (router, pubsub_task) = self.build_router().await?;

        Ok(BoundWebSocketServer {
            listener,
            router,
            local_addr,
            pubsub_task,
        })
    }

    pub async fn serve(self) -> Result<()> {
        self.bind().await?.serve().await
    }

    async fn build_router(&self) -> Result<(axum::Router, Option<WebSocketPubSubTask>)> {
        let ws_config = self.app.config().websocket()?;
        validate_query_token_config(&ws_config)?;
        let registry = self
            .app
            .container()
            .resolve::<crate::websocket::WebSocketChannelRegistry>()?;
        let registered_channels: Vec<RegisteredChannel> = registry.registered_channels().to_vec();
        let backend = RuntimeBackend::from_config(self.app.config())?;
        let state =
            WebSocketServerState::new(self.app.clone(), registered_channels, backend, ws_config);
        let pubsub_task = state.start_pubsub().await?;

        let router = axum::Router::new()
            .route(&state.ws_config.path, get(websocket_handler))
            .with_state(state);
        Ok((router, pubsub_task))
    }
}

pub struct BoundWebSocketServer {
    listener: TcpListener,
    router: axum::Router,
    local_addr: SocketAddr,
    pubsub_task: Option<WebSocketPubSubTask>,
}

impl BoundWebSocketServer {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn serve(self) -> Result<()> {
        self.serve_until(super::shutdown::shutdown_signal()).await
    }

    async fn serve_until<S>(self, shutdown: S) -> Result<()>
    where
        S: Future<Output = ()> + Send + 'static,
    {
        let Self {
            listener,
            router,
            pubsub_task,
            ..
        } = self;
        let result = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(Error::other);
        drop(pubsub_task);
        result
    }
}

struct WebSocketPubSubTask {
    shutdown: StdMutex<Option<oneshot::Sender<()>>>,
    handle: StdMutex<Option<JoinHandle<()>>>,
}

impl WebSocketPubSubTask {
    fn new(shutdown: oneshot::Sender<()>, handle: JoinHandle<()>) -> Self {
        Self {
            shutdown: StdMutex::new(Some(shutdown)),
            handle: StdMutex::new(Some(handle)),
        }
    }
}

impl Drop for WebSocketPubSubTask {
    fn drop(&mut self) {
        if let Some(shutdown) = lock_unpoisoned(&self.shutdown, "websocket pubsub shutdown").take()
        {
            let _ = shutdown.send(());
        }
        if let Some(handle) = lock_unpoisoned(&self.handle, "websocket pubsub task").take() {
            // Dropping a JoinHandle detaches the task; abort so a task stuck
            // in a backend call cannot outlive the server.
            handle.abort();
        }
    }
}

/// Commands sent to the per-connection writer task.
enum WriterCommand {
    Json(ServerMessage),
    Ping,
    Pong(Bytes),
    Close,
}

#[derive(Clone)]
struct WebSocketServerState {
    app: AppContext,
    channels: Arc<HashMap<ChannelId, RegisteredChannel>>,
    hub: ConnectionHub,
    backend: RuntimeBackend,
    ws_config: WebSocketConfig,
}

impl WebSocketServerState {
    fn new(
        app: AppContext,
        channels: Vec<RegisteredChannel>,
        backend: RuntimeBackend,
        ws_config: WebSocketConfig,
    ) -> Self {
        let map = channels
            .into_iter()
            .map(|channel| (channel.id.clone(), channel))
            .collect::<HashMap<_, _>>();
        let diagnostics = app.diagnostics().ok();
        let outbound_buffer_size = ws_config.outbound_buffer_size;
        Self {
            app,
            channels: Arc::new(map),
            hub: ConnectionHub::new(diagnostics, outbound_buffer_size),
            backend,
            ws_config,
        }
    }

    async fn start_pubsub(&self) -> Result<Option<WebSocketPubSubTask>> {
        if self.channels.is_empty() {
            return Ok(None);
        }

        let backend = self.backend.clone();
        let mut topics = self
            .channels
            .keys()
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();

        // Subscribe to the system disconnect topic for force-disconnect support.
        topics.push("__system:disconnect".to_string());

        let server_state = self.clone();
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let task = async move {
                const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
                const MAX_BACKOFF: Duration = Duration::from_secs(30);
                let mut backoff = INITIAL_BACKOFF;

                'resubscribe: loop {
                    let mut subscription = tokio::select! {
                        _ = &mut shutdown_rx => return,
                        subscription = backend.subscribe_ws(&topics) => match subscription {
                            Ok(subscription) => subscription,
                            Err(error) => {
                                tracing::error!(
                                    target: "foundry.websocket",
                                    error = %error,
                                    retry_in_ms = backoff.as_millis() as u64,
                                    "websocket pubsub subscribe failed; retrying"
                                );
                                tokio::select! {
                                    _ = &mut shutdown_rx => return,
                                    _ = tokio::time::sleep(backoff) => {}
                                }
                                backoff = (backoff * 2).min(MAX_BACKOFF);
                                continue 'resubscribe;
                            }
                        }
                    };
                    backoff = INITIAL_BACKOFF;

                    loop {
                        let message = tokio::select! {
                            _ = &mut shutdown_rx => return,
                            message = subscription.recv() => message,
                        };
                        let Some(message) = message else {
                            // The subscription ended (e.g. the backend connection
                            // dropped). Resubscribe instead of going silent while
                            // publishes keep succeeding with no one listening.
                            tracing::warn!(
                                target: "foundry.websocket",
                                "websocket pubsub subscription ended; resubscribing"
                            );
                            continue 'resubscribe;
                        };

                        // Handle force-disconnect commands on the system topic.
                        if message.topic == "__system:disconnect" {
                            #[derive(serde::Deserialize)]
                            struct DisconnectCommand {
                                actor_id: String,
                            }
                            if let Ok(cmd) =
                                serde_json::from_str::<DisconnectCommand>(&message.payload)
                            {
                                let closed =
                                    server_state.hub.disconnect_by_actor(&cmd.actor_id).await;
                                server_state.cleanup_closed_connections(closed).await;
                            } else {
                                tracing::error!(
                                    "foundry websocket pubsub: invalid disconnect command payload"
                                );
                            }
                            continue;
                        }

                        let envelope = match serde_json::from_str::<ServerMessage>(&message.payload)
                        {
                            Ok(envelope) => envelope,
                            Err(error) => {
                                tracing::error!("foundry websocket pubsub decode failed: {error}");
                                continue;
                            }
                        };
                        server_state.broadcast(&envelope).await;
                    }
                }
            };

            if let Err(panic) = catch_future_panic(task).await {
                tracing::error!(
                    target: "foundry.websocket",
                    panic = %panic_payload_message(panic),
                    "websocket pubsub task panicked"
                );
            }
        });

        Ok(Some(WebSocketPubSubTask::new(shutdown_tx, handle)))
    }

    async fn capture_identity(&self, headers: &HeaderMap) -> ConnectionIdentity {
        let Ok(auth) = self.app.auth() else {
            return ConnectionIdentity {
                bearer_token: None,
                session_id: None,
                auth_error: Some(AuthError::internal("auth manager is not available")),
                client_ip: None,
            };
        };

        // Try bearer token first (Authorization header)
        if headers.contains_key(axum::http::header::AUTHORIZATION) {
            return match auth.extract_token(headers) {
                Ok(token) => ConnectionIdentity {
                    bearer_token: Some(token),
                    session_id: None,
                    auth_error: None,
                    client_ip: None,
                },
                Err(error) => ConnectionIdentity {
                    bearer_token: None,
                    session_id: None,
                    auth_error: Some(error),
                    client_ip: None,
                },
            };
        }

        // Fall back to session cookie
        if let Ok(sessions) = self.app.sessions() {
            if let Some(sid) = sessions.extract_session_id(headers) {
                return ConnectionIdentity {
                    bearer_token: None,
                    session_id: Some(sid),
                    auth_error: None,
                    client_ip: None,
                };
            }
        }

        ConnectionIdentity::default()
    }

    async fn authorize_channel(
        &self,
        connection_id: u64,
        channel: &RegisteredChannel,
    ) -> std::result::Result<Option<Actor>, AuthError> {
        if !channel.options.requires_auth() {
            return Ok(None);
        }

        let auth = match self.app.auth() {
            Ok(auth) => auth,
            Err(error) => {
                self.record_auth_outcome(AuthOutcome::Error);
                return Err(AuthError::internal(error.to_string()));
            }
        };
        let authorizer = match self.app.authorizer() {
            Ok(authorizer) => authorizer,
            Err(error) => {
                self.record_auth_outcome(AuthOutcome::Error);
                return Err(AuthError::internal(error.to_string()));
            }
        };
        let guard_id = channel
            .options
            .guard_id()
            .cloned()
            .unwrap_or_else(|| auth.default_guard().clone());

        if let Some(actor) = self.hub.cached_actor(connection_id, &guard_id).await? {
            let permissions = channel.options.permissions_set();
            if let Err(error) = authorizer.authorize_permissions(&actor, &permissions).await {
                self.record_auth_outcome(auth_outcome_from_error(&error));
                return Err(error);
            }
            self.record_auth_outcome(AuthOutcome::Success);
            return Ok(Some(actor));
        }

        let identity = self.hub.identity(connection_id).await?;
        if let Some(error) = identity.auth_error {
            self.record_auth_outcome(auth_outcome_from_error(&error));
            return Err(error);
        }

        // Resolve actor from either bearer token or session cookie
        let actor = if let Some(session_id) = identity.session_id {
            let sessions = self
                .app
                .sessions()
                .map_err(|e| AuthError::internal(e.to_string()))
                .inspect_err(|e| self.record_auth_outcome(auth_outcome_from_error(e)))?;
            match sessions.validate(&session_id).await {
                Ok(Some(actor)) => actor.with_guard(guard_id.clone()),
                Ok(None) => {
                    let error = AuthError::unauthorized_code(AuthErrorCode::InvalidSession);
                    self.record_auth_outcome(auth_outcome_from_error(&error));
                    return Err(error);
                }
                Err(e) => {
                    let error = AuthError::internal(e.to_string());
                    self.record_auth_outcome(auth_outcome_from_error(&error));
                    return Err(error);
                }
            }
        } else if let Some(token) = identity.bearer_token {
            match auth.authenticate_token(&token, Some(&guard_id)).await {
                Ok(actor) => actor,
                Err(error) => {
                    self.record_auth_outcome(auth_outcome_from_error(&error));
                    return Err(error);
                }
            }
        } else {
            let error = AuthError::unauthorized_code(AuthErrorCode::MissingAuthCredentials);
            self.record_auth_outcome(auth_outcome_from_error(&error));
            return Err(error);
        };
        let permissions = channel.options.permissions_set();
        if let Err(error) = authorizer.authorize_permissions(&actor, &permissions).await {
            self.record_auth_outcome(auth_outcome_from_error(&error));
            return Err(error);
        }
        self.hub
            .cache_actor(
                connection_id,
                actor.clone(),
                self.ws_config.max_connections_per_user,
            )
            .await?;
        self.record_auth_outcome(AuthOutcome::Success);
        Ok(Some(actor))
    }

    fn record_auth_outcome(&self, outcome: AuthOutcome) {
        if let Ok(diagnostics) = self.app.diagnostics() {
            diagnostics.record_auth_outcome(outcome);
        }
    }

    async fn send(&self, connection_id: u64, command: WriterCommand) -> Result<()> {
        match self.hub.send(connection_id, command).await {
            Ok(()) => Ok(()),
            Err(HubSendError::Missing) => Err(Error::message("websocket connection not found")),
            Err(HubSendError::Closed) => {
                if let Some(closed) = self.hub.unregister(connection_id).await {
                    self.cleanup_closed_connections(vec![closed]).await;
                }
                Err(Error::message("websocket connection closed"))
            }
            Err(HubSendError::Full) => {
                if let Some(closed) = self.hub.unregister(connection_id).await {
                    self.cleanup_closed_connections(vec![closed]).await;
                }
                Err(Error::message("websocket outbound buffer full"))
            }
        }
    }

    async fn broadcast(&self, message: &ServerMessage) {
        let closed = self.hub.broadcast(message).await;
        self.cleanup_closed_connections(closed).await;
    }

    async fn broadcast_except(&self, exclude_id: u64, message: &ServerMessage) {
        let closed = self.hub.broadcast_except(exclude_id, message).await;
        self.cleanup_closed_connections(closed).await;
    }

    async fn close_connection(&self, connection_id: u64) {
        if let Some(closed) = self.hub.unregister(connection_id).await {
            self.cleanup_closed_connections(vec![closed]).await;
        }
    }

    async fn cleanup_closed_connections(&self, mut closed: Vec<ClosedConnection>) {
        while let Some(connection) = closed.pop() {
            for subscription in &connection.subscriptions {
                let Some(channel) = self.channels.get(&subscription.channel) else {
                    continue;
                };
                if let Some(ref on_leave) = channel.options.on_leave {
                    let ctx = WebSocketContext::new(
                        self.app.clone(),
                        connection.connection_id,
                        actor_for_channel(&connection.actors, channel),
                        subscription.channel.clone(),
                        subscription.room.clone(),
                    );
                    if let Err(error) = on_leave(ctx).await {
                        tracing::warn!(
                            target: "foundry.websocket",
                            error = %error,
                            "on_leave hook failed during connection cleanup"
                        );
                    }
                }
            }

            for entry in connection.presence_entries {
                let _ = self.backend.srem(&entry.key, &entry.member_value).await;
                let Some(channel) = self.channels.get(&entry.channel) else {
                    continue;
                };
                if channel.options.presence {
                    let leave_msg = ServerMessage {
                        channel: entry.channel,
                        event: PRESENCE_LEAVE_EVENT,
                        room: entry.room,
                        payload: serde_json::json!({
                            "actor_id": entry.actor_id,
                        }),
                    };
                    let additionally_closed = self.hub.broadcast(&leave_msg).await;
                    closed.extend(additionally_closed);
                }
            }
        }
    }
}

fn actor_for_channel(
    actors: &HashMap<GuardId, Actor>,
    channel: &RegisteredChannel,
) -> Option<Actor> {
    if let Some(guard) = channel.options.guard_id() {
        return actors.get(guard).cloned();
    }
    if actors.len() == 1 {
        return actors.values().next().cloned();
    }
    None
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    uri: axum::http::Uri,
    headers: HeaderMap,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    State(state): State<WebSocketServerState>,
) -> Response {
    let production_like = state
        .app
        .config()
        .app()
        .map(|config| config.environment.is_production_like())
        .unwrap_or(false);
    if !origin_allowed(&headers, &state.ws_config.allowed_origins, production_like) {
        return websocket_rejection(StatusCode::FORBIDDEN, "websocket origin is not allowed");
    }

    // Support short-lived tokens via query param for browser WebSocket
    // connections which cannot set custom headers. Keep this bounded because
    // URLs can be logged outside Foundry by proxies or load balancers.
    let mut headers = headers;
    if state.ws_config.query_token_enabled
        && !headers.contains_key(axum::http::header::AUTHORIZATION)
    {
        if let Some(query) = uri.query() {
            match bearer_token_from_query(
                query,
                &state.ws_config.query_token_name,
                state.ws_config.query_token_max_length,
            ) {
                Ok(Some(token)) => {
                    let value = match format!("Bearer {token}").parse() {
                        Ok(value) => value,
                        Err(_) => {
                            return websocket_rejection(
                                StatusCode::BAD_REQUEST,
                                "websocket token query parameter is invalid",
                            );
                        }
                    };
                    headers.insert(axum::http::header::AUTHORIZATION, value);
                }
                Ok(None) => {}
                Err(error) => {
                    return websocket_rejection(StatusCode::BAD_REQUEST, error.to_string());
                }
            }
        }
    }

    let mut identity = state.capture_identity(&headers).await;
    match extract_client_ip(&state.app, &headers, peer_addr) {
        Ok(ip) => identity.client_ip = ip,
        Err(error) => {
            return websocket_rejection(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    }

    let mut upgrade = ws;
    if state.ws_config.max_message_size_bytes > 0 {
        upgrade = upgrade.max_message_size(state.ws_config.max_message_size_bytes);
    }
    if state.ws_config.max_frame_size_bytes > 0 {
        upgrade = upgrade.max_frame_size(state.ws_config.max_frame_size_bytes);
    }
    if state.ws_config.max_write_buffer_size_bytes > 0 {
        upgrade = upgrade.max_write_buffer_size(state.ws_config.max_write_buffer_size_bytes);
    }

    upgrade.on_upgrade(move |socket| handle_socket(socket, state, identity))
}

fn websocket_rejection(status: StatusCode, message: impl Into<String>) -> Response {
    let message = message.into();
    let public_message = if status.is_server_error() {
        Error::internal_server_error_message()
    } else {
        message.as_str()
    };
    let mut response = (
        status,
        axum::Json(ErrorResponse::new(public_message, status)),
    )
        .into_response();
    if status.is_server_error() {
        crate::logging::mark_handler_error_response(
            &mut response,
            status.as_u16(),
            message,
            Vec::new(),
        );
    }
    response
}

fn protocol_payload(payload: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(payload).expect("websocket protocol payload should serialize")
}

fn validate_query_token_config(config: &WebSocketConfig) -> Result<()> {
    if !config.query_token_enabled {
        return Ok(());
    }

    let name = config.query_token_name.trim();
    if name.is_empty() {
        return Err(Error::message(
            "websocket.query_token_name cannot be empty when query tokens are enabled",
        ));
    }
    if name.len() > 64 {
        return Err(Error::message(
            "websocket.query_token_name cannot exceed 64 bytes",
        ));
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '&' | '=' | '?' | '#'))
    {
        return Err(Error::message(
            "websocket.query_token_name contains invalid query parameter characters",
        ));
    }

    Ok(())
}

fn bearer_token_from_query(
    query: &str,
    token_name: &str,
    max_length: usize,
) -> Result<Option<String>> {
    let mut token = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key != token_name {
            continue;
        }
        if token.is_some() {
            return Err(Error::message("duplicate websocket token query parameter"));
        }
        if value.is_empty() {
            return Err(Error::message(
                "websocket token query parameter cannot be empty",
            ));
        }
        if value.chars().any(char::is_control) {
            return Err(Error::message(
                "websocket token query parameter cannot contain control characters",
            ));
        }
        if max_length > 0 && value.len() > max_length {
            return Err(Error::message(format!(
                "websocket token query parameter exceeds maximum length of {max_length} bytes"
            )));
        }
        token = Some(value.into_owned());
    }
    Ok(token)
}

fn origin_allowed(headers: &HeaderMap, allowed_origins: &[String], production_like: bool) -> bool {
    if allowed_origins.iter().any(|origin| origin == "*") {
        return true;
    }

    let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return allowed_origins.is_empty();
    };

    if allowed_origins.is_empty() {
        return !production_like || origin_matches_request_host(headers, origin);
    }

    allowed_origins.iter().any(|allowed| allowed == origin)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OriginAuthority {
    host: String,
    port: Option<u16>,
}

impl OriginAuthority {
    fn parse(value: &str) -> Option<Self> {
        let raw = value.split(',').next()?.trim();
        if raw.is_empty() {
            return None;
        }

        let authority = raw.parse::<Authority>().ok()?;
        let host = authority
            .host()
            .trim_matches(|ch| ch == '[' || ch == ']')
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }

        Some(Self {
            host,
            port: authority.port_u16(),
        })
    }

    fn matches(&self, other: &Self) -> bool {
        self.host == other.host
            && (self.port == other.port || self.port.is_none() || other.port.is_none())
    }
}

fn origin_matches_request_host(headers: &HeaderMap, origin: &str) -> bool {
    let Some(origin) = origin_authority(origin) else {
        return false;
    };

    request_authorities(headers)
        .into_iter()
        .any(|authority| origin.matches(&authority))
}

fn origin_authority(origin: &str) -> Option<OriginAuthority> {
    let uri = origin.parse::<Uri>().ok()?;
    if !matches!(uri.scheme_str(), Some("http") | Some("https")) {
        return None;
    }

    OriginAuthority::parse(uri.authority()?.as_str())
}

fn request_authorities(headers: &HeaderMap) -> Vec<OriginAuthority> {
    let mut authorities = Vec::new();

    for name in ["x-forwarded-host", axum::http::header::HOST.as_str()] {
        let Some(authority) = headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(OriginAuthority::parse)
        else {
            continue;
        };

        if !authorities.contains(&authority) {
            authorities.push(authority);
        }
    }

    authorities
}

fn extract_client_ip(
    app: &AppContext,
    headers: &HeaderMap,
    peer_addr: SocketAddr,
) -> Result<Option<String>> {
    let http = app.config().http()?;
    let ip = crate::http::middleware::resolve_real_ip_from_trusted_proxy_config(
        headers,
        peer_addr.ip(),
        &http.trusted_proxy,
    )?;
    if ip.is_unspecified() {
        Ok(None)
    } else {
        Ok(Some(ip.to_string()))
    }
}

async fn handle_socket(
    socket: WebSocket,
    state: WebSocketServerState,
    identity: ConnectionIdentity,
) {
    let (connection_id, mut outbound, last_pong_at) = state.hub.register(identity).await;
    let (mut sender, mut receiver) = socket.split();

    // Writer task: serializes WriterCommands into WebSocket frames.
    let writer = tokio::spawn(async move {
        while let Some(command) = outbound.recv().await {
            match command {
                WriterCommand::Json(message) => {
                    let payload = match serde_json::to_string(&message) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if sender.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                WriterCommand::Ping => {
                    if sender.send(Message::Ping(vec![].into())).await.is_err() {
                        break;
                    }
                }
                WriterCommand::Pong(payload) => {
                    if sender.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                WriterCommand::Close => {
                    let _ = sender.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    });

    // Heartbeat task: sends pings and closes the connection on timeout.
    let heartbeat_state = state.clone();
    let heartbeat_pong = last_pong_at.clone();
    let heartbeat_interval = Duration::from_secs(state.ws_config.heartbeat_interval_seconds.max(1));
    let heartbeat_timeout = Duration::from_secs(state.ws_config.heartbeat_timeout_seconds.max(1));
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat_interval);
        interval.tick().await; // skip first immediate tick
        loop {
            interval.tick().await;
            if heartbeat_state
                .send(connection_id, WriterCommand::Ping)
                .await
                .is_err()
            {
                break;
            }
            let elapsed = heartbeat_pong.lock().await.elapsed();
            if elapsed > heartbeat_interval + heartbeat_timeout {
                let _ = heartbeat_state
                    .send(connection_id, WriterCommand::Close)
                    .await;
                break;
            }
        }
    });

    while let Some(result) = receiver.next().await {
        let message = match result {
            Ok(message) => message,
            Err(_) => break,
        };

        match message {
            Message::Text(text) => {
                if let Err(error) =
                    process_client_message(&state, connection_id, text.to_string()).await
                {
                    let _ = state
                        .send(
                            connection_id,
                            WriterCommand::Json(ServerMessage {
                                channel: SYSTEM_CHANNEL,
                                event: ERROR_EVENT,
                                room: None,
                                payload: error.payload(),
                            }),
                        )
                        .await;
                }
            }
            Message::Pong(_) => {
                *last_pong_at.lock().await = tokio::time::Instant::now();
            }
            Message::Ping(payload) => {
                let _ = state
                    .send(connection_id, WriterCommand::Pong(payload))
                    .await;
            }
            Message::Close(_) => break,
            Message::Binary(_) => {}
        }
    }

    state.close_connection(connection_id).await;
    abort_websocket_connection_task("writer", writer).await;
    abort_websocket_connection_task("heartbeat", heartbeat).await;
}

async fn abort_websocket_connection_task(name: &'static str, handle: JoinHandle<()>) {
    handle.abort();
    match handle.await {
        Ok(()) => {}
        Err(error) if error.is_cancelled() => {}
        Err(error) if error.is_panic() => {
            let message = panic_payload_message(error.into_panic());
            tracing::error!(
                target: "foundry.websocket",
                task = name,
                panic = %message,
                "websocket connection task panicked"
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "foundry.websocket",
                task = name,
                error = %error,
                "websocket connection task ended unexpectedly"
            );
        }
    }
}

async fn process_client_message(
    state: &WebSocketServerState,
    connection_id: u64,
    payload: String,
) -> Result<()> {
    if state.ws_config.max_message_size_bytes > 0
        && payload.len() > state.ws_config.max_message_size_bytes
    {
        return Err(Error::http(413, "websocket message exceeds maximum size"));
    }

    // Per-connection rate limiting.
    if !state
        .hub
        .check_rate_limit(connection_id, state.ws_config.max_messages_per_second)
        .await
    {
        state
            .send(
                connection_id,
                WriterCommand::Json(ServerMessage {
                    channel: SYSTEM_CHANNEL,
                    event: ERROR_EVENT,
                    room: None,
                    payload: protocol_payload(ErrorResponse::new(
                        "rate limit exceeded",
                        StatusCode::TOO_MANY_REQUESTS,
                    )),
                }),
            )
            .await
            .ok();
        return Ok(());
    }

    let message: ClientMessage = serde_json::from_str(&payload)
        .map_err(|error| Error::http(400, format!("invalid websocket message: {error}")))?;
    validate_client_message(&message, &state.ws_config)?;
    if let Ok(diagnostics) = state.app.diagnostics() {
        diagnostics.record_websocket_inbound_message_on(&message.channel);
    }
    let Some(channel) = state.channels.get(&message.channel) else {
        return Err(Error::http(
            404,
            format!("websocket channel `{}` is not registered", message.channel),
        ));
    };

    match message.action {
        ClientAction::Subscribe => {
            let actor = match state.authorize_channel(connection_id, channel).await {
                Ok(actor) => actor,
                Err(error) => {
                    state
                        .send(
                            connection_id,
                            WriterCommand::Json(ServerMessage {
                                channel: SYSTEM_CHANNEL,
                                event: ERROR_EVENT,
                                room: None,
                                payload: error.payload(),
                            }),
                        )
                        .await
                        .ok();
                    return Ok(());
                }
            };

            // Authorization callback (Feature 4).
            if let Some(ref authorize) = channel.options.authorize {
                let ctx = WebSocketContext::new(
                    state.app.clone(),
                    connection_id,
                    actor.clone(),
                    message.channel.clone(),
                    message.room.clone(),
                );
                if let Err(error) =
                    authorize(ctx, message.channel.clone(), message.room.clone()).await
                {
                    state
                        .send(
                            connection_id,
                            WriterCommand::Json(ServerMessage {
                                channel: SYSTEM_CHANNEL,
                                event: ERROR_EVENT,
                                room: None,
                                payload: error.payload(),
                            }),
                        )
                        .await
                        .ok();
                    return Ok(());
                }
            }

            let subscribed = state
                .hub
                .subscribe(
                    connection_id,
                    &message.channel,
                    message.room.clone(),
                    state.ws_config.max_subscriptions_per_connection,
                )
                .await?;

            // Track presence if enabled for this channel.
            if channel.options.presence && subscribed {
                let actor_id = actor
                    .as_ref()
                    .map(|a| a.id.clone())
                    .unwrap_or_else(|| format!("anon:{connection_id}"));
                let now = chrono::Utc::now().timestamp();
                let key = presence_key(&message.channel);
                let member_value = presence_member_value(&actor_id, &message.channel, now);
                let _ = state.backend.sadd(&key, &member_value).await;
                state
                    .hub
                    .add_presence_entry(
                        connection_id,
                        PresenceEntry {
                            key,
                            member_value,
                            channel: message.channel.clone(),
                            room: message.room.clone(),
                            actor_id: actor_id.clone(),
                        },
                    )
                    .await;

                // Broadcast presence join event to all subscribers.
                let join_msg = ServerMessage {
                    channel: message.channel.clone(),
                    event: PRESENCE_JOIN_EVENT,
                    room: message.room.clone(),
                    payload: protocol_payload(WebSocketPresenceJoinPayload::new(actor_id, now)),
                };
                state.broadcast_except(connection_id, &join_msg).await;
            }

            // Invoke on_join lifecycle hook.
            if let Some(ref on_join) = channel.options.on_join {
                let ctx = WebSocketContext::new(
                    state.app.clone(),
                    connection_id,
                    actor.clone(),
                    message.channel.clone(),
                    message.room.clone(),
                );
                if let Err(e) = on_join(ctx).await {
                    tracing::warn!(target: "foundry.websocket", error = %e, "on_join hook failed");
                }
            }

            // Replay recent messages before sending SUBSCRIBED so the client catches up.
            if channel.options.replay_count > 0 {
                let history_key = format!("ws:history:{}", message.channel);
                if let Ok(messages) = state
                    .backend
                    .lrange(&history_key, 0, channel.options.replay_count as i64 - 1)
                    .await
                {
                    // Messages are stored newest-first (LPUSH), send oldest-first.
                    for raw in messages.into_iter().rev() {
                        if let Ok(msg) = serde_json::from_str::<ServerMessage>(&raw) {
                            if !message_reaches_subscription(&msg, &message.channel, &message.room)
                            {
                                continue;
                            }
                            let _ = state.send(connection_id, WriterCommand::Json(msg)).await;
                        }
                    }
                }
            }

            state
                .send(
                    connection_id,
                    WriterCommand::Json(ServerMessage {
                        channel: message.channel,
                        event: SUBSCRIBED_EVENT,
                        room: message.room,
                        payload: serde_json::Value::Null,
                    }),
                )
                .await?;
        }
        ClientAction::Unsubscribe => {
            // Invoke on_leave lifecycle hook.
            if let Some(ref on_leave) = channel.options.on_leave {
                let actors = state.hub.actors(connection_id).await.unwrap_or_default();
                let ctx = WebSocketContext::new(
                    state.app.clone(),
                    connection_id,
                    actor_for_channel(&actors, channel),
                    message.channel.clone(),
                    message.room.clone(),
                );
                if let Err(e) = on_leave(ctx).await {
                    tracing::warn!(target: "foundry.websocket", error = %e, "on_leave hook failed");
                }
            }

            // Clean up presence entries for this subscription before unsubscribing.
            let entries = state
                .hub
                .take_presence_entries_for_subscription(
                    connection_id,
                    &message.channel,
                    &message.room,
                )
                .await;
            for entry in &entries {
                let _ = state.backend.srem(&entry.key, &entry.member_value).await;
            }

            for entry in entries {
                let leave_msg = ServerMessage {
                    channel: entry.channel,
                    event: PRESENCE_LEAVE_EVENT,
                    room: entry.room,
                    payload: protocol_payload(WebSocketPresenceLeavePayload::new(entry.actor_id)),
                };
                state.broadcast(&leave_msg).await;
            }

            state
                .hub
                .unsubscribe(connection_id, &message.channel, message.room.clone())
                .await;
            state
                .send(
                    connection_id,
                    WriterCommand::Json(ServerMessage {
                        channel: message.channel,
                        event: UNSUBSCRIBED_EVENT,
                        room: message.room,
                        payload: serde_json::Value::Null,
                    }),
                )
                .await?;
        }
        ClientAction::Message => {
            if !state
                .hub
                .is_subscribed(connection_id, &message.channel, &message.room)
                .await
            {
                return Err(Error::http(
                    403,
                    format!(
                        "websocket connection is not subscribed to channel `{}`",
                        message.channel
                    ),
                ));
            }

            let actor = match state.authorize_channel(connection_id, channel).await {
                Ok(actor) => actor,
                Err(error) => {
                    state
                        .send(
                            connection_id,
                            WriterCommand::Json(ServerMessage {
                                channel: SYSTEM_CHANNEL,
                                event: ERROR_EVENT,
                                room: None,
                                payload: error.payload(),
                            }),
                        )
                        .await
                        .ok();
                    return Ok(());
                }
            };
            let context = WebSocketContext::new(
                state.app.clone(),
                connection_id,
                actor,
                message.channel.clone(),
                message.room.clone(),
            );
            let result = run_channel_handler(
                channel,
                context,
                message.payload.unwrap_or(serde_json::Value::Null),
            )
            .await;

            // Send ACK if requested.
            if let Some(ack_id) = message.ack_id {
                let payload = match &result {
                    Ok(()) => WebSocketAckPayload::ok(ack_id),
                    Err(error) => WebSocketAckPayload::error(ack_id, error.to_string()),
                };
                let _ = state
                    .send(
                        connection_id,
                        WriterCommand::Json(ServerMessage {
                            channel: SYSTEM_CHANNEL,
                            event: ACK_EVENT,
                            room: None,
                            payload: protocol_payload(payload),
                        }),
                    )
                    .await;
            }

            result?;
        }
        ClientAction::ClientEvent => {
            if !channel.options.allow_client_events {
                return Err(Error::http(
                    403,
                    "client events not allowed on this channel",
                ));
            }
            let event_id = message
                .event
                .clone()
                .unwrap_or_else(|| ChannelEventId::new("client_event"));
            if is_reserved_channel_protocol_event(&event_id) {
                return Err(Error::http(
                    400,
                    reserved_channel_protocol_event_message("client", &event_id),
                ));
            }
            if !channel.options.client_events.is_empty()
                && !channel.options.client_events.contains(&event_id)
            {
                return Err(Error::http(
                    403,
                    format!(
                        "client event `{event_id}` is not allowed on channel `{}`",
                        message.channel
                    ),
                ));
            }

            if !state
                .hub
                .is_subscribed(connection_id, &message.channel, &message.room)
                .await
            {
                return Err(Error::http(
                    403,
                    format!(
                        "websocket connection is not subscribed to channel `{}`",
                        message.channel
                    ),
                ));
            }

            if let Err(error) = state.authorize_channel(connection_id, channel).await {
                state
                    .send(
                        connection_id,
                        WriterCommand::Json(ServerMessage {
                            channel: SYSTEM_CHANNEL,
                            event: ERROR_EVENT,
                            room: None,
                            payload: error.payload(),
                        }),
                    )
                    .await
                    .ok();
                return Ok(());
            }

            let server_msg = ServerMessage {
                channel: message.channel,
                event: event_id,
                room: message.room,
                payload: message.payload.unwrap_or(serde_json::Value::Null),
            };

            // Broadcast to all subscribers EXCEPT the sender.
            state.broadcast_except(connection_id, &server_msg).await;
        }
    }

    Ok(())
}

fn validate_client_message(message: &ClientMessage, config: &WebSocketConfig) -> Result<()> {
    validate_ws_identifier(
        "websocket channel",
        message.channel.as_str(),
        config.max_channel_length,
    )?;
    if let Some(room) = &message.room {
        validate_ws_identifier("websocket room", room, config.max_room_length)?;
    }
    if let Some(event) = &message.event {
        validate_ws_identifier("websocket event", event.as_str(), config.max_event_length)?;
    }
    if let Some(ack_id) = &message.ack_id {
        validate_ws_identifier("websocket ack_id", ack_id, config.max_ack_id_length)?;
    }
    Ok(())
}

fn validate_ws_identifier(label: &'static str, value: &str, max_length: usize) -> Result<()> {
    if value.is_empty() {
        return Err(Error::http(400, format!("{label} cannot be empty")));
    }
    if value.chars().any(char::is_control) {
        return Err(Error::http(
            400,
            format!("{label} cannot contain control characters"),
        ));
    }
    if max_length > 0 && value.len() > max_length {
        return Err(Error::http(
            400,
            format!("{label} exceeds maximum length of {max_length} bytes"),
        ));
    }
    Ok(())
}

async fn run_channel_handler(
    channel: &RegisteredChannel,
    context: WebSocketContext,
    payload: serde_json::Value,
) -> Result<()> {
    match catch_async_panic(|| channel.handler.handle(context, payload)).await {
        Ok(result) => result,
        Err(panic) => {
            let message = panic_payload_message(panic);
            tracing::error!(
                target: "foundry.websocket",
                channel = %channel.id,
                panic = %message,
                "WebSocket channel handler panicked"
            );
            Err(Error::message(format!(
                "websocket handler panicked: {message}"
            )))
        }
    }
}

#[derive(Clone)]
struct ConnectionHub {
    next_id: Arc<AtomicU64>,
    state: Arc<RwLock<HubState>>,
    diagnostics: Option<Arc<RuntimeDiagnostics>>,
    outbound_buffer_size: usize,
}

impl ConnectionHub {
    fn new(diagnostics: Option<Arc<RuntimeDiagnostics>>, outbound_buffer_size: usize) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            state: Arc::new(RwLock::new(HubState::default())),
            diagnostics,
            outbound_buffer_size: outbound_buffer_size.max(1),
        }
    }
}

impl ConnectionHub {
    async fn register(
        &self,
        identity: ConnectionIdentity,
    ) -> (
        u64,
        mpsc::Receiver<WriterCommand>,
        Arc<tokio::sync::Mutex<tokio::time::Instant>>,
    ) {
        let connection_id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel(self.outbound_buffer_size);
        let last_pong_at = Arc::new(tokio::sync::Mutex::new(tokio::time::Instant::now()));
        let client_ip = identity.client_ip.clone();
        let mut hub = self.state.write().await;
        hub.connections.insert(
            connection_id,
            ConnectionState {
                subscriptions: HashSet::new(),
                presence_entries: Vec::new(),
                identity,
                actors: HashMap::new(),
                sender: tx,
                message_count: 0,
                rate_window_start: tokio::time::Instant::now(),
            },
        );
        if let Some(ref ip) = client_ip {
            let tracking_key = format!("ip:{ip}");
            hub.user_connections
                .entry(tracking_key)
                .or_default()
                .insert(connection_id);
        }
        drop(hub);

        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.record_websocket_connection(WebSocketConnectionState::Opened);
        }

        tracing::info!(
            target: "foundry.websocket",
            connection_id = connection_id,
            "WebSocket connection opened"
        );
        (connection_id, rx, last_pong_at)
    }

    async fn unregister(&self, connection_id: u64) -> Option<ClosedConnection> {
        let mut hub = self.state.write().await;
        let state = hub.connections.remove(&connection_id)?;
        Some(self.close_state(&mut hub, connection_id, state))
    }

    async fn subscribe(
        &self,
        connection_id: u64,
        channel: &ChannelId,
        room: Option<String>,
        max_subscriptions_per_connection: usize,
    ) -> Result<bool> {
        if let Some(state) = self.state.write().await.connections.get_mut(&connection_id) {
            let subscription = SubscriptionKey {
                channel: channel.clone(),
                room,
            };
            if !state.subscriptions.contains(&subscription)
                && max_subscriptions_per_connection > 0
                && state.subscriptions.len() >= max_subscriptions_per_connection
            {
                return Err(Error::http(429, "websocket subscription limit exceeded"));
            }

            let created = state.subscriptions.insert(subscription);
            if created {
                if let Some(diagnostics) = &self.diagnostics {
                    diagnostics.record_websocket_subscription_opened_on(channel);
                }
            }
            return Ok(created);
        }

        Ok(false)
    }

    async fn unsubscribe(
        &self,
        connection_id: u64,
        channel: &ChannelId,
        room: Option<String>,
    ) -> bool {
        if let Some(state) = self.state.write().await.connections.get_mut(&connection_id) {
            let removed = state.subscriptions.remove(&SubscriptionKey {
                channel: channel.clone(),
                room,
            });
            if removed {
                if let Some(diagnostics) = &self.diagnostics {
                    diagnostics.record_websocket_subscription_closed_on(channel);
                }
            }
            return removed;
        }

        false
    }

    async fn send(
        &self,
        connection_id: u64,
        command: WriterCommand,
    ) -> std::result::Result<(), HubSendError> {
        let channel = if let WriterCommand::Json(ref msg) = command {
            Some(msg.channel.clone())
        } else {
            None
        };
        let sender = self
            .state
            .read()
            .await
            .connections
            .get(&connection_id)
            .map(|state| state.sender.clone())
            .ok_or(HubSendError::Missing)?;
        sender.try_send(command).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => HubSendError::Full,
            mpsc::error::TrySendError::Closed(_) => HubSendError::Closed,
        })?;
        if let Some(diagnostics) = &self.diagnostics {
            if let Some(ref ch) = channel {
                diagnostics.record_websocket_outbound_message_on(ch);
            }
        }
        Ok(())
    }

    async fn broadcast(&self, message: &ServerMessage) -> Vec<ClosedConnection> {
        let senders = {
            let hub = self.state.read().await;
            hub.connections
                .iter()
                .filter(|(_, state)| state.accepts(message))
                .map(|(id, state)| (*id, state.sender.clone()))
                .collect::<Vec<_>>()
        };

        self.send_to_many(senders, message).await
    }

    async fn broadcast_except(
        &self,
        exclude_id: u64,
        message: &ServerMessage,
    ) -> Vec<ClosedConnection> {
        let senders = {
            let hub = self.state.read().await;
            hub.connections
                .iter()
                .filter(|(id, state)| **id != exclude_id && state.accepts(message))
                .map(|(id, state)| (*id, state.sender.clone()))
                .collect::<Vec<_>>()
        };

        self.send_to_many(senders, message).await
    }

    async fn identity(
        &self,
        connection_id: u64,
    ) -> std::result::Result<ConnectionIdentity, AuthError> {
        self.state
            .read()
            .await
            .connections
            .get(&connection_id)
            .map(|state| state.identity.clone())
            .ok_or_else(|| AuthError::internal("websocket connection not found"))
    }

    async fn cached_actor(
        &self,
        connection_id: u64,
        guard: &GuardId,
    ) -> std::result::Result<Option<Actor>, AuthError> {
        self.state
            .read()
            .await
            .connections
            .get(&connection_id)
            .map(|state| state.actors.get(guard).cloned())
            .ok_or_else(|| AuthError::internal("websocket connection not found"))
    }

    async fn actors(
        &self,
        connection_id: u64,
    ) -> std::result::Result<HashMap<GuardId, Actor>, AuthError> {
        self.state
            .read()
            .await
            .connections
            .get(&connection_id)
            .map(|state| state.actors.clone())
            .ok_or_else(|| AuthError::internal("websocket connection not found"))
    }

    async fn cache_actor(
        &self,
        connection_id: u64,
        actor: Actor,
        max_connections_per_user: u32,
    ) -> std::result::Result<(), AuthError> {
        let actor_id = actor.id.clone();
        let guard = actor.guard.clone();

        let mut hub = self.state.write().await;

        if max_connections_per_user > 0 {
            if let Some(existing) = hub.user_connections.get(&actor_id) {
                let other_connections = existing.iter().filter(|id| **id != connection_id).count();
                if other_connections >= max_connections_per_user as usize {
                    return Err(AuthError::forbidden_code(
                        AuthErrorCode::MaxConnectionsPerUserExceeded,
                    ));
                }
            }
        }

        let state = hub
            .connections
            .get_mut(&connection_id)
            .ok_or_else(|| AuthError::internal("websocket connection not found"))?;
        state.actors.insert(guard, actor);

        // Remove IP-based tracking (anonymous → authenticated transition)
        if let Some(ref ip) = state.identity.client_ip {
            let ip_key = format!("ip:{ip}");
            if let Some(set) = hub.user_connections.get_mut(&ip_key) {
                set.remove(&connection_id);
                if set.is_empty() {
                    hub.user_connections.remove(&ip_key);
                }
            }
        }

        // Track by actor ID
        hub.user_connections
            .entry(actor_id)
            .or_default()
            .insert(connection_id);

        Ok(())
    }

    async fn add_presence_entry(&self, connection_id: u64, entry: PresenceEntry) {
        if let Some(state) = self.state.write().await.connections.get_mut(&connection_id) {
            state.presence_entries.push(entry);
        }
    }

    async fn take_presence_entries_for_subscription(
        &self,
        connection_id: u64,
        channel: &ChannelId,
        room: &Option<String>,
    ) -> Vec<PresenceEntry> {
        let mut hub = self.state.write().await;
        let Some(state) = hub.connections.get_mut(&connection_id) else {
            return Vec::new();
        };
        let (matching, remaining): (Vec<_>, Vec<_>) = state
            .presence_entries
            .drain(..)
            .partition(|e| e.channel == *channel && e.room == *room);
        state.presence_entries = remaining;
        matching
    }

    async fn is_subscribed(
        &self,
        connection_id: u64,
        channel: &ChannelId,
        room: &Option<String>,
    ) -> bool {
        self.state
            .read()
            .await
            .connections
            .get(&connection_id)
            .map(|state| {
                state.subscriptions.contains(&SubscriptionKey {
                    channel: channel.clone(),
                    room: room.clone(),
                })
            })
            .unwrap_or(false)
    }

    /// Per-connection rate limiting. Returns `true` if the message is allowed.
    async fn check_rate_limit(&self, connection_id: u64, max_per_second: u32) -> bool {
        if max_per_second == 0 {
            return true; // unlimited
        }
        let mut hub = self.state.write().await;
        let Some(state) = hub.connections.get_mut(&connection_id) else {
            return false;
        };
        if state.rate_window_start.elapsed() >= Duration::from_secs(1) {
            state.message_count = 0;
            state.rate_window_start = tokio::time::Instant::now();
        }
        state.message_count += 1;
        state.message_count <= max_per_second
    }

    /// Force-disconnect all connections belonging to a given actor.
    async fn disconnect_by_actor(&self, actor_id: &str) -> Vec<ClosedConnection> {
        let mut hub = self.state.write().await;
        let to_remove: Vec<u64> = hub
            .connections
            .iter()
            .filter(|(_, state)| state.actors.values().any(|a| a.id == actor_id))
            .map(|(id, _)| *id)
            .collect();

        let mut closed = Vec::new();
        for id in &to_remove {
            if let Some(state) = hub.connections.remove(id) {
                let _ = state.sender.try_send(WriterCommand::Close);
                closed.push(self.close_state(&mut hub, *id, state));
            }
        }

        closed
    }

    async fn send_to_many(
        &self,
        senders: Vec<(u64, mpsc::Sender<WriterCommand>)>,
        message: &ServerMessage,
    ) -> Vec<ClosedConnection> {
        let mut to_close = Vec::new();
        for (id, sender) in senders {
            if sender
                .try_send(WriterCommand::Json(message.clone()))
                .is_err()
            {
                to_close.push(id);
            }
        }

        if to_close.is_empty() {
            return Vec::new();
        }

        let mut hub = self.state.write().await;
        let mut closed = Vec::new();
        for id in to_close {
            if let Some(state) = hub.connections.remove(&id) {
                closed.push(self.close_state(&mut hub, id, state));
            }
        }
        closed
    }

    fn close_state(
        &self,
        hub: &mut HubState,
        connection_id: u64,
        state: ConnectionState,
    ) -> ClosedConnection {
        if let Some(diagnostics) = &self.diagnostics {
            for key in &state.subscriptions {
                diagnostics.record_websocket_subscription_closed_on(&key.channel);
            }
            diagnostics.record_websocket_connection(WebSocketConnectionState::Closed);
        }

        for actor in state.actors.values() {
            remove_connection_tracking(&mut hub.user_connections, &actor.id, connection_id);
        }
        if let Some(ref ip) = state.identity.client_ip {
            remove_connection_tracking(
                &mut hub.user_connections,
                &format!("ip:{ip}"),
                connection_id,
            );
        }

        tracing::info!(
            target: "foundry.websocket",
            connection_id = connection_id,
            "WebSocket connection closed"
        );

        ClosedConnection {
            connection_id,
            subscriptions: state.subscriptions.into_iter().collect(),
            presence_entries: state.presence_entries,
            actors: state.actors,
        }
    }
}

#[derive(Default)]
struct HubState {
    connections: HashMap<u64, ConnectionState>,
    user_connections: HashMap<String, HashSet<u64>>,
}

enum HubSendError {
    Missing,
    Closed,
    Full,
}

struct ClosedConnection {
    connection_id: u64,
    subscriptions: Vec<SubscriptionKey>,
    presence_entries: Vec<PresenceEntry>,
    actors: HashMap<GuardId, Actor>,
}

fn remove_connection_tracking(
    user_connections: &mut HashMap<String, HashSet<u64>>,
    key: &str,
    connection_id: u64,
) {
    if let Some(set) = user_connections.get_mut(key) {
        set.remove(&connection_id);
        if set.is_empty() {
            user_connections.remove(key);
        }
    }
}

struct ConnectionState {
    subscriptions: HashSet<SubscriptionKey>,
    presence_entries: Vec<PresenceEntry>,
    identity: ConnectionIdentity,
    actors: HashMap<GuardId, Actor>,
    sender: mpsc::Sender<WriterCommand>,
    message_count: u32,
    rate_window_start: tokio::time::Instant,
}

impl ConnectionState {
    fn accepts(&self, message: &ServerMessage) -> bool {
        self.subscriptions.iter().any(|subscription| {
            message_reaches_subscription(message, &subscription.channel, &subscription.room)
        })
    }
}

fn message_reaches_subscription(
    message: &ServerMessage,
    channel: &ChannelId,
    room: &Option<String>,
) -> bool {
    if message.channel != *channel {
        return false;
    }

    match (&message.room, room) {
        (None, _) => true,
        (Some(message_room), Some(subscription_room)) => message_room == subscription_room,
        (Some(_), None) => false,
    }
}

#[derive(Debug, Clone, Default)]
struct ConnectionIdentity {
    bearer_token: Option<String>,
    session_id: Option<String>,
    auth_error: Option<AuthError>,
    client_ip: Option<String>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SubscriptionKey {
    channel: ChannelId,
    room: Option<String>,
}

/// Tracks presence values that need cleanup on disconnect.
#[derive(Debug, Clone)]
struct PresenceEntry {
    key: String,
    member_value: String,
    channel: ChannelId,
    room: Option<String>,
    actor_id: String,
}

fn auth_outcome_from_error(error: &AuthError) -> AuthOutcome {
    match error {
        AuthError::Unauthorized(_) => AuthOutcome::Unauthorized,
        AuthError::Forbidden(_) => AuthOutcome::Forbidden,
        AuthError::Internal(_) => AuthOutcome::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::body::to_bytes;

    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::support::runtime::RuntimeBackend;
    use crate::validation::RuleRegistry;
    use crate::websocket::WebSocketRegistrar;

    #[test]
    fn message_routing_matches_room_contract() {
        let channel = ChannelId::new("chat");
        let all_message = ServerMessage {
            channel: channel.clone(),
            event: ChannelEventId::new("notice"),
            room: None,
            payload: serde_json::Value::Null,
        };
        let room_message = ServerMessage {
            channel: channel.clone(),
            event: ChannelEventId::new("notice"),
            room: Some("room:42".to_string()),
            payload: serde_json::Value::Null,
        };

        assert!(message_reaches_subscription(&all_message, &channel, &None));
        assert!(message_reaches_subscription(
            &all_message,
            &channel,
            &Some("room:42".to_string())
        ));
        assert!(!message_reaches_subscription(
            &room_message,
            &channel,
            &None
        ));
        assert!(message_reaches_subscription(
            &room_message,
            &channel,
            &Some("room:42".to_string())
        ));
        assert!(!message_reaches_subscription(
            &room_message,
            &channel,
            &Some("room:7".to_string())
        ));
    }

    #[tokio::test]
    async fn hub_reports_full_outbound_buffer() {
        let hub = ConnectionHub::new(None, 1);
        let (connection_id, _rx, _last_pong) = hub.register(ConnectionIdentity::default()).await;
        let message = ServerMessage {
            channel: ChannelId::new("chat"),
            event: ChannelEventId::new("notice"),
            room: None,
            payload: serde_json::Value::Null,
        };

        assert!(hub
            .send(connection_id, WriterCommand::Json(message.clone()))
            .await
            .is_ok());
        assert!(matches!(
            hub.send(connection_id, WriterCommand::Json(message)).await,
            Err(HubSendError::Full)
        ));
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn websocket_state(
        channels: Vec<RegisteredChannel>,
        namespace: &'static str,
    ) -> WebSocketServerState {
        websocket_state_with_config(channels, namespace, WebSocketConfig::default())
    }

    fn websocket_state_with_config(
        channels: Vec<RegisteredChannel>,
        namespace: &'static str,
        ws_config: WebSocketConfig,
    ) -> WebSocketServerState {
        WebSocketServerState::new(
            test_app(),
            channels,
            RuntimeBackend::memory(namespace),
            ws_config,
        )
    }

    #[tokio::test]
    async fn pubsub_task_drop_cancels_backend_subscription() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(
                ChannelId::new("chat"),
                |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
            )
            .unwrap();
        let state = websocket_state(registrar.into_channels(), "ws-pubsub-task-drop");
        let runtime = match &state.backend {
            RuntimeBackend::Memory(runtime) => runtime.clone(),
            RuntimeBackend::Redis(_) => unreachable!("test backend is memory"),
        };

        let pubsub_task = state.start_pubsub().await.unwrap().unwrap();

        for _ in 0..20 {
            if runtime.ws_tx.receiver_count() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(runtime.ws_tx.receiver_count(), 1);

        drop(pubsub_task);

        for _ in 0..20 {
            if runtime.ws_tx.receiver_count() == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }

        assert_eq!(runtime.ws_tx.receiver_count(), 0);
    }

    #[tokio::test]
    async fn websocket_connection_task_abort_waits_for_task_drop() {
        struct DropFlag(Arc<AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let dropped_flag = dropped.clone();
        let (started_tx, started_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let _drop_flag = DropFlag(dropped_flag);
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.unwrap();

        abort_websocket_connection_task("test", handle).await;

        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn websocket_connection_task_panic_isolated_when_waiting() {
        let handle = tokio::spawn(async {
            panic!("websocket connection child boom");
        });

        tokio::task::yield_now().await;

        abort_websocket_connection_task("test", handle).await;
    }

    async fn next_json(outbound: &mut mpsc::Receiver<WriterCommand>) -> ServerMessage {
        match outbound
            .recv()
            .await
            .expect("expected outbound websocket frame")
        {
            WriterCommand::Json(message) => message,
            WriterCommand::Ping | WriterCommand::Pong(_) | WriterCommand::Close => {
                panic!("expected JSON websocket frame")
            }
        }
    }

    async fn subscribe(
        state: &WebSocketServerState,
        connection_id: u64,
        outbound: &mut mpsc::Receiver<WriterCommand>,
        channel: &'static str,
    ) {
        process_client_message(
            state,
            connection_id,
            serde_json::json!({
                "action": "subscribe",
                "channel": channel,
            })
            .to_string(),
        )
        .await
        .unwrap();

        let subscribed = next_json(outbound).await;
        assert_eq!(subscribed.event, SUBSCRIBED_EVENT);
        assert_eq!(subscribed.channel, ChannelId::new(channel));
    }

    #[test]
    fn websocket_query_token_is_decoded_and_validated() {
        assert_eq!(
            bearer_token_from_query("token=abc%20123&other=value", "token", 4096)
                .unwrap()
                .as_deref(),
            Some("abc 123")
        );
        assert!(bearer_token_from_query("other=value", "token", 4096)
            .unwrap()
            .is_none());
        assert_eq!(
            bearer_token_from_query("token=one&token=two", "token", 4096)
                .unwrap_err()
                .to_string(),
            "duplicate websocket token query parameter"
        );
        assert_eq!(
            bearer_token_from_query("token=", "token", 4096)
                .unwrap_err()
                .to_string(),
            "websocket token query parameter cannot be empty"
        );
        assert_eq!(
            bearer_token_from_query("auth=abc&token=ignored", "auth", 4096)
                .unwrap()
                .as_deref(),
            Some("abc")
        );
        assert_eq!(
            bearer_token_from_query("token=abcdef", "token", 3)
                .unwrap_err()
                .to_string(),
            "websocket token query parameter exceeds maximum length of 3 bytes"
        );
    }

    #[test]
    fn websocket_empty_origin_allowlist_is_permissive_outside_production() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            "https://example.com".parse().unwrap(),
        );

        assert!(origin_allowed(&headers, &[], false));
    }

    #[test]
    fn websocket_empty_origin_allowlist_allows_same_origin_in_production() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            "https://example.com".parse().unwrap(),
        );
        headers.insert(axum::http::header::HOST, "example.com".parse().unwrap());

        assert!(origin_allowed(&headers, &[], true));
        assert!(origin_allowed(&HeaderMap::new(), &[], true));
    }

    #[test]
    fn websocket_empty_origin_allowlist_allows_forwarded_same_origin_in_production() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            "https://example.com".parse().unwrap(),
        );
        headers.insert(axum::http::header::HOST, "127.0.0.1:3010".parse().unwrap());
        headers.insert("x-forwarded-host", "example.com".parse().unwrap());

        assert!(origin_allowed(&headers, &[], true));
    }

    #[test]
    fn websocket_empty_origin_allowlist_rejects_cross_origin_in_production() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            "https://evil.test".parse().unwrap(),
        );
        headers.insert(axum::http::header::HOST, "example.com".parse().unwrap());

        assert!(!origin_allowed(&headers, &[], true));
    }

    #[test]
    fn websocket_allowed_origins_still_match_explicit_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            "https://example.com".parse().unwrap(),
        );

        assert!(origin_allowed(
            &headers,
            &["https://example.com".to_string()],
            true
        ));
        assert!(!origin_allowed(
            &headers,
            &["https://other.test".to_string()],
            true
        ));
    }

    #[tokio::test]
    async fn websocket_server_rejections_hide_internal_error_messages() {
        let response = websocket_rejection(
            StatusCode::INTERNAL_SERVER_ERROR,
            "trusted proxy config leaked secret=abc",
        );

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["message"], Error::internal_server_error_message());
        assert_eq!(payload["status"], 500);
        assert!(!payload["message"].as_str().unwrap().contains("secret"));
    }

    #[test]
    fn websocket_query_token_config_is_validated() {
        assert!(validate_query_token_config(&WebSocketConfig::default()).is_ok());
        assert!(validate_query_token_config(&WebSocketConfig {
            query_token_enabled: false,
            query_token_name: String::new(),
            ..WebSocketConfig::default()
        })
        .is_ok());

        assert_eq!(
            validate_query_token_config(&WebSocketConfig {
                query_token_name: String::new(),
                ..WebSocketConfig::default()
            })
            .unwrap_err()
            .to_string(),
            "websocket.query_token_name cannot be empty when query tokens are enabled"
        );
        assert_eq!(
            validate_query_token_config(&WebSocketConfig {
                query_token_name: "bad name".to_string(),
                ..WebSocketConfig::default()
            })
            .unwrap_err()
            .to_string(),
            "websocket.query_token_name contains invalid query parameter characters"
        );
    }

    #[tokio::test]
    async fn websocket_uses_peer_ip_unless_trusted_proxy_matches() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.10".parse().unwrap());
        let peer: SocketAddr = "203.0.113.5:443".parse().unwrap();
        let app = test_app();

        assert_eq!(
            extract_client_ip(&app, &headers, peer).unwrap().as_deref(),
            Some("203.0.113.5")
        );

        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("foundry.toml"),
            r#"
                [http.trusted_proxy]
                enabled = true
                trusted_cidrs = ["203.0.113.0/24"]
                headers = ["x-forwarded-for"]
            "#,
        )
        .unwrap();
        let trusted_app = AppContext::new(
            Container::new(),
            ConfigRepository::from_dir(directory.path()).unwrap(),
            RuleRegistry::new(),
        )
        .unwrap();

        assert_eq!(
            extract_client_ip(&trusted_app, &headers, peer)
                .unwrap()
                .as_deref(),
            Some("198.51.100.10")
        );
    }

    #[tokio::test]
    async fn websocket_message_size_and_identifier_limits_are_enforced() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(
                ChannelId::new("chat"),
                |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
            )
            .unwrap();
        let state = websocket_state_with_config(
            registrar.into_channels(),
            "ws-client-limits",
            WebSocketConfig {
                max_message_size_bytes: 32,
                max_room_length: 4,
                max_ack_id_length: 3,
                ..WebSocketConfig::default()
            },
        );
        let (connection_id, _outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;

        let oversized = process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "subscribe",
                "channel": "chat",
                "room": "long-room",
            })
            .to_string(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            &oversized,
            Error::Http { status, .. } if *status == 413
        ));
        assert_eq!(
            oversized.to_string(),
            "websocket message exceeds maximum size"
        );

        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(
                ChannelId::new("chat"),
                |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
            )
            .unwrap();
        let state = websocket_state_with_config(
            registrar.into_channels(),
            "ws-client-field-limits",
            WebSocketConfig {
                max_message_size_bytes: 0,
                max_room_length: 4,
                max_ack_id_length: 3,
                ..WebSocketConfig::default()
            },
        );
        let (connection_id, mut outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;

        let room_error = process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "subscribe",
                "channel": "chat",
                "room": "abcde",
            })
            .to_string(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            room_error.to_string(),
            "websocket room exceeds maximum length of 4 bytes"
        );

        subscribe(&state, connection_id, &mut outbound, "chat").await;
        let ack_error = process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "chat",
                "ack_id": "abcd",
            })
            .to_string(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            ack_error.to_string(),
            "websocket ack_id exceeds maximum length of 3 bytes"
        );
    }

    #[tokio::test]
    async fn websocket_subscription_limit_rejects_new_subscriptions() {
        let mut registrar = WebSocketRegistrar::new();
        for channel in ["one", "two"] {
            registrar
                .channel(
                    ChannelId::owned(channel.to_string()),
                    |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
                )
                .unwrap();
        }
        let state = websocket_state_with_config(
            registrar.into_channels(),
            "ws-subscription-limit",
            WebSocketConfig {
                max_subscriptions_per_connection: 1,
                ..WebSocketConfig::default()
            },
        );
        let (connection_id, mut outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;
        subscribe(&state, connection_id, &mut outbound, "one").await;

        let error = process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "subscribe",
                "channel": "two",
            })
            .to_string(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            &error,
            Error::Http { status, .. } if *status == 429
        ));
        assert_eq!(error.to_string(), "websocket subscription limit exceeded");
    }

    #[tokio::test]
    async fn websocket_rate_limit_error_uses_standard_error_payload() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(
                ChannelId::new("chat"),
                |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
            )
            .unwrap();
        let state = websocket_state_with_config(
            registrar.into_channels(),
            "ws-rate-limit-error-payload",
            WebSocketConfig {
                max_messages_per_second: 1,
                ..WebSocketConfig::default()
            },
        );
        let (connection_id, mut outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;

        process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "subscribe",
                "channel": "chat",
            })
            .to_string(),
        )
        .await
        .unwrap();
        let subscribed = next_json(&mut outbound).await;
        assert_eq!(subscribed.event, SUBSCRIBED_EVENT);

        process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "chat",
            })
            .to_string(),
        )
        .await
        .unwrap();

        let limited = next_json(&mut outbound).await;
        assert_eq!(limited.channel, SYSTEM_CHANNEL);
        assert_eq!(limited.event, ERROR_EVENT);
        assert_eq!(limited.payload["message"], "rate limit exceeded");
        assert_eq!(limited.payload["status"], 429);
        assert!(limited.payload.get("error").is_none());
    }

    #[tokio::test]
    async fn channel_handler_success_sends_ok_ack() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(
                ChannelId::new("chat"),
                |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
            )
            .unwrap();
        let state = websocket_state(registrar.into_channels(), "ws-handler-success");
        let (connection_id, mut outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;
        subscribe(&state, connection_id, &mut outbound, "chat").await;

        process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "chat",
                "ack_id": "ack-ok",
            })
            .to_string(),
        )
        .await
        .unwrap();

        let ack = next_json(&mut outbound).await;
        assert_eq!(ack.event, ACK_EVENT);
        assert_eq!(ack.payload["ack_id"], "ack-ok");
        assert_eq!(ack.payload["status"], "ok");
        assert!(ack.payload["error"].is_null());
    }

    #[tokio::test]
    async fn channel_handler_error_sends_error_ack() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(
                ChannelId::new("fail"),
                |_context: WebSocketContext, _payload: serde_json::Value| async {
                    Err(Error::message("handler failed"))
                },
            )
            .unwrap();
        let state = websocket_state(registrar.into_channels(), "ws-handler-error");
        let (connection_id, mut outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;
        subscribe(&state, connection_id, &mut outbound, "fail").await;

        let error = process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "fail",
                "ack_id": "ack-error",
            })
            .to_string(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.to_string(), "handler failed");
        let ack = next_json(&mut outbound).await;
        assert_eq!(ack.event, ACK_EVENT);
        assert_eq!(ack.payload["ack_id"], "ack-error");
        assert_eq!(ack.payload["status"], "error");
        assert_eq!(ack.payload["error"], "handler failed");
    }

    #[tokio::test]
    async fn channel_handler_panic_sends_error_ack_and_connection_can_continue() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(
                ChannelId::new("panic"),
                |_context: WebSocketContext, _payload: serde_json::Value| async {
                    panic!("handler explode");
                    #[allow(unreachable_code)]
                    Ok(())
                },
            )
            .unwrap();
        registrar
            .channel(
                ChannelId::new("chat"),
                |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
            )
            .unwrap();
        let state = websocket_state(registrar.into_channels(), "ws-handler-panic");
        let (connection_id, mut outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;
        subscribe(&state, connection_id, &mut outbound, "panic").await;
        subscribe(&state, connection_id, &mut outbound, "chat").await;

        let error = process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "panic",
                "ack_id": "ack-panic",
            })
            .to_string(),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "websocket handler panicked: handler explode"
        );
        let ack = next_json(&mut outbound).await;
        assert_eq!(ack.event, ACK_EVENT);
        assert_eq!(ack.payload["ack_id"], "ack-panic");
        assert_eq!(ack.payload["status"], "error");
        assert_eq!(
            ack.payload["error"],
            "websocket handler panicked: handler explode"
        );

        process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "chat",
                "ack_id": "ack-after-panic",
            })
            .to_string(),
        )
        .await
        .unwrap();
        let ack = next_json(&mut outbound).await;
        assert_eq!(ack.event, ACK_EVENT);
        assert_eq!(ack.payload["ack_id"], "ack-after-panic");
        assert_eq!(ack.payload["status"], "ok");
    }

    struct FactoryPanicHandler;

    impl crate::websocket::ChannelHandler for FactoryPanicHandler {
        fn handle<'life0, 'async_trait>(
            &'life0 self,
            _context: WebSocketContext,
            _payload: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            panic!("handler factory explode")
        }
    }

    #[tokio::test]
    async fn channel_handler_factory_panic_sends_error_ack_and_connection_can_continue() {
        let mut registrar = WebSocketRegistrar::new();
        registrar
            .channel(ChannelId::new("panic-build"), FactoryPanicHandler)
            .unwrap();
        registrar
            .channel(
                ChannelId::new("chat"),
                |_context: WebSocketContext, _payload: serde_json::Value| async { Ok(()) },
            )
            .unwrap();
        let state = websocket_state(registrar.into_channels(), "ws-handler-factory-panic");
        let (connection_id, mut outbound, _last_pong) =
            state.hub.register(ConnectionIdentity::default()).await;
        subscribe(&state, connection_id, &mut outbound, "panic-build").await;
        subscribe(&state, connection_id, &mut outbound, "chat").await;

        let error = process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "panic-build",
                "ack_id": "ack-factory-panic",
            })
            .to_string(),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "websocket handler panicked: handler factory explode"
        );
        let ack = next_json(&mut outbound).await;
        assert_eq!(ack.event, ACK_EVENT);
        assert_eq!(ack.payload["ack_id"], "ack-factory-panic");
        assert_eq!(ack.payload["status"], "error");
        assert_eq!(
            ack.payload["error"],
            "websocket handler panicked: handler factory explode"
        );

        process_client_message(
            &state,
            connection_id,
            serde_json::json!({
                "action": "message",
                "channel": "chat",
                "ack_id": "ack-after-factory-panic",
            })
            .to_string(),
        )
        .await
        .unwrap();
        let ack = next_json(&mut outbound).await;
        assert_eq!(ack.event, ACK_EVENT);
        assert_eq!(ack.payload["ack_id"], "ack-after-factory-panic");
        assert_eq!(ack.payload["status"], "ok");
    }

    #[tokio::test]
    async fn bound_server_exits_when_shutdown_future_completes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        let server = BoundWebSocketServer {
            listener,
            router: axum::Router::new(),
            local_addr,
            pubsub_task: None,
        };

        server.serve_until(async {}).await.unwrap();
    }
}
