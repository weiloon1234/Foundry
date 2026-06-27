use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use clap::{Arg, ArgAction, Command};
use semver::{Version, VersionReq};
use toml::Value;

pub use crate::support::{PluginAssetId, PluginId, PluginScaffoldId};

use crate::cli::{CommandInvocation, CommandRegistrar};
use crate::foundation::{AppContext, Error, Result, ServiceProvider, ServiceRegistrar};
use crate::http::RouteRegistrar;
use crate::logging::{catch_async_panic, catch_sync_panic, panic_payload_message};
use crate::scheduler::ScheduleRegistrar;
use crate::support::ValidationRuleId;
use crate::validation::ValidationRule;
use crate::websocket::WebSocketRouteRegistrar;

/// Type-erased registration action applied to ServiceRegistrar during bootstrap.
type RegistrarAction = Box<dyn FnOnce(&ServiceRegistrar) -> Result<()> + Send>;

const PLUGIN_LIST_COMMAND: crate::support::CommandId =
    crate::support::CommandId::new("plugin:list");
const PLUGIN_INSTALL_ASSETS_COMMAND: crate::support::CommandId =
    crate::support::CommandId::new("plugin:install-assets");
const PLUGIN_SCAFFOLD_COMMAND: crate::support::CommandId =
    crate::support::CommandId::new("plugin:scaffold");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginDependency {
    id: PluginId,
    version_req: VersionReq,
}

impl PluginDependency {
    pub fn new<I>(id: I, version_req: VersionReq) -> Self
    where
        I: Into<PluginId>,
    {
        Self {
            id: id.into(),
            version_req,
        }
    }

    pub fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn version_req(&self) -> &VersionReq {
        &self.version_req
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginAssetKind {
    Config,
    Migration,
    Static,
}

#[derive(Clone, Debug)]
pub struct PluginAsset {
    id: PluginAssetId,
    kind: PluginAssetKind,
    target_path: PathBuf,
    contents: Arc<[u8]>,
}

impl PluginAsset {
    pub fn text<I, P>(
        id: I,
        kind: PluginAssetKind,
        target_path: P,
        contents: impl Into<String>,
    ) -> Self
    where
        I: Into<PluginAssetId>,
        P: Into<PathBuf>,
    {
        let contents = contents.into().into_bytes().into_boxed_slice();
        Self {
            id: id.into(),
            kind,
            target_path: target_path.into(),
            contents: Arc::from(contents),
        }
    }

    pub fn binary<I, P>(
        id: I,
        kind: PluginAssetKind,
        target_path: P,
        contents: impl Into<Vec<u8>>,
    ) -> Self
    where
        I: Into<PluginAssetId>,
        P: Into<PathBuf>,
    {
        let contents = contents.into().into_boxed_slice();
        Self {
            id: id.into(),
            kind,
            target_path: target_path.into(),
            contents: Arc::from(contents),
        }
    }

    pub fn id(&self) -> &PluginAssetId {
        &self.id
    }

    pub fn kind(&self) -> &PluginAssetKind {
        &self.kind
    }

    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    fn contents(&self) -> &[u8] {
        &self.contents
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginScaffoldVar {
    name: String,
    description: Option<String>,
    default: Option<String>,
}

impl PluginScaffoldVar {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            default: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default = Some(value.into());
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description_text(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn default_value(&self) -> Option<&str> {
        self.default.as_deref()
    }
}

#[derive(Clone, Debug)]
struct PluginScaffoldFile {
    path: PathBuf,
    contents: Arc<str>,
}

#[derive(Clone, Debug)]
pub struct PluginScaffold {
    id: PluginScaffoldId,
    description: Option<String>,
    vars: Vec<PluginScaffoldVar>,
    files: Vec<PluginScaffoldFile>,
}

impl PluginScaffold {
    pub fn new<I>(id: I) -> Self
    where
        I: Into<PluginScaffoldId>,
    {
        Self {
            id: id.into(),
            description: None,
            vars: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn variable(mut self, variable: PluginScaffoldVar) -> Self {
        self.vars.push(variable);
        self
    }

    pub fn file(mut self, path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        self.files.push(PluginScaffoldFile {
            path: path.into(),
            contents: Arc::from(contents.into()),
        });
        self
    }

    pub fn id(&self) -> &PluginScaffoldId {
        &self.id
    }

    pub fn description_text(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn variables(&self) -> &[PluginScaffoldVar] {
        &self.vars
    }

    pub fn files(&self) -> Vec<PathBuf> {
        self.files.iter().map(|file| file.path.clone()).collect()
    }

    fn validate(&self) -> Result<()> {
        let mut vars = BTreeSet::new();
        for variable in &self.vars {
            if !vars.insert(variable.name.clone()) {
                return Err(Error::message(format!(
                    "plugin scaffold `{}` has duplicate variable `{}`",
                    self.id, variable.name
                )));
            }
        }

        let mut files = BTreeSet::new();
        for file in &self.files {
            validate_relative_output_path(&file.path, "plugin scaffold")?;
            if !files.insert(file.path.clone()) {
                return Err(Error::message(format!(
                    "plugin scaffold `{}` has duplicate file `{}`",
                    self.id,
                    file.path.display()
                )));
            }
        }

        Ok(())
    }

    fn render(&self, values: &BTreeMap<String, String>) -> Result<Vec<(PathBuf, Vec<u8>)>> {
        let mut resolved = BTreeMap::new();
        for variable in &self.vars {
            match values.get(variable.name()) {
                Some(value) => {
                    resolved.insert(variable.name().to_string(), value.clone());
                }
                None => match variable.default_value() {
                    Some(value) => {
                        resolved.insert(variable.name().to_string(), value.to_string());
                    }
                    None => {
                        return Err(Error::message(format!(
                            "missing scaffold variable `{}` for `{}`",
                            variable.name(),
                            self.id
                        )));
                    }
                },
            }
        }

        for key in values.keys() {
            if !self.vars.iter().any(|variable| variable.name() == key) {
                return Err(Error::message(format!(
                    "unknown scaffold variable `{key}` for `{}`",
                    self.id
                )));
            }
        }

        self.files
            .iter()
            .map(|file| {
                let rendered_path = render_template(&file.path.to_string_lossy(), &resolved)?;
                Ok((
                    PathBuf::from(rendered_path),
                    render_template(&file.contents, &resolved)?.into_bytes(),
                ))
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct PluginManifest {
    id: PluginId,
    version: Version,
    foundry_version: VersionReq,
    dependencies: Vec<PluginDependency>,
    description: Option<String>,
    assets: Vec<PluginAsset>,
    scaffolds: Vec<PluginScaffold>,
}

impl PluginManifest {
    pub fn new<I>(id: I, version: Version, foundry_version: VersionReq) -> Self
    where
        I: Into<PluginId>,
    {
        Self {
            id: id.into(),
            version,
            foundry_version,
            dependencies: Vec::new(),
            description: None,
            assets: Vec::new(),
            scaffolds: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn dependency(mut self, dependency: PluginDependency) -> Self {
        self.dependencies.push(dependency);
        self
    }

    pub fn depends_on<I>(self, id: I, version_req: VersionReq) -> Self
    where
        I: Into<PluginId>,
    {
        self.dependency(PluginDependency::new(id, version_req))
    }

    pub fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn foundry_version(&self) -> &VersionReq {
        &self.foundry_version
    }

    pub fn dependencies(&self) -> &[PluginDependency] {
        &self.dependencies
    }

    pub fn description_text(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn assets(&self) -> &[PluginAsset] {
        &self.assets
    }

    pub fn scaffolds(&self) -> &[PluginScaffold] {
        &self.scaffolds
    }

    fn with_assets_and_scaffolds(
        mut self,
        assets: Vec<PluginAsset>,
        scaffolds: Vec<PluginScaffold>,
    ) -> Self {
        self.assets = assets;
        self.scaffolds = scaffolds;
        self
    }
}

#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn manifest(&self) -> PluginManifest;

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()>;

    async fn boot(&self, _app: &AppContext) -> Result<()> {
        Ok(())
    }

    /// Called during graceful shutdown in reverse dependency order.
    /// Use for cleanup: flush buffers, close external connections, etc.
    async fn shutdown(&self, _app: &AppContext) -> Result<()> {
        Ok(())
    }
}

pub(crate) async fn boot_plugin(plugin: &Arc<dyn Plugin>, app: &AppContext) -> Result<()> {
    let manifest = plugin_manifest(plugin)?;
    run_plugin_lifecycle_callback(manifest.id(), "boot", || plugin.boot(app)).await
}

pub(crate) async fn shutdown_plugin(plugin: &Arc<dyn Plugin>, app: &AppContext) -> Result<()> {
    let manifest = plugin_manifest(plugin)?;
    run_plugin_lifecycle_callback(manifest.id(), "shutdown", || plugin.shutdown(app)).await
}

async fn run_plugin_lifecycle_callback<F, Fut>(
    id: &PluginId,
    phase: &'static str,
    callback: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    match catch_async_panic(callback).await {
        Ok(result) => result,
        Err(panic) => Err(plugin_panic_error(Some(id), phase, panic)),
    }
}

fn plugin_manifest(plugin: &Arc<dyn Plugin>) -> Result<PluginManifest> {
    match catch_sync_panic(|| plugin.manifest()) {
        Ok(manifest) => Ok(manifest),
        Err(panic) => Err(plugin_panic_error(None, "manifest", panic)),
    }
}

fn register_plugin(plugin: &Arc<dyn Plugin>, manifest: &PluginManifest) -> Result<PluginRegistrar> {
    let mut registrar = PluginRegistrar::new();
    match catch_sync_panic(|| plugin.register(&mut registrar)) {
        Ok(Ok(())) => Ok(registrar),
        Ok(Err(error)) => Err(error),
        Err(panic) => Err(plugin_panic_error(Some(manifest.id()), "register", panic)),
    }
}

fn plugin_panic_error(
    id: Option<&PluginId>,
    phase: &'static str,
    panic: Box<dyn std::any::Any + Send>,
) -> Error {
    let message = panic_payload_message(panic);
    match id {
        Some(id) => {
            tracing::error!(
                target: "foundry.plugin",
                plugin = %id,
                phase = phase,
                panic = %message,
                "plugin lifecycle panicked"
            );
            Error::message(format!("plugin `{id}` {phase} panicked: {message}"))
        }
        None => {
            tracing::error!(
                target: "foundry.plugin",
                phase = phase,
                panic = %message,
                "plugin lifecycle panicked"
            );
            Error::message(format!("plugin {phase} panicked: {message}"))
        }
    }
}

pub struct PluginRegistrar {
    providers: Vec<Arc<dyn ServiceProvider>>,
    routes: Vec<RouteRegistrar>,
    commands: Vec<CommandRegistrar>,
    schedules: Vec<ScheduleRegistrar>,
    websocket_routes: Vec<WebSocketRouteRegistrar>,
    validation_rules: Vec<(ValidationRuleId, Arc<dyn ValidationRule>)>,
    config_defaults: Vec<Value>,
    assets: Vec<PluginAsset>,
    scaffolds: Vec<PluginScaffold>,
    middlewares: Vec<crate::http::middleware::MiddlewareConfig>,
    /// Type-erased registrations applied to ServiceRegistrar during bootstrap.
    registrar_actions: Vec<RegistrarAction>,
}

impl Default for PluginRegistrar {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistrar {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            routes: Vec::new(),
            commands: Vec::new(),
            schedules: Vec::new(),
            websocket_routes: Vec::new(),
            validation_rules: Vec::new(),
            config_defaults: Vec::new(),
            assets: Vec::new(),
            scaffolds: Vec::new(),
            middlewares: Vec::new(),
            registrar_actions: Vec::new(),
        }
    }

    pub fn register_provider<P>(&mut self, provider: P) -> &mut Self
    where
        P: ServiceProvider,
    {
        self.providers.push(Arc::new(provider));
        self
    }

    pub fn register_routes<F>(&mut self, registrar: F) -> &mut Self
    where
        F: Fn(&mut crate::http::HttpRegistrar) -> Result<()> + Send + Sync + 'static,
    {
        self.routes.push(Arc::new(registrar));
        self
    }

    pub fn register_commands<F>(&mut self, registrar: F) -> &mut Self
    where
        F: Fn(&mut crate::cli::CommandRegistry) -> Result<()> + Send + Sync + 'static,
    {
        self.commands.push(Arc::new(registrar));
        self
    }

    pub fn register_schedule<F>(&mut self, registrar: F) -> &mut Self
    where
        F: Fn(&mut crate::scheduler::ScheduleRegistry) -> Result<()> + Send + Sync + 'static,
    {
        self.schedules.push(Arc::new(registrar));
        self
    }

    pub fn register_websocket_routes<F>(&mut self, registrar: F) -> &mut Self
    where
        F: Fn(&mut crate::websocket::WebSocketRegistrar) -> Result<()> + Send + Sync + 'static,
    {
        self.websocket_routes.push(Arc::new(registrar));
        self
    }

    pub fn register_validation_rule<I, R>(&mut self, id: I, rule: R) -> &mut Self
    where
        I: Into<ValidationRuleId>,
        R: ValidationRule,
    {
        self.validation_rules.push((id.into(), Arc::new(rule)));
        self
    }

    pub fn config_defaults(&mut self, defaults: Value) -> &mut Self {
        self.config_defaults.push(defaults);
        self
    }

    pub fn register_assets<I>(&mut self, assets: I) -> Result<&mut Self>
    where
        I: IntoIterator<Item = PluginAsset>,
    {
        let mut ids = self
            .assets
            .iter()
            .map(|asset| asset.id.clone())
            .collect::<BTreeSet<_>>();
        for asset in assets {
            validate_relative_output_path(asset.target_path(), "plugin asset")?;
            if !ids.insert(asset.id.clone()) {
                return Err(Error::message(format!(
                    "plugin asset `{}` already registered",
                    asset.id
                )));
            }
            self.assets.push(asset);
        }
        Ok(self)
    }

    pub fn register_scaffolds<I>(&mut self, scaffolds: I) -> Result<&mut Self>
    where
        I: IntoIterator<Item = PluginScaffold>,
    {
        let mut ids = self
            .scaffolds
            .iter()
            .map(|scaffold| scaffold.id.clone())
            .collect::<BTreeSet<_>>();
        for scaffold in scaffolds {
            scaffold.validate()?;
            if !ids.insert(scaffold.id.clone()) {
                return Err(Error::message(format!(
                    "plugin scaffold `{}` already registered",
                    scaffold.id
                )));
            }
            self.scaffolds.push(scaffold);
        }
        Ok(self)
    }

    // ── Direct registration methods (bypass ServiceProvider indirection) ──

    pub fn register_guard<I, G>(&mut self, id: I, guard: G) -> &mut Self
    where
        I: Into<crate::support::GuardId> + Send + 'static,
        G: crate::auth::BearerAuthenticator,
    {
        let id = id.into();
        self.registrar_actions
            .push(Box::new(move |r| r.register_guard(id, guard)));
        self
    }

    pub fn register_policy<I, P>(&mut self, id: I, policy: P) -> &mut Self
    where
        I: Into<crate::support::PolicyId> + Send + 'static,
        P: crate::auth::Policy,
    {
        let id = id.into();
        self.registrar_actions
            .push(Box::new(move |r| r.register_policy(id, policy)));
        self
    }

    pub fn register_authenticatable<M>(&mut self) -> &mut Self
    where
        M: crate::auth::Authenticatable,
    {
        self.registrar_actions
            .push(Box::new(|r| r.register_authenticatable::<M>()));
        self
    }

    pub fn listen_event<E, L>(&mut self, listener: L) -> &mut Self
    where
        E: crate::events::Event,
        L: crate::events::EventListener<E>,
    {
        self.registrar_actions
            .push(Box::new(move |r| r.listen_event::<E, L>(listener)));
        self
    }

    pub fn register_job<J>(&mut self) -> &mut Self
    where
        J: crate::jobs::Job,
    {
        self.registrar_actions
            .push(Box::new(|r| r.register_job::<J>()));
        self
    }

    pub fn register_job_middleware<M>(&mut self, middleware: M) -> &mut Self
    where
        M: crate::jobs::JobMiddleware,
    {
        self.registrar_actions
            .push(Box::new(move |r| r.register_job_middleware(middleware)));
        self
    }

    pub fn register_notification_channel<I, N>(&mut self, id: I, channel: N) -> &mut Self
    where
        I: Into<crate::support::NotificationChannelId> + Send + 'static,
        N: crate::notifications::NotificationChannel,
    {
        let id = id.into();
        self.registrar_actions.push(Box::new(move |r| {
            r.register_notification_channel(id, channel)
        }));
        self
    }

    pub fn register_datatable<D>(&mut self) -> &mut Self
    where
        D: crate::datatable::Datatable,
    {
        self.registrar_actions
            .push(Box::new(|r| r.register_datatable::<D>()));
        self
    }

    pub fn register_readiness_check<I, C>(&mut self, id: I, check: C) -> &mut Self
    where
        I: Into<crate::support::ProbeId> + Send + 'static,
        C: crate::logging::ReadinessCheck,
    {
        let id = id.into();
        self.registrar_actions
            .push(Box::new(move |r| r.register_readiness_check(id, check)));
        self
    }

    pub fn register_storage_driver(
        &mut self,
        name: impl Into<String>,
        factory: crate::storage::StorageDriverFactory,
    ) -> &mut Self {
        let name = name.into();
        self.registrar_actions
            .push(Box::new(move |r| r.register_storage_driver(&name, factory)));
        self
    }

    pub fn register_email_driver(
        &mut self,
        name: impl Into<String>,
        factory: crate::email::EmailDriverFactory,
    ) -> &mut Self {
        let name = name.into();
        self.registrar_actions
            .push(Box::new(move |r| r.register_email_driver(&name, factory)));
        self
    }

    pub fn register_middleware(
        &mut self,
        config: crate::http::middleware::MiddlewareConfig,
    ) -> &mut Self {
        self.middlewares.push(config);
        self
    }
}

/// Summary of what a single plugin contributed during registration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginContributions {
    pub route_count: usize,
    pub command_count: usize,
    pub schedule_count: usize,
    pub websocket_route_count: usize,
    pub validation_rule_count: usize,
    pub provider_count: usize,
    pub middleware_count: usize,
    pub registrar_action_count: usize,
    pub asset_count: usize,
    pub scaffold_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginDependencyDescriptor {
    pub id: PluginId,
    pub version_req: VersionReq,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginAssetDescriptor {
    pub id: PluginAssetId,
    pub kind: PluginAssetKind,
    pub target_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginScaffoldVarDescriptor {
    pub name: String,
    pub description: Option<String>,
    pub default: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginScaffoldDescriptor {
    pub id: PluginScaffoldId,
    pub description: Option<String>,
    pub variables: Vec<PluginScaffoldVarDescriptor>,
    pub files: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginDescriptor {
    pub id: PluginId,
    pub version: Version,
    pub foundry_version: VersionReq,
    pub description: Option<String>,
    pub dependencies: Vec<PluginDependencyDescriptor>,
    pub assets: Vec<PluginAssetDescriptor>,
    pub scaffolds: Vec<PluginScaffoldDescriptor>,
    pub contributions: PluginContributions,
}

pub struct PluginRegistry {
    plugins: Vec<PluginManifest>,
    contributions: HashMap<PluginId, PluginContributions>,
}

impl PluginRegistry {
    pub fn new(
        plugins: Vec<PluginManifest>,
        contributions: HashMap<PluginId, PluginContributions>,
    ) -> Self {
        Self {
            plugins,
            contributions,
        }
    }

    pub fn plugins(&self) -> &[PluginManifest] {
        &self.plugins
    }

    pub fn plugin(&self, id: &PluginId) -> Option<&PluginManifest> {
        self.plugins.iter().find(|plugin| plugin.id() == id)
    }

    pub fn descriptors(&self) -> Vec<PluginDescriptor> {
        self.plugins
            .iter()
            .map(|plugin| PluginDescriptor {
                id: plugin.id().clone(),
                version: plugin.version().clone(),
                foundry_version: plugin.foundry_version().clone(),
                description: plugin.description_text().map(ToOwned::to_owned),
                dependencies: plugin
                    .dependencies()
                    .iter()
                    .map(|dependency| PluginDependencyDescriptor {
                        id: dependency.id().clone(),
                        version_req: dependency.version_req().clone(),
                    })
                    .collect(),
                assets: plugin
                    .assets()
                    .iter()
                    .map(|asset| PluginAssetDescriptor {
                        id: asset.id().clone(),
                        kind: asset.kind().clone(),
                        target_path: asset.target_path().to_path_buf(),
                    })
                    .collect(),
                scaffolds: plugin
                    .scaffolds()
                    .iter()
                    .map(|scaffold| PluginScaffoldDescriptor {
                        id: scaffold.id().clone(),
                        description: scaffold.description_text().map(ToOwned::to_owned),
                        variables: scaffold
                            .variables()
                            .iter()
                            .map(|variable| PluginScaffoldVarDescriptor {
                                name: variable.name().to_string(),
                                description: variable.description_text().map(ToOwned::to_owned),
                                default: variable.default_value().map(ToOwned::to_owned),
                            })
                            .collect(),
                        files: scaffold.files(),
                    })
                    .collect(),
                contributions: self.contributions(plugin.id()).cloned().unwrap_or_default(),
            })
            .collect()
    }

    pub fn install_assets(&self, options: &PluginInstallOptions) -> Result<Vec<PathBuf>> {
        let plugins = self.select_plugins(options.plugin.as_ref(), options.all)?;
        let mut written = Vec::new();
        for plugin in plugins {
            for asset in plugin.assets() {
                let path = write_output_file(
                    &options.target_dir,
                    asset.target_path(),
                    asset.contents(),
                    options.force,
                )?;
                written.push(path);
            }
        }
        Ok(written)
    }

    pub fn render_scaffold(&self, options: &PluginScaffoldOptions) -> Result<Vec<PathBuf>> {
        let plugin = self.plugin(&options.plugin).ok_or_else(|| {
            Error::message(format!("plugin `{}` is not registered", options.plugin))
        })?;
        let scaffold = plugin
            .scaffolds()
            .iter()
            .find(|scaffold| scaffold.id() == &options.scaffold)
            .ok_or_else(|| {
                Error::message(format!(
                    "plugin `{}` does not expose scaffold `{}`",
                    plugin.id(),
                    options.scaffold
                ))
            })?;

        let rendered = scaffold.render(&options.values)?;
        let mut written = Vec::new();
        for (relative_path, contents) in rendered {
            let path = write_output_file(
                &options.target_dir,
                &relative_path,
                &contents,
                options.force,
            )?;
            written.push(path);
        }
        Ok(written)
    }

    pub fn contributions(&self, id: &PluginId) -> Option<&PluginContributions> {
        self.contributions.get(id)
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    fn select_plugins(&self, plugin: Option<&PluginId>, all: bool) -> Result<Vec<&PluginManifest>> {
        if all {
            return Ok(self.plugins.iter().collect());
        }

        match plugin {
            Some(plugin_id) => {
                let plugin = self.plugin(plugin_id).ok_or_else(|| {
                    Error::message(format!("plugin `{plugin_id}` is not registered"))
                })?;
                Ok(vec![plugin])
            }
            None => Err(Error::message(
                "select a plugin with `--plugin` or install from all plugins with `--all`",
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginInstallOptions {
    plugin: Option<PluginId>,
    all: bool,
    force: bool,
    target_dir: PathBuf,
}

impl Default for PluginInstallOptions {
    fn default() -> Self {
        Self {
            plugin: None,
            all: false,
            force: false,
            target_dir: default_target_dir(),
        }
    }
}

impl PluginInstallOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn plugin<I>(mut self, plugin: I) -> Self
    where
        I: Into<PluginId>,
    {
        self.plugin = Some(plugin.into());
        self.all = false;
        self
    }

    pub fn all(mut self) -> Self {
        self.all = true;
        self.plugin = None;
        self
    }

    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    pub fn target_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.target_dir = path.into();
        self
    }
}

#[derive(Clone, Debug)]
pub struct PluginScaffoldOptions {
    plugin: PluginId,
    scaffold: PluginScaffoldId,
    values: BTreeMap<String, String>,
    force: bool,
    target_dir: PathBuf,
}

impl PluginScaffoldOptions {
    pub fn new<P, S>(plugin: P, scaffold: S) -> Self
    where
        P: Into<PluginId>,
        S: Into<PluginScaffoldId>,
    {
        Self {
            plugin: plugin.into(),
            scaffold: scaffold.into(),
            values: BTreeMap::new(),
            force: false,
            target_dir: default_target_dir(),
        }
    }

    pub fn set_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }

    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    pub fn target_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.target_dir = path.into();
        self
    }
}

pub(crate) struct PreparedPlugins {
    pub(crate) registry: Arc<PluginRegistry>,
    pub(crate) instances: Vec<Arc<dyn Plugin>>,
    pub(crate) providers: Vec<Arc<dyn ServiceProvider>>,
    pub(crate) routes: Vec<RouteRegistrar>,
    pub(crate) commands: Vec<CommandRegistrar>,
    pub(crate) schedules: Vec<ScheduleRegistrar>,
    pub(crate) websocket_routes: Vec<WebSocketRouteRegistrar>,
    pub(crate) validation_rules: Vec<(ValidationRuleId, Arc<dyn ValidationRule>)>,
    pub(crate) config_defaults: Vec<Value>,
    pub(crate) middlewares: Vec<crate::http::middleware::MiddlewareConfig>,
    pub(crate) registrar_actions: Vec<RegistrarAction>,
}

struct ResolvedPlugin {
    instance: Arc<dyn Plugin>,
    manifest: PluginManifest,
    providers: Vec<Arc<dyn ServiceProvider>>,
    routes: Vec<RouteRegistrar>,
    commands: Vec<CommandRegistrar>,
    schedules: Vec<ScheduleRegistrar>,
    websocket_routes: Vec<WebSocketRouteRegistrar>,
    validation_rules: Vec<(ValidationRuleId, Arc<dyn ValidationRule>)>,
    config_defaults: Vec<Value>,
    middlewares: Vec<crate::http::middleware::MiddlewareConfig>,
    registrar_actions: Vec<RegistrarAction>,
}

pub(crate) fn prepare_plugins(plugins: &[Arc<dyn Plugin>]) -> Result<PreparedPlugins> {
    let ordered = resolve_plugin_order(plugins)?;
    let mut manifests = Vec::with_capacity(ordered.len());
    let mut instances = Vec::with_capacity(ordered.len());
    let mut providers = Vec::new();
    let mut routes = Vec::new();
    let mut commands = Vec::new();
    let mut schedules = Vec::new();
    let mut websocket_routes = Vec::new();
    let mut validation_rules = Vec::new();
    let mut config_defaults = Vec::new();
    let mut middlewares = Vec::new();
    let mut registrar_actions = Vec::new();
    let mut contributions = HashMap::new();

    for resolved in ordered {
        contributions.insert(
            resolved.manifest.id().clone(),
            PluginContributions {
                route_count: resolved.routes.len(),
                command_count: resolved.commands.len(),
                schedule_count: resolved.schedules.len(),
                websocket_route_count: resolved.websocket_routes.len(),
                validation_rule_count: resolved.validation_rules.len(),
                provider_count: resolved.providers.len(),
                middleware_count: resolved.middlewares.len(),
                registrar_action_count: resolved.registrar_actions.len(),
                asset_count: resolved.manifest.assets().len(),
                scaffold_count: resolved.manifest.scaffolds().len(),
            },
        );
        manifests.push(resolved.manifest);
        instances.push(resolved.instance);
        providers.extend(resolved.providers);
        routes.extend(resolved.routes);
        commands.extend(resolved.commands);
        schedules.extend(resolved.schedules);
        websocket_routes.extend(resolved.websocket_routes);
        validation_rules.extend(resolved.validation_rules);
        config_defaults.extend(resolved.config_defaults);
        middlewares.extend(resolved.middlewares);
        registrar_actions.extend(resolved.registrar_actions);
    }

    Ok(PreparedPlugins {
        registry: Arc::new(PluginRegistry::new(manifests, contributions)),
        instances,
        providers,
        routes,
        commands,
        schedules,
        websocket_routes,
        validation_rules,
        config_defaults,
        middlewares,
        registrar_actions,
    })
}

fn resolve_plugin_order(plugins: &[Arc<dyn Plugin>]) -> Result<Vec<ResolvedPlugin>> {
    let foundry_version = Version::parse(env!("CARGO_PKG_VERSION")).map_err(Error::other)?;
    let mut nodes = Vec::with_capacity(plugins.len());
    let mut by_id = HashMap::new();

    for (index, plugin) in plugins.iter().enumerate() {
        let manifest = plugin_manifest(plugin)?;
        if !manifest.foundry_version().matches(&foundry_version) {
            return Err(Error::message(format!(
                "plugin `{}` requires Foundry `{}` but this build is `{foundry_version}`",
                manifest.id(),
                manifest.foundry_version()
            )));
        }

        if by_id.insert(manifest.id().clone(), index).is_some() {
            return Err(Error::message(format!(
                "plugin `{}` already registered",
                manifest.id()
            )));
        }

        nodes.push((plugin.clone(), manifest));
    }

    for (_, manifest) in &nodes {
        for dependency in manifest.dependencies() {
            let dependency_manifest = nodes
                .get(*by_id.get(dependency.id()).ok_or_else(|| {
                    Error::message(format!(
                        "plugin `{}` depends on missing plugin `{}`",
                        manifest.id(),
                        dependency.id()
                    ))
                })?)
                .map(|(_, manifest)| manifest)
                .expect("plugin dependency index should exist");

            if !dependency
                .version_req()
                .matches(dependency_manifest.version())
            {
                return Err(Error::message(format!(
                    "plugin `{}` requires `{}` {} but found {}",
                    manifest.id(),
                    dependency.id(),
                    dependency.version_req(),
                    dependency_manifest.version()
                )));
            }
        }
    }

    let mut ordered_indexes = Vec::new();
    let mut permanent = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for (index, (_, manifest)) in nodes.iter().enumerate() {
        visit_plugin(
            manifest.id(),
            index,
            &nodes,
            &by_id,
            &mut permanent,
            &mut visiting,
            &mut ordered_indexes,
        )?;
    }

    let mut resolved = Vec::with_capacity(ordered_indexes.len());
    for index in ordered_indexes {
        let (instance, manifest) = nodes[index].clone();
        let registrar = register_plugin(&instance, &manifest)?;
        let manifest = manifest.with_assets_and_scaffolds(registrar.assets, registrar.scaffolds);
        resolved.push(ResolvedPlugin {
            instance,
            manifest,
            providers: registrar.providers,
            routes: registrar.routes,
            commands: registrar.commands,
            schedules: registrar.schedules,
            websocket_routes: registrar.websocket_routes,
            validation_rules: registrar.validation_rules,
            config_defaults: registrar.config_defaults,
            middlewares: registrar.middlewares,
            registrar_actions: registrar.registrar_actions,
        });
    }

    Ok(resolved)
}

fn visit_plugin(
    id: &PluginId,
    index: usize,
    nodes: &[(Arc<dyn Plugin>, PluginManifest)],
    by_id: &HashMap<PluginId, usize>,
    permanent: &mut BTreeSet<PluginId>,
    visiting: &mut BTreeSet<PluginId>,
    ordered_indexes: &mut Vec<usize>,
) -> Result<()> {
    if permanent.contains(id) {
        return Ok(());
    }

    if !visiting.insert(id.clone()) {
        return Err(Error::message(format!(
            "plugin dependency cycle detected at `{id}`"
        )));
    }

    let manifest = &nodes[index].1;
    for dependency in manifest.dependencies() {
        let dependency_index = by_id.get(dependency.id()).copied().ok_or_else(|| {
            Error::message(format!(
                "plugin `{}` depends on missing plugin `{}`",
                manifest.id(),
                dependency.id()
            ))
        })?;
        visit_plugin(
            dependency.id(),
            dependency_index,
            nodes,
            by_id,
            permanent,
            visiting,
            ordered_indexes,
        )?;
    }

    visiting.remove(id);
    permanent.insert(id.clone());
    ordered_indexes.push(index);
    Ok(())
}

pub(crate) fn builtin_cli_registrar() -> CommandRegistrar {
    Arc::new(|registry| {
        registry.command(
            PLUGIN_LIST_COMMAND,
            Command::new(PLUGIN_LIST_COMMAND.as_str().to_string())
                .about("List registered Foundry plugins"),
            |invocation| async move { plugin_list_command(invocation).await },
        )?;
        registry.command(
            PLUGIN_INSTALL_ASSETS_COMMAND,
            Command::new(PLUGIN_INSTALL_ASSETS_COMMAND.as_str().to_string())
                .about("Install plugin assets into the current app")
                .arg(
                    Arg::new("plugin")
                        .long("plugin")
                        .value_name("PLUGIN_ID")
                        .help("Install assets from one plugin"),
                )
                .arg(
                    Arg::new("all")
                        .long("all")
                        .action(ArgAction::SetTrue)
                        .help("Install assets from every registered plugin"),
                )
                .arg(
                    Arg::new("to")
                        .long("to")
                        .value_name("PATH")
                        .help("Target directory for installed assets"),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .action(ArgAction::SetTrue)
                        .help("Overwrite existing files"),
                ),
            |invocation| async move { plugin_install_assets_command(invocation).await },
        )?;
        registry.command(
            PLUGIN_SCAFFOLD_COMMAND,
            Command::new(PLUGIN_SCAFFOLD_COMMAND.as_str().to_string())
                .about("Render a plugin scaffold into the current app")
                .arg(
                    Arg::new("plugin")
                        .long("plugin")
                        .required(true)
                        .value_name("PLUGIN_ID")
                        .help("Plugin that owns the scaffold"),
                )
                .arg(
                    Arg::new("template")
                        .long("template")
                        .required(true)
                        .value_name("SCAFFOLD_ID")
                        .help("Scaffold template identifier"),
                )
                .arg(
                    Arg::new("set")
                        .long("set")
                        .value_name("KEY=VALUE")
                        .action(ArgAction::Append)
                        .help("Assign a scaffold variable"),
                )
                .arg(
                    Arg::new("to")
                        .long("to")
                        .value_name("PATH")
                        .help("Target directory for rendered files"),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .action(ArgAction::SetTrue)
                        .help("Overwrite existing files"),
                ),
            |invocation| async move { plugin_scaffold_command(invocation).await },
        )?;
        Ok(())
    })
}

async fn plugin_list_command(invocation: CommandInvocation) -> Result<()> {
    let registry = invocation.app().plugins()?;
    for plugin in registry.plugins() {
        let dependencies = if plugin.dependencies().is_empty() {
            "none".to_string()
        } else {
            plugin
                .dependencies()
                .iter()
                .map(|dependency| format!("{} {}", dependency.id(), dependency.version_req()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        print!(
            "{} v{} | deps: {}",
            plugin.id(),
            plugin.version(),
            dependencies,
        );
        if let Some(contrib) = registry.contributions(plugin.id()) {
            let mut parts = Vec::new();
            if contrib.route_count > 0 {
                parts.push(format!("{} routes", contrib.route_count));
            }
            if contrib.command_count > 0 {
                parts.push(format!("{} commands", contrib.command_count));
            }
            if contrib.schedule_count > 0 {
                parts.push(format!("{} schedules", contrib.schedule_count));
            }
            if contrib.provider_count > 0 {
                parts.push(format!("{} providers", contrib.provider_count));
            }
            if contrib.middleware_count > 0 {
                parts.push(format!("{} middlewares", contrib.middleware_count));
            }
            if contrib.registrar_action_count > 0 {
                parts.push(format!("{} registrations", contrib.registrar_action_count));
            }
            if contrib.asset_count > 0 {
                parts.push(format!("{} assets", contrib.asset_count));
            }
            if contrib.scaffold_count > 0 {
                parts.push(format!("{} scaffolds", contrib.scaffold_count));
            }
            if !parts.is_empty() {
                print!(" | {}", parts.join(", "));
            }
        }
        println!();
    }
    Ok(())
}

async fn plugin_install_assets_command(invocation: CommandInvocation) -> Result<()> {
    let matches = invocation.matches();
    let mut options = PluginInstallOptions::new();
    if let Some(path) = matches.get_one::<String>("to") {
        options = options.target_dir(path);
    }
    if matches.get_flag("force") {
        options = options.force();
    }
    if matches.get_flag("all") {
        options = options.all();
    } else if let Some(plugin) = matches.get_one::<String>("plugin") {
        options = options.plugin(PluginId::owned(plugin.clone()));
    }

    let registry = invocation.app().plugins()?;
    let written = registry.install_assets(&options)?;
    for path in written {
        println!("{}", path.display());
    }
    Ok(())
}

async fn plugin_scaffold_command(invocation: CommandInvocation) -> Result<()> {
    let matches = invocation.matches();
    let plugin = matches
        .get_one::<String>("plugin")
        .cloned()
        .ok_or_else(|| Error::message("missing `--plugin`"))?;
    let template = matches
        .get_one::<String>("template")
        .cloned()
        .ok_or_else(|| Error::message("missing `--template`"))?;
    let mut options =
        PluginScaffoldOptions::new(PluginId::owned(plugin), PluginScaffoldId::owned(template));
    if let Some(path) = matches.get_one::<String>("to") {
        options = options.target_dir(path);
    }
    if matches.get_flag("force") {
        options = options.force();
    }
    if let Some(values) = matches.get_many::<String>("set") {
        for assignment in values {
            let (key, value) = assignment.split_once('=').ok_or_else(|| {
                Error::message(format!(
                    "invalid scaffold assignment `{assignment}`, expected KEY=VALUE"
                ))
            })?;
            options = options.set_var(key, value);
        }
    }

    let registry = invocation.app().plugins()?;
    let written = registry.render_scaffold(&options)?;
    for path in written {
        println!("{}", path.display());
    }
    Ok(())
}

fn default_target_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn write_output_file(
    target_dir: &Path,
    relative_path: &Path,
    contents: &[u8],
    force: bool,
) -> Result<PathBuf> {
    validate_relative_output_path(relative_path, "plugin output")?;
    reject_existing_output_symlinks(target_dir, relative_path)?;

    let path = target_dir.join(relative_path);
    if path.exists() && !force {
        return Err(Error::message(format!(
            "refusing to overwrite existing file `{}` without `--force`",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(Error::other)?;
    }
    fs::write(&path, contents).map_err(Error::other)?;
    Ok(path)
}

fn validate_relative_output_path(path: &Path, label: &str) -> Result<()> {
    let display = path.display();
    if path.as_os_str().is_empty() {
        return Err(Error::message(format!("{label} path cannot be empty")));
    }
    if path.is_absolute() {
        return Err(Error::message(format!(
            "{label} path `{display}` must be relative"
        )));
    }

    let raw = path
        .to_str()
        .ok_or_else(|| Error::message(format!("{label} path `{display}` must be valid UTF-8")))?;
    if raw.contains('\\') {
        return Err(Error::message(format!(
            "{label} path `{display}` cannot contain backslash separators"
        )));
    }

    let mut has_component = false;
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                has_component = true;
                let value = value.to_str().ok_or_else(|| {
                    Error::message(format!(
                        "{label} path `{display}` contains a non-UTF-8 component"
                    ))
                })?;
                if value.chars().any(|ch| ch.is_control()) {
                    return Err(Error::message(format!(
                        "{label} path `{display}` cannot contain control characters"
                    )));
                }
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(Error::message(format!(
                    "{label} path `{display}` must stay inside the target directory"
                )));
            }
        }
    }

    if !has_component {
        return Err(Error::message(format!("{label} path cannot be empty")));
    }

    Ok(())
}

fn reject_existing_output_symlinks(target_dir: &Path, relative_path: &Path) -> Result<()> {
    let mut current = target_dir.to_path_buf();
    for component in relative_path.components() {
        let Component::Normal(value) = component else {
            continue;
        };
        current.push(value);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::message(format!(
                    "refusing to write plugin output through symlink `{}`",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(Error::other(error)),
        }
    }

    Ok(())
}

fn render_template(template: &str, values: &BTreeMap<String, String>) -> Result<String> {
    let mut rendered = template.to_string();
    for (key, value) in values {
        rendered = rendered.replace(&format!("{{{{{key}}}}}"), value);
    }

    if let Some(start) = rendered.find("{{") {
        if let Some(end) = rendered[start + 2..].find("}}") {
            let unresolved = &rendered[start + 2..start + 2 + end];
            return Err(Error::message(format!(
                "unresolved scaffold variable `{}`",
                unresolved.trim()
            )));
        }
    }

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use semver::{Version, VersionReq};
    use tempfile::tempdir;

    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{
        prepare_plugins, run_plugin_lifecycle_callback, Plugin, PluginAsset, PluginAssetKind,
        PluginDependency, PluginId, PluginInstallOptions, PluginManifest, PluginRegistrar,
        PluginRegistry, PluginScaffold, PluginScaffoldOptions, PluginScaffoldVar,
    };
    use crate::foundation::{AppContext, Error, Result, ServiceProvider, ServiceRegistrar};
    use crate::support::{PluginAssetId, PluginScaffoldId};

    struct EmptyPlugin {
        manifest: PluginManifest,
    }

    struct RegisterPanicPlugin {
        manifest: PluginManifest,
    }

    struct ManifestPanicPlugin;

    impl EmptyPlugin {
        fn new(manifest: PluginManifest) -> Self {
            Self { manifest }
        }
    }

    #[async_trait]
    impl Plugin for EmptyPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        fn register(&self, _registrar: &mut PluginRegistrar) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl Plugin for RegisterPanicPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        fn register(&self, _registrar: &mut PluginRegistrar) -> Result<()> {
            panic!("register boom")
        }
    }

    #[async_trait]
    impl Plugin for ManifestPanicPlugin {
        fn manifest(&self) -> PluginManifest {
            panic!("manifest boom")
        }

        fn register(&self, _registrar: &mut PluginRegistrar) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rejects_duplicate_plugin_ids() {
        let manifest = PluginManifest::new(
            PluginId::new("foundry.example"),
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        );
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(EmptyPlugin::new(manifest.clone())),
            Arc::new(EmptyPlugin::new(manifest)),
        ];

        let error = prepare_plugins(&plugins).err().unwrap();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn rejects_missing_plugin_dependencies() {
        let manifest = PluginManifest::new(
            PluginId::new("foundry.example"),
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .dependency(PluginDependency::new(
            PluginId::new("foundry.base"),
            VersionReq::parse("^1").unwrap(),
        ));
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(EmptyPlugin::new(manifest))];

        let error = prepare_plugins(&plugins).err().unwrap();
        assert!(error.to_string().contains("missing plugin"));
    }

    #[test]
    fn rejects_dependency_cycles() {
        let first = PluginManifest::new(
            PluginId::new("foundry.first"),
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .dependency(PluginDependency::new(
            PluginId::new("foundry.second"),
            VersionReq::parse("^1").unwrap(),
        ));
        let second = PluginManifest::new(
            PluginId::new("foundry.second"),
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .dependency(PluginDependency::new(
            PluginId::new("foundry.first"),
            VersionReq::parse("^1").unwrap(),
        ));

        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(EmptyPlugin::new(first)),
            Arc::new(EmptyPlugin::new(second)),
        ];

        let error = prepare_plugins(&plugins).err().unwrap();
        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn sorts_dependencies_before_dependents() {
        let base = PluginManifest::new(
            PluginId::new("foundry.base"),
            Version::parse("1.2.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        );
        let dependent = PluginManifest::new(
            PluginId::new("foundry.dependent"),
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .dependency(PluginDependency::new(
            PluginId::new("foundry.base"),
            VersionReq::parse("^1.2").unwrap(),
        ));
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(EmptyPlugin::new(dependent)),
            Arc::new(EmptyPlugin::new(base)),
        ];

        let prepared = prepare_plugins(&plugins).unwrap();
        let ids = prepared
            .registry
            .plugins()
            .iter()
            .map(|manifest| manifest.id().clone())
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                PluginId::new("foundry.base"),
                PluginId::new("foundry.dependent")
            ]
        );
    }

    #[test]
    fn rejects_incompatible_foundry_version() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(EmptyPlugin::new(PluginManifest::new(
            PluginId::new("foundry.example"),
            Version::parse("1.0.0").unwrap(),
            VersionReq::parse(">=9").unwrap(),
        )))];

        let error = prepare_plugins(&plugins).err().unwrap();
        assert!(error.to_string().contains("requires Foundry"));
    }

    #[test]
    fn plugin_manifest_panic_becomes_prepare_error() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ManifestPanicPlugin)];

        let error = prepare_plugins(&plugins).err().unwrap();

        assert!(error.to_string().contains("plugin manifest panicked"));
        assert!(error.to_string().contains("manifest boom"));
    }

    #[test]
    fn plugin_register_panic_becomes_prepare_error() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(RegisterPanicPlugin {
            manifest: PluginManifest::new(
                PluginId::new("foundry.panic.register"),
                Version::parse("1.0.0").unwrap(),
                VersionReq::parse("^0.1").unwrap(),
            ),
        })];

        let error = prepare_plugins(&plugins).err().unwrap();

        assert!(error
            .to_string()
            .contains("plugin `foundry.panic.register` register panicked"));
        assert!(error.to_string().contains("register boom"));
    }

    #[tokio::test]
    async fn plugin_lifecycle_error_remains_unchanged() {
        let id = PluginId::new("foundry.lifecycle.error");

        let error = run_plugin_lifecycle_callback(&id, "boot", || async {
            Err(Error::message("plugin boot failed"))
        })
        .await
        .unwrap_err();

        assert_eq!(error.to_string(), "plugin boot failed");
    }

    #[tokio::test]
    async fn plugin_lifecycle_factory_panic_becomes_error() {
        let id = PluginId::new("foundry.lifecycle.factory_panic");

        let error =
            run_plugin_lifecycle_callback(&id, "boot", || -> std::future::Ready<Result<()>> {
                panic!("plugin factory boom")
            })
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains(
                "plugin `foundry.lifecycle.factory_panic` boot panicked: plugin factory boom"
            ),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn plugin_lifecycle_future_panic_becomes_error() {
        let id = PluginId::new("foundry.lifecycle.future_panic");

        let error = run_plugin_lifecycle_callback(&id, "shutdown", || async {
            panic!("plugin future boom");
            #[allow(unreachable_code)]
            Ok(())
        })
        .await
        .unwrap_err();

        assert!(
            error.to_string().contains(
                "plugin `foundry.lifecycle.future_panic` shutdown panicked: plugin future boom"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn installs_assets_and_detects_collisions() {
        let directory = tempdir().unwrap();
        let registry = PluginRegistry::new(
            vec![PluginManifest::new(
                PluginId::new("foundry.example"),
                Version::parse("1.0.0").unwrap(),
                VersionReq::parse("^0.1").unwrap(),
            )
            .with_assets_and_scaffolds(
                vec![PluginAsset::text(
                    PluginAssetId::new("config"),
                    PluginAssetKind::Config,
                    "config/plugin.toml",
                    "enabled = true\n",
                )],
                Vec::new(),
            )],
            HashMap::new(),
        );

        let written = registry
            .install_assets(
                &PluginInstallOptions::new()
                    .plugin(PluginId::new("foundry.example"))
                    .target_dir(directory.path()),
            )
            .unwrap();
        assert_eq!(written.len(), 1);

        let error = registry
            .install_assets(
                &PluginInstallOptions::new()
                    .plugin(PluginId::new("foundry.example"))
                    .target_dir(directory.path()),
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains("refusing to overwrite"));
    }

    #[test]
    fn rejects_plugin_asset_paths_outside_target_dir() {
        let directory = tempdir().unwrap();
        let outside = directory.path().join("outside.toml");
        let registry = PluginRegistry::new(
            vec![PluginManifest::new(
                PluginId::new("foundry.example"),
                Version::parse("1.0.0").unwrap(),
                VersionReq::parse("^0.1").unwrap(),
            )
            .with_assets_and_scaffolds(
                vec![PluginAsset::text(
                    PluginAssetId::new("escape"),
                    PluginAssetKind::Config,
                    "../outside.toml",
                    "enabled = true\n",
                )],
                Vec::new(),
            )],
            HashMap::new(),
        );

        let error = registry
            .install_assets(
                &PluginInstallOptions::new()
                    .plugin(PluginId::new("foundry.example"))
                    .target_dir(directory.path().join("app")),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains("must stay inside"),
            "unexpected error: {error}"
        );
        assert!(!outside.exists());
    }

    #[test]
    fn rejects_plugin_asset_backslash_paths() {
        let mut registrar = PluginRegistrar::new();
        let error = registrar
            .register_assets(vec![PluginAsset::text(
                PluginAssetId::new("windows-path"),
                PluginAssetKind::Config,
                "config\\plugin.toml",
                "enabled = true\n",
            )])
            .err()
            .unwrap();

        assert!(
            error.to_string().contains("backslash"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn renders_scaffolds_with_validation() {
        let directory = tempdir().unwrap();
        let registry = PluginRegistry::new(
            vec![PluginManifest::new(
                PluginId::new("foundry.example"),
                Version::parse("1.0.0").unwrap(),
                VersionReq::parse("^0.1").unwrap(),
            )
            .with_assets_and_scaffolds(
                Vec::new(),
                vec![PluginScaffold::new(PluginScaffoldId::new("portal"))
                    .variable(PluginScaffoldVar::new("name"))
                    .file(
                        "src/app/{{name}}.rs",
                        "pub const NAME: &str = \"{{name}}\";\n",
                    )],
            )],
            HashMap::new(),
        );

        registry
            .render_scaffold(
                &PluginScaffoldOptions::new(
                    PluginId::new("foundry.example"),
                    PluginScaffoldId::new("portal"),
                )
                .set_var("name", "dashboard")
                .target_dir(directory.path()),
            )
            .unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join("src/app/dashboard.rs")).unwrap(),
            "pub const NAME: &str = \"dashboard\";\n"
        );

        let error = registry
            .render_scaffold(
                &PluginScaffoldOptions::new(
                    PluginId::new("foundry.example"),
                    PluginScaffoldId::new("portal"),
                )
                .set_var("name", "dashboard")
                .set_var("extra", "value")
                .target_dir(directory.path())
                .force(),
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains("unknown scaffold variable"));
    }

    #[test]
    fn rejects_rendered_scaffold_paths_outside_target_dir() {
        let directory = tempdir().unwrap();
        let registry = PluginRegistry::new(
            vec![PluginManifest::new(
                PluginId::new("foundry.example"),
                Version::parse("1.0.0").unwrap(),
                VersionReq::parse("^0.1").unwrap(),
            )
            .with_assets_and_scaffolds(
                Vec::new(),
                vec![PluginScaffold::new(PluginScaffoldId::new("portal"))
                    .variable(PluginScaffoldVar::new("name"))
                    .file("src/app/{{name}}.rs", "pub const NAME: &str = \"x\";\n")],
            )],
            HashMap::new(),
        );

        let error = registry
            .render_scaffold(
                &PluginScaffoldOptions::new(
                    PluginId::new("foundry.example"),
                    PluginScaffoldId::new("portal"),
                )
                .set_var("name", "../../outside")
                .target_dir(directory.path()),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains("must stay inside"),
            "unexpected error: {error}"
        );
        assert!(!directory.path().join("outside.rs").exists());
    }

    #[test]
    fn rejects_static_scaffold_paths_outside_target_dir_at_registration() {
        let mut registrar = PluginRegistrar::new();
        let error = registrar
            .register_scaffolds(vec![
                PluginScaffold::new(PluginScaffoldId::new("escape")).file("../outside.rs", "")
            ])
            .err()
            .unwrap();

        assert!(
            error.to_string().contains("must stay inside"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_write_plugin_output_through_symlink() {
        let directory = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir_all(directory.path().join("config")).unwrap();
        std::os::unix::fs::symlink(
            outside.path(),
            directory.path().join("config").join("linked"),
        )
        .unwrap();

        let registry = PluginRegistry::new(
            vec![PluginManifest::new(
                PluginId::new("foundry.example"),
                Version::parse("1.0.0").unwrap(),
                VersionReq::parse("^0.1").unwrap(),
            )
            .with_assets_and_scaffolds(
                vec![PluginAsset::text(
                    PluginAssetId::new("asset"),
                    PluginAssetKind::Config,
                    "config/linked/plugin.toml",
                    "enabled = true\n",
                )],
                Vec::new(),
            )],
            HashMap::new(),
        );

        let error = registry
            .install_assets(
                &PluginInstallOptions::new()
                    .plugin(PluginId::new("foundry.example"))
                    .target_dir(directory.path()),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains("through symlink"),
            "unexpected error: {error}"
        );
        assert!(!outside.path().join("plugin.toml").exists());
    }

    struct ProviderPlugin {
        manifest: PluginManifest,
        order: Arc<Mutex<Vec<&'static str>>>,
    }

    struct MarkerProvider {
        order: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl ServiceProvider for MarkerProvider {
        async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
            registrar.singleton(String::from("plugin"))?;
            self.order.lock().unwrap().push("provider-register");
            Ok(())
        }

        async fn boot(&self, _app: &AppContext) -> Result<()> {
            self.order.lock().unwrap().push("provider-boot");
            Ok(())
        }
    }

    #[async_trait]
    impl Plugin for ProviderPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
            registrar.register_provider(MarkerProvider {
                order: self.order.clone(),
            });
            Ok(())
        }

        async fn boot(&self, _app: &AppContext) -> Result<()> {
            self.order.lock().unwrap().push("plugin-boot");
            Ok(())
        }
    }

    #[test]
    fn plugin_registrar_collects_provider_contributions() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ProviderPlugin {
            manifest: PluginManifest::new(
                PluginId::new("foundry.example"),
                Version::parse("1.0.0").unwrap(),
                VersionReq::parse("^0.1").unwrap(),
            ),
            order: order.clone(),
        })];

        let prepared = prepare_plugins(&plugins).unwrap();
        assert_eq!(prepared.providers.len(), 1);
        assert_eq!(prepared.instances.len(), 1);
    }

    #[test]
    fn plugin_registry_descriptors_expose_manifest_and_contribution_metadata() {
        let manifest = PluginManifest::new(
            PluginId::new("foundry.example"),
            Version::parse("1.2.3").unwrap(),
            VersionReq::parse("^0.1").unwrap(),
        )
        .description("Example plugin")
        .depends_on(
            PluginId::new("foundry.base"),
            VersionReq::parse("^1").unwrap(),
        );
        let asset = PluginAsset::text(
            PluginAssetId::new("config"),
            PluginAssetKind::Config,
            "config/example.toml",
            "enabled = true",
        );
        let scaffold = PluginScaffold::new(PluginScaffoldId::new("model"))
            .description("Create a model")
            .variable(
                PluginScaffoldVar::new("name")
                    .description("Model name")
                    .default("User"),
            )
            .file("src/{{name}}.rs", "pub struct {{name}};");
        let registry = PluginRegistry::new(
            vec![manifest.with_assets_and_scaffolds(vec![asset], vec![scaffold])],
            HashMap::from([(
                PluginId::new("foundry.example"),
                super::PluginContributions {
                    route_count: 1,
                    command_count: 2,
                    schedule_count: 3,
                    websocket_route_count: 4,
                    validation_rule_count: 5,
                    provider_count: 6,
                    middleware_count: 7,
                    registrar_action_count: 8,
                    asset_count: 1,
                    scaffold_count: 1,
                },
            )]),
        );

        let descriptors = registry.descriptors();

        assert_eq!(descriptors.len(), 1);
        let descriptor = &descriptors[0];
        assert_eq!(descriptor.id, PluginId::new("foundry.example"));
        assert_eq!(descriptor.version, Version::parse("1.2.3").unwrap());
        assert_eq!(
            descriptor.foundry_version,
            VersionReq::parse("^0.1").unwrap()
        );
        assert_eq!(descriptor.description.as_deref(), Some("Example plugin"));
        assert_eq!(descriptor.dependencies[0].id, PluginId::new("foundry.base"));
        assert_eq!(
            descriptor.dependencies[0].version_req,
            VersionReq::parse("^1").unwrap()
        );
        assert_eq!(descriptor.assets[0].id, PluginAssetId::new("config"));
        assert_eq!(descriptor.assets[0].kind, PluginAssetKind::Config);
        assert_eq!(
            descriptor.assets[0].target_path,
            PathBuf::from("config/example.toml")
        );
        assert_eq!(descriptor.scaffolds[0].id, PluginScaffoldId::new("model"));
        assert_eq!(
            descriptor.scaffolds[0].variables[0].name,
            "name".to_string()
        );
        assert_eq!(
            descriptor.scaffolds[0].files,
            vec![PathBuf::from("src/{{name}}.rs")]
        );
        assert_eq!(descriptor.contributions.command_count, 2);
        assert_eq!(descriptor.contributions.validation_rule_count, 5);
    }

    #[test]
    fn resolves_diamond_dependency_graph() {
        // A depends on B and C; both B and C depend on D.
        // Expected order: D, then B and C (either order), then A.
        let d = PluginManifest::new(
            PluginId::new("d"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        );
        let b = PluginManifest::new(
            PluginId::new("b"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        )
        .depends_on(PluginId::new("d"), VersionReq::parse("^1").unwrap());
        let c = PluginManifest::new(
            PluginId::new("c"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        )
        .depends_on(PluginId::new("d"), VersionReq::parse("^1").unwrap());
        let a = PluginManifest::new(
            PluginId::new("a"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        )
        .depends_on(PluginId::new("b"), VersionReq::parse("^1").unwrap())
        .depends_on(PluginId::new("c"), VersionReq::parse("^1").unwrap());

        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(EmptyPlugin::new(a)),
            Arc::new(EmptyPlugin::new(b)),
            Arc::new(EmptyPlugin::new(c)),
            Arc::new(EmptyPlugin::new(d)),
        ];

        let prepared = prepare_plugins(&plugins).unwrap();
        let ids: Vec<_> = prepared
            .instances
            .iter()
            .map(|p| p.manifest().id().clone())
            .collect();

        // D must be first (leaf dependency)
        assert_eq!(ids[0], PluginId::new("d"));
        // A must be last (depends on everything)
        assert_eq!(*ids.last().unwrap(), PluginId::new("a"));
        // B and C are in the middle (either order is valid)
        assert!(ids[1] == PluginId::new("b") || ids[1] == PluginId::new("c"));
        assert!(ids[2] == PluginId::new("b") || ids[2] == PluginId::new("c"));
        assert_ne!(ids[1], ids[2]);
    }

    #[test]
    fn rejects_conflicting_version_requirements_in_nested_deps() {
        // B requires D ^1.0, C requires D ^2.0, but only D 1.0 is registered.
        let d = PluginManifest::new(
            PluginId::new("d"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        );
        let b = PluginManifest::new(
            PluginId::new("b"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        )
        .depends_on(PluginId::new("d"), VersionReq::parse("^1").unwrap());
        let c = PluginManifest::new(
            PluginId::new("c"),
            Version::new(1, 0, 0),
            VersionReq::parse(">=0.1").unwrap(),
        )
        .depends_on(PluginId::new("d"), VersionReq::parse("^2").unwrap()); // conflict!

        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(EmptyPlugin::new(b)),
            Arc::new(EmptyPlugin::new(c)),
            Arc::new(EmptyPlugin::new(d)),
        ];

        let error = match prepare_plugins(&plugins) {
            Err(e) => e,
            Ok(_) => panic!("expected version mismatch error"),
        };
        assert!(
            error.to_string().contains("requires") && error.to_string().contains("found"),
            "expected version mismatch error, got: {error}"
        );
    }

    #[test]
    fn registrar_collects_direct_registrations_and_middlewares() {
        let mut registrar = PluginRegistrar::new();
        registrar.register_middleware(crate::http::middleware::MiddlewareConfig::Compression(
            crate::http::middleware::Compression,
        ));

        // The middleware should be collected
        assert_eq!(registrar.middlewares.len(), 1);
        // Registrar actions are collected but not yet applied
        assert!(registrar.registrar_actions.is_empty());
    }
}
