use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use foundry::prelude::*;
use futures_util::{SinkExt, StreamExt};
use semver::{Version, VersionReq};
use tempfile::tempdir;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const BASE_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.base");
const DEPENDENT_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.dependent");
const PHONE_RULE: ValidationRuleId = ValidationRuleId::new("plugin.phone");
const PLUGIN_COMMAND: CommandId = CommandId::new("plugin-demo");
const PLUGIN_SCHEDULE: ScheduleId = ScheduleId::new("plugin.demo.tick");
const PLUGIN_CHANNEL: ChannelId = ChannelId::new("plugin.chat");
const PLUGIN_EVENT: ChannelEventId = ChannelEventId::new("echo");
const PLUGIN_ASSET: PluginAssetId = PluginAssetId::new("plugin-config");
const PLUGIN_SCAFFOLD: PluginScaffoldId = PluginScaffoldId::new("portal");

#[derive(Clone)]
struct GreetingService(String);

#[derive(Clone)]
struct DerivedGreeting(String);

#[derive(Clone)]
struct SharedLog(Arc<Mutex<Vec<String>>>);

#[derive(Debug, Deserialize)]
struct CreateContact {
    phone: String,
}

#[async_trait]
impl RequestValidator for CreateContact {
    async fn validate(&self, validator: &mut Validator) -> Result<()> {
        validator
            .field("phone", self.phone.clone())
            .required()
            .rule(PHONE_RULE)
            .apply()
            .await?;
        Ok(())
    }
}

#[async_trait]
impl foundry::validation::FromMultipart for CreateContact {
    async fn from_multipart(
        multipart: &mut axum::extract::Multipart,
    ) -> foundry::foundation::Result<Self> {
        let mut phone = None;
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| foundry::foundation::Error::message(format!("multipart error: {e}")))?
        {
            if field.name() == Some("phone") {
                phone = Some(field.text().await.map_err(|e| {
                    foundry::foundation::Error::message(format!("field error: {e}"))
                })?);
            }
        }
        Ok(Self {
            phone: phone.unwrap_or_default(),
        })
    }
}

struct PhoneRule;

#[async_trait]
impl ValidationRule for PhoneRule {
    async fn validate(
        &self,
        _context: &RuleContext,
        value: &str,
    ) -> std::result::Result<(), ValidationError> {
        if value.chars().all(|character| character.is_ascii_digit()) && value.len() >= 10 {
            Ok(())
        } else {
            Err(ValidationError::new(
                "phone",
                "phone must contain at least 10 digits",
            ))
        }
    }
}

#[derive(Clone)]
struct BasePlugin {
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct DependentPlugin {
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct BasePluginProvider {
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct DependentPluginProvider {
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct AppVerifierProvider {
    log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ServiceProvider for BasePluginProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        let greeting = registrar
            .config()
            .string("plugin_demo.greeting")
            .unwrap_or_else(|| "missing".to_string());
        registrar.singleton(GreetingService(greeting))?;
        registrar.singleton(SharedLog(self.log.clone()))?;
        self.log
            .lock()
            .unwrap()
            .push("base-provider-register".to_string());
        Ok(())
    }

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push("base-provider-boot".to_string());
        Ok(())
    }
}

#[async_trait]
impl ServiceProvider for DependentPluginProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        let greeting = registrar.resolve::<GreetingService>()?;
        registrar.singleton(DerivedGreeting(format!("dep:{}", greeting.0)))?;
        self.log
            .lock()
            .unwrap()
            .push("dep-provider-register".to_string());
        Ok(())
    }

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push("dep-provider-boot".to_string());
        Ok(())
    }
}

#[async_trait]
impl ServiceProvider for AppVerifierProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        let derived = registrar.resolve::<DerivedGreeting>()?;
        self.log
            .lock()
            .unwrap()
            .push(format!("app-provider-register:{}", derived.0));
        Ok(())
    }

    async fn boot(&self, app: &AppContext) -> Result<()> {
        let greeting = app.resolve::<GreetingService>()?;
        let derived = app.resolve::<DerivedGreeting>()?;
        self.log
            .lock()
            .unwrap()
            .push(format!("app-provider-boot:{}:{}", greeting.0, derived.0));
        Ok(())
    }
}

#[async_trait]
impl Plugin for BasePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            BASE_PLUGIN_ID,
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .description("Base plugin")
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar
            .config_defaults(
                toml::from_str(
                    r#"
                        [plugin_demo]
                        greeting = "from-plugin"
                    "#,
                )
                .unwrap(),
            )
            .register_provider(BasePluginProvider {
                log: self.log.clone(),
            })
            .register_validation_rule(PHONE_RULE, PhoneRule)
            .register_routes(register_plugin_routes)
            .register_commands(register_plugin_commands)
            .register_schedule(register_plugin_schedule)
            .register_websocket_routes(register_plugin_websocket);
        registrar.register_assets([PluginAsset::text(
            PLUGIN_ASSET,
            PluginAssetKind::Config,
            "config/plugin-base.toml",
            "enabled = true\n",
        )])?;
        registrar.register_scaffolds([PluginScaffold::new(PLUGIN_SCAFFOLD)
            .description("Plugin portal scaffold")
            .variable(PluginScaffoldVar::new("name"))
            .file(
                "src/generated/{{name}}.rs",
                "pub const PORTAL_NAME: &str = \"{{name}}\";\n",
            )])?;
        Ok(())
    }

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push("base-plugin-boot".to_string());
        Ok(())
    }
}

#[async_trait]
impl Plugin for DependentPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            DEPENDENT_PLUGIN_ID,
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .dependency(PluginDependency::new(
            BASE_PLUGIN_ID,
            VersionReq::parse("^1").unwrap(),
        ))
        .description("Dependent plugin")
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar.register_provider(DependentPluginProvider {
            log: self.log.clone(),
        });
        Ok(())
    }

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        self.log.lock().unwrap().push("dep-plugin-boot".to_string());
        Ok(())
    }
}

fn register_plugin_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.route("/plugin/ready", get(plugin_ready));
    registrar.route("/plugin/contacts", post(create_contact));
    Ok(())
}

async fn plugin_ready(State(app): State<AppContext>) -> impl IntoResponse {
    let greeting = app.resolve::<GreetingService>().unwrap();
    Json(serde_json::json!({
        "greeting": greeting.0,
    }))
}

async fn create_contact(Validated(payload): Validated<CreateContact>) -> impl IntoResponse {
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "phone": payload.phone,
        })),
    )
}

fn register_plugin_commands(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        PLUGIN_COMMAND,
        Command::new("plugin-demo").about("plugin demo command"),
        |invocation| async move {
            let log = invocation.app().resolve::<SharedLog>()?;
            log.0.lock().unwrap().push("plugin-command".to_string());
            Ok(())
        },
    )?;
    Ok(())
}

fn register_plugin_schedule(registry: &mut ScheduleRegistry) -> Result<()> {
    registry.cron(
        PLUGIN_SCHEDULE,
        CronExpression::parse("*/1 * * * * *")?,
        |invocation| async move {
            let log = invocation.app().resolve::<SharedLog>()?;
            log.0.lock().unwrap().push("plugin-schedule".to_string());
            Ok(())
        },
    )?;
    Ok(())
}

fn register_plugin_websocket(registrar: &mut WebSocketRegistrar) -> Result<()> {
    registrar.channel(
        PLUGIN_CHANNEL,
        |context: WebSocketContext, payload: serde_json::Value| async move {
            context.publish(PLUGIN_EVENT, payload).await
        },
    )?;
    Ok(())
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn write_plugin_config(dir: &Path, server_port: u16, websocket_port: u16) {
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

            [plugin_demo]
            greeting = "from-app"
        "#
        ),
    )
    .unwrap();
}

fn build_plugin_app(config_dir: &Path, log: Arc<Mutex<Vec<String>>>) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_plugin(BasePlugin { log: log.clone() })
        .register_plugin(DependentPlugin { log: log.clone() })
        .register_provider(AppVerifierProvider { log })
}

async fn wait_for_http_ready(base_url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..40 {
        if client
            .get(format!("{base_url}/plugin/ready"))
            .send()
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("plugin http server did not become ready");
}

async fn wait_for_log_entry(log: &Arc<Mutex<Vec<String>>>, entry: &str) {
    for _ in 0..40 {
        if log.lock().unwrap().iter().any(|item| item == entry) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("log entry `{entry}` not observed");
}

async fn connect_websocket(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    connect_async(url).await.unwrap().0
}

#[tokio::test]
async fn plugins_bootstrap_in_dependency_order_and_app_config_overrides_defaults() {
    let config_dir = tempdir().unwrap();
    write_plugin_config(config_dir.path(), free_port(), free_port());
    let log = Arc::new(Mutex::new(Vec::new()));

    let kernel = build_plugin_app(config_dir.path(), log.clone())
        .build_http_kernel()
        .await
        .unwrap();
    let app = kernel.app().clone();

    assert_eq!(app.resolve::<GreetingService>().unwrap().0, "from-app");
    assert_eq!(app.resolve::<DerivedGreeting>().unwrap().0, "dep:from-app");

    let plugin_ids = app
        .plugins()
        .unwrap()
        .plugins()
        .iter()
        .map(|plugin| plugin.id().clone())
        .collect::<Vec<_>>();
    assert_eq!(plugin_ids, vec![BASE_PLUGIN_ID, DEPENDENT_PLUGIN_ID]);

    assert_eq!(
        log.lock().unwrap().clone(),
        vec![
            "base-provider-register",
            "dep-provider-register",
            "app-provider-register:dep:from-app",
            "base-provider-boot",
            "dep-provider-boot",
            "base-plugin-boot",
            "dep-plugin-boot",
            "app-provider-boot:from-app:dep:from-app",
        ]
    );
}

#[tokio::test]
async fn plugin_contributed_runtime_features_work_across_kernels() {
    let config_dir = tempdir().unwrap();
    let server_port = free_port();
    let websocket_port = free_port();
    write_plugin_config(config_dir.path(), server_port, websocket_port);
    let log = Arc::new(Mutex::new(Vec::new()));

    let cli = build_plugin_app(config_dir.path(), log.clone())
        .build_cli_kernel()
        .await
        .unwrap();
    cli.run_with_args(["foundry", "plugin-demo"]).await.unwrap();
    assert!(log.lock().unwrap().contains(&"plugin-command".to_string()));

    let scheduler = build_plugin_app(config_dir.path(), log.clone())
        .build_scheduler_kernel()
        .await
        .unwrap();
    let executed = scheduler.run_once().await.unwrap();
    assert_eq!(executed, vec![PLUGIN_SCHEDULE]);
    wait_for_log_entry(&log, "plugin-schedule").await;

    let http_kernel = build_plugin_app(config_dir.path(), log.clone())
        .build_http_kernel()
        .await
        .unwrap();
    let http_server = http_kernel.bind().await.unwrap();
    let base_url = format!("http://{}", http_server.local_addr());
    let http_task = tokio::spawn(async move { http_server.serve().await.unwrap() });
    wait_for_http_ready(&base_url).await;

    let client = reqwest::Client::new();
    let invalid = client
        .post(format!("{base_url}/plugin/contacts"))
        .json(&serde_json::json!({ "phone": "abc" }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let valid = client
        .post(format!("{base_url}/plugin/contacts"))
        .json(&serde_json::json!({ "phone": "0123456789" }))
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), StatusCode::CREATED);
    assert_eq!(
        valid.json::<serde_json::Value>().await.unwrap()["phone"],
        "0123456789"
    );

    let ready = client
        .get(format!("{base_url}/plugin/ready"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(ready["greeting"], "from-app");
    http_task.abort();

    let websocket_kernel = build_plugin_app(config_dir.path(), log.clone())
        .build_websocket_kernel()
        .await
        .unwrap();
    let websocket_server = websocket_kernel.bind().await.unwrap();
    let websocket_task = tokio::spawn(async move { websocket_server.serve().await.unwrap() });

    let mut socket = connect_websocket(&format!("ws://127.0.0.1:{websocket_port}/ws")).await;
    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Subscribe,
                channel: PLUGIN_CHANNEL,
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
    let subscribed: ServerMessage =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(subscribed.event, SUBSCRIBED_EVENT);

    socket
        .send(Message::Text(
            serde_json::to_string(&ClientMessage {
                action: ClientAction::Message,
                channel: PLUGIN_CHANNEL,
                room: None,
                payload: Some(serde_json::json!({ "body": "hi" })),
                event: None,
                ack_id: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let echoed: ServerMessage =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(echoed.event, PLUGIN_EVENT);
    assert_eq!(echoed.payload["body"], "hi");

    websocket_task.abort();
}

#[tokio::test]
async fn built_in_plugin_cli_commands_install_assets_and_render_scaffolds() {
    let config_dir = tempdir().unwrap();
    write_plugin_config(config_dir.path(), free_port(), free_port());
    let log = Arc::new(Mutex::new(Vec::new()));
    let output_dir = tempdir().unwrap();

    build_plugin_app(config_dir.path(), log.clone())
        .build_cli_kernel()
        .await
        .unwrap()
        .run_with_args(["foundry", "plugin:list"])
        .await
        .unwrap();

    build_plugin_app(config_dir.path(), log.clone())
        .build_cli_kernel()
        .await
        .unwrap()
        .run_with_args([
            "foundry",
            "plugin:install-assets",
            "--plugin",
            BASE_PLUGIN_ID.as_str(),
            "--to",
            output_dir.path().to_str().unwrap(),
        ])
        .await
        .unwrap();

    assert_eq!(
        fs::read_to_string(output_dir.path().join("config/plugin-base.toml")).unwrap(),
        "enabled = true\n"
    );

    build_plugin_app(config_dir.path(), log)
        .build_cli_kernel()
        .await
        .unwrap()
        .run_with_args([
            "foundry",
            "plugin:scaffold",
            "--plugin",
            BASE_PLUGIN_ID.as_str(),
            "--template",
            PLUGIN_SCAFFOLD.as_str(),
            "--set",
            "name=dashboard",
            "--to",
            output_dir.path().to_str().unwrap(),
        ])
        .await
        .unwrap();

    assert_eq!(
        fs::read_to_string(output_dir.path().join("src/generated/dashboard.rs")).unwrap(),
        "pub const PORTAL_NAME: &str = \"dashboard\";\n"
    );
}

// ---------------------------------------------------------------------------
// Phase 1: Direct registration (no ServiceProvider wrapper)
// ---------------------------------------------------------------------------

const DIRECT_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.direct");
const DIRECT_GUARD: GuardId = GuardId::new("direct_guard");
const DIRECT_POLICY: PolicyId = PolicyId::new("direct_policy");

struct DirectRegistrationPlugin;

struct DirectGuard;

#[async_trait]
impl foundry::auth::BearerAuthenticator for DirectGuard {
    async fn authenticate(&self, token: &str) -> Result<Option<Actor>> {
        if token == "direct-secret" {
            Ok(Some(Actor::new("direct-user", DIRECT_GUARD)))
        } else {
            Ok(None)
        }
    }
}

struct DirectActorHydrator;

#[async_trait]
impl ActorHydrator for DirectActorHydrator {
    async fn hydrate(&self, actor: &Actor, _app: &AppContext) -> Result<Option<Actor>> {
        Ok(Some(
            actor
                .clone()
                .with_permissions([PermissionId::new("direct:hydrated")]),
        ))
    }
}

struct DirectPolicy;

#[async_trait]
impl foundry::auth::Policy for DirectPolicy {
    async fn evaluate(&self, actor: &Actor, _app: &AppContext) -> Result<bool> {
        Ok(actor.id == "direct-user")
    }
}

impl foundry::plugin::Plugin for DirectRegistrationPlugin {
    fn manifest(&self) -> foundry::plugin::PluginManifest {
        foundry::plugin::PluginManifest::new(
            DIRECT_PLUGIN_ID,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1.0").unwrap(),
        )
    }

    fn register(&self, registrar: &mut foundry::plugin::PluginRegistrar) -> Result<()> {
        // Direct registration — no ServiceProvider wrapper needed
        registrar.register_guard(DIRECT_GUARD, DirectGuard);
        registrar.register_actor_hydrator(DIRECT_GUARD, DirectActorHydrator);
        registrar.register_policy(DIRECT_POLICY, DirectPolicy);
        registrar.register_middleware(foundry::MiddlewareConfig::Compression(foundry::Compression));
        registrar.register_routes(|r| {
            r.route(
                "/direct-plugin",
                axum::routing::get(|| async { "direct-plugin-ok" }),
            );
            Ok(())
        });
        Ok(())
    }
}

#[tokio::test]
async fn plugin_direct_registration_works_without_provider_wrapper() {
    let kernel = App::builder()
        .register_plugin(DirectRegistrationPlugin)
        .build_http_kernel()
        .await
        .unwrap();

    let app = kernel.app();

    // Guard is registered — verify through auth manager
    let auth = app.auth().unwrap();
    let actor = auth
        .authenticate_token("direct-secret", Some(&DIRECT_GUARD))
        .await
        .unwrap();
    assert_eq!(actor.id, "direct-user");
    assert!(actor.has_permission(PermissionId::new("direct:hydrated")));

    // Policy is registered — verify through authorizer
    let authorizer = app.authorizer().unwrap();
    let allowed = authorizer
        .allows_policy(&actor, DIRECT_POLICY)
        .await
        .unwrap();
    assert!(allowed);
}

struct DirectHydratorCollisionProvider;

#[async_trait]
impl ServiceProvider for DirectHydratorCollisionProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_actor_hydrator(DIRECT_GUARD, DirectActorHydrator)
    }
}

#[tokio::test]
async fn plugin_and_provider_actor_hydrators_cannot_claim_the_same_guard() {
    let error = App::builder()
        .register_plugin(DirectRegistrationPlugin)
        .register_provider(DirectHydratorCollisionProvider)
        .build_cli_kernel()
        .await
        .err()
        .expect("actor hydrator collision should fail bootstrap");

    assert!(error
        .to_string()
        .contains("actor hydrator for guard `direct_guard` already registered"));
}

// ---------------------------------------------------------------------------
// Semantic event ID collisions across plugin and application registrations
// ---------------------------------------------------------------------------

const EVENT_COLLISION_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.event_collision");

#[derive(Clone, Serialize)]
struct PluginCollisionEvent;

impl Event for PluginCollisionEvent {
    const ID: EventId = EventId::new("tests.shared_event_id");
}

#[derive(Clone, Serialize)]
struct ApplicationCollisionEvent;

impl Event for ApplicationCollisionEvent {
    const ID: EventId = EventId::new("tests.shared_event_id");
}

struct PluginCollisionListener;

#[async_trait]
impl EventListener<PluginCollisionEvent> for PluginCollisionListener {
    async fn handle(&self, _context: &EventContext, _event: &PluginCollisionEvent) -> Result<()> {
        Ok(())
    }
}

struct ApplicationCollisionListener;

#[async_trait]
impl EventListener<ApplicationCollisionEvent> for ApplicationCollisionListener {
    async fn handle(
        &self,
        _context: &EventContext,
        _event: &ApplicationCollisionEvent,
    ) -> Result<()> {
        Ok(())
    }
}

struct EventCollisionPlugin;

impl Plugin for EventCollisionPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            EVENT_COLLISION_PLUGIN_ID,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1.0").unwrap(),
        )
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
        registrar.listen_event::<PluginCollisionEvent, _>(PluginCollisionListener);
        Ok(())
    }
}

struct EventCollisionProvider;

#[async_trait]
impl ServiceProvider for EventCollisionProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.listen_event::<ApplicationCollisionEvent, _>(ApplicationCollisionListener)
    }
}

#[tokio::test]
async fn plugin_and_application_event_types_cannot_share_a_semantic_id() {
    let error = App::builder()
        .register_plugin(EventCollisionPlugin)
        .register_provider(EventCollisionProvider)
        .build_cli_kernel()
        .await
        .err()
        .expect("event ID collision should fail bootstrap");

    assert!(error
        .to_string()
        .contains("event ID `tests.shared_event_id`"));
    assert!(error
        .to_string()
        .contains(std::any::type_name::<PluginCollisionEvent>()));
    assert!(error
        .to_string()
        .contains(std::any::type_name::<ApplicationCollisionEvent>()));
}

// ---------------------------------------------------------------------------
// Phase 4: Shutdown lifecycle
// ---------------------------------------------------------------------------

const SHUTDOWN_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.shutdown_test");
const BOOT_PANIC_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.boot_panic");
const SHUTDOWN_PANIC_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.shutdown_panic");
const SHUTDOWN_FOLLOWER_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.shutdown_follower");

struct ShutdownPlugin {
    log: Arc<Mutex<Vec<String>>>,
}

struct BootPanicPlugin;

struct ShutdownPanicPlugin {
    log: Arc<Mutex<Vec<String>>>,
}

struct ShutdownFollowerPlugin {
    log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl foundry::plugin::Plugin for ShutdownPlugin {
    fn manifest(&self) -> foundry::plugin::PluginManifest {
        foundry::plugin::PluginManifest::new(
            SHUTDOWN_PLUGIN_ID,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1.0").unwrap(),
        )
    }

    fn register(&self, _registrar: &mut foundry::plugin::PluginRegistrar) -> Result<()> {
        Ok(())
    }

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        self.log.lock().unwrap().push("booted".to_string());
        Ok(())
    }

    async fn shutdown(&self, _app: &AppContext) -> Result<()> {
        self.log.lock().unwrap().push("shutdown".to_string());
        Ok(())
    }
}

#[async_trait]
impl foundry::plugin::Plugin for BootPanicPlugin {
    fn manifest(&self) -> foundry::plugin::PluginManifest {
        foundry::plugin::PluginManifest::new(
            BOOT_PANIC_PLUGIN_ID,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1.0").unwrap(),
        )
    }

    fn register(&self, _registrar: &mut foundry::plugin::PluginRegistrar) -> Result<()> {
        Ok(())
    }

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        panic!("boot boom")
    }
}

#[async_trait]
impl foundry::plugin::Plugin for ShutdownPanicPlugin {
    fn manifest(&self) -> foundry::plugin::PluginManifest {
        foundry::plugin::PluginManifest::new(
            SHUTDOWN_PANIC_PLUGIN_ID,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1.0").unwrap(),
        )
    }

    fn register(&self, _registrar: &mut foundry::plugin::PluginRegistrar) -> Result<()> {
        Ok(())
    }

    async fn shutdown(&self, _app: &AppContext) -> Result<()> {
        self.log.lock().unwrap().push("panic-shutdown".to_string());
        panic!("shutdown boom")
    }
}

#[async_trait]
impl foundry::plugin::Plugin for ShutdownFollowerPlugin {
    fn manifest(&self) -> foundry::plugin::PluginManifest {
        foundry::plugin::PluginManifest::new(
            SHUTDOWN_FOLLOWER_PLUGIN_ID,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1.0").unwrap(),
        )
    }

    fn register(&self, _registrar: &mut foundry::plugin::PluginRegistrar) -> Result<()> {
        Ok(())
    }

    async fn shutdown(&self, _app: &AppContext) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push("follower-shutdown".to_string());
        Ok(())
    }
}

#[tokio::test]
async fn plugin_shutdown_called_in_reverse_dependency_order() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let kernel = App::builder()
        .register_plugin(ShutdownPlugin { log: log.clone() })
        .build_http_kernel()
        .await
        .unwrap();

    // Boot should have been called
    assert_eq!(log.lock().unwrap().as_slice(), &["booted"]);

    // Trigger shutdown manually
    kernel.app().shutdown_plugins().await.unwrap();

    assert_eq!(log.lock().unwrap().as_slice(), &["booted", "shutdown"]);
}

const BOOT_FAIL_DEPENDENT_PLUGIN_ID: PluginId = PluginId::new("foundry.plugin.boot_fail_dependent");

struct BootFailDependentPlugin {
    log: Arc<Mutex<Vec<String>>>,
}

struct FailingAfterPluginsProvider;

#[async_trait]
impl ServiceProvider for FailingAfterPluginsProvider {
    async fn boot(&self, _app: &AppContext) -> Result<()> {
        Err(Error::message("application provider boot failed"))
    }
}

#[async_trait]
impl foundry::plugin::Plugin for BootFailDependentPlugin {
    fn manifest(&self) -> foundry::plugin::PluginManifest {
        foundry::plugin::PluginManifest::new(
            BOOT_FAIL_DEPENDENT_PLUGIN_ID,
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1.0").unwrap(),
        )
        .depends_on(SHUTDOWN_PLUGIN_ID, VersionReq::parse(">=1.0.0").unwrap())
    }

    fn register(&self, _registrar: &mut foundry::plugin::PluginRegistrar) -> Result<()> {
        Ok(())
    }

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        Err(foundry::foundation::Error::message("dependent boot failed"))
    }

    async fn shutdown(&self, _app: &AppContext) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push("dependent-shutdown".to_string());
        Ok(())
    }
}

#[tokio::test]
async fn plugins_booted_before_a_boot_failure_are_shut_down() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let error = match App::builder()
        .register_plugin(ShutdownPlugin { log: log.clone() })
        .register_plugin(BootFailDependentPlugin { log: log.clone() })
        .build_cli_kernel()
        .await
    {
        Ok(_) => panic!("expected dependent plugin boot failure to fail bootstrap"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("dependent boot failed"));
    // ShutdownPlugin booted before the failure, so its shutdown must run as
    // part of the bootstrap rollback.
    assert_eq!(
        log.lock().unwrap().as_slice(),
        &["booted", "dependent-shutdown", "shutdown"]
    );
}

#[tokio::test]
async fn plugins_are_rolled_back_when_application_provider_boot_fails() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let error = match App::builder()
        .register_plugin(ShutdownPlugin { log: log.clone() })
        .register_provider(FailingAfterPluginsProvider)
        .build_cli_kernel()
        .await
    {
        Ok(_) => panic!("expected provider boot failure to fail bootstrap"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("application provider boot failed"));
    assert_eq!(log.lock().unwrap().as_slice(), &["booted", "shutdown"]);
}

#[tokio::test]
async fn plugin_boot_panic_becomes_bootstrap_error() {
    let error = match App::builder()
        .register_plugin(BootPanicPlugin)
        .build_cli_kernel()
        .await
    {
        Ok(_) => panic!("expected plugin boot panic to fail bootstrap"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("plugin `foundry.plugin.boot_panic` boot panicked"));
    assert!(error.to_string().contains("boot boom"));
}

#[tokio::test]
async fn plugin_shutdown_panic_isolated_and_later_plugins_still_shutdown() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let kernel = App::builder()
        .register_plugin(ShutdownFollowerPlugin { log: log.clone() })
        .register_plugin(ShutdownPanicPlugin { log: log.clone() })
        .build_cli_kernel()
        .await
        .unwrap();

    let error = kernel.app().shutdown_plugins().await.unwrap_err();
    assert!(error.to_string().contains("shutdown boom"));

    assert_eq!(
        log.lock().unwrap().as_slice(),
        &["panic-shutdown", "follower-shutdown"]
    );

    kernel.app().shutdown_plugins().await.unwrap();
    assert_eq!(
        log.lock().unwrap().as_slice(),
        &["panic-shutdown", "follower-shutdown"]
    );
}
