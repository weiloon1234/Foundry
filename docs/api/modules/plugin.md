# plugin

Compile-time plugin system with dependency validation

[Back to index](../index.md)

## foundry::plugin

```rust
enum PluginAssetKind { Config, Migration, Static }
struct PluginAsset
  fn text<I, P>( id: I, kind: PluginAssetKind, target_path: P, contents: impl Into<String>, ) -> Self
  fn binary<I, P>( id: I, kind: PluginAssetKind, target_path: P, contents: impl Into<Vec<u8>>, ) -> Self
  fn id(&self) -> &PluginAssetId
  fn kind(&self) -> &PluginAssetKind
  fn target_path(&self) -> &Path
struct PluginAssetDescriptor
struct PluginContributions
struct PluginDependency
  fn new<I>(id: I, version_req: VersionReq) -> Self
  fn id(&self) -> &PluginId
  fn version_req(&self) -> &VersionReq
struct PluginDependencyDescriptor
struct PluginDescriptor
struct PluginInstallOptions
  fn new() -> Self
  fn plugin<I>(self, plugin: I) -> Self
  fn all(self) -> Self
  fn force(self) -> Self
  fn target_dir(self, path: impl Into<PathBuf>) -> Self
struct PluginManifest
  fn new<I>(id: I, version: Version, foundry_version: VersionReq) -> Self
  fn description(self, description: impl Into<String>) -> Self
  fn dependency(self, dependency: PluginDependency) -> Self
  fn depends_on<I>(self, id: I, version_req: VersionReq) -> Self
  fn id(&self) -> &PluginId
  fn version(&self) -> &Version
  fn foundry_version(&self) -> &VersionReq
  fn dependencies(&self) -> &[PluginDependency]
  fn description_text(&self) -> Option<&str>
  fn assets(&self) -> &[PluginAsset]
  fn scaffolds(&self) -> &[PluginScaffold]
struct PluginRegistrar
  fn new() -> Self
  fn register_provider<P>(&mut self, provider: P) -> &mut Self
  fn register_routes<F>(&mut self, registrar: F) -> &mut Self
  fn register_commands<F>(&mut self, registrar: F) -> &mut Self
  fn register_schedule<F>(&mut self, registrar: F) -> &mut Self
  fn register_websocket_routes<F>(&mut self, registrar: F) -> &mut Self
  fn register_validation_rule<I, R>(&mut self, id: I, rule: R) -> &mut Self
  fn config_defaults(&mut self, defaults: Value) -> &mut Self
  fn register_assets<I>(&mut self, assets: I) -> Result<&mut Self>
  fn register_scaffolds<I>(&mut self, scaffolds: I) -> Result<&mut Self>
  fn register_guard<I, G>(&mut self, id: I, guard: G) -> &mut Self
  fn register_policy<I, P>(&mut self, id: I, policy: P) -> &mut Self
  fn register_authenticatable<M>(&mut self) -> &mut Self
  fn listen_event<E, L>(&mut self, listener: L) -> &mut Self
  fn register_job<J>(&mut self) -> &mut Self
  fn register_job_middleware<M>(&mut self, middleware: M) -> &mut Self
  fn register_notification_channel<I, N>( &mut self, id: I, channel: N, ) -> &mut Self
  fn register_datatable<D>(&mut self) -> &mut Self
  fn register_readiness_check<I, C>(&mut self, id: I, check: C) -> &mut Self
  fn register_storage_driver( &mut self, name: impl Into<String>, factory: StorageDriverFactory, ) -> &mut Self
  fn register_email_driver( &mut self, name: impl Into<String>, factory: EmailDriverFactory, ) -> &mut Self
  fn register_middleware(&mut self, config: MiddlewareConfig) -> &mut Self
struct PluginRegistry
  fn new( plugins: Vec<PluginManifest>, contributions: HashMap<PluginId, PluginContributions>, ) -> Self
  fn plugins(&self) -> &[PluginManifest]
  fn plugin(&self, id: &PluginId) -> Option<&PluginManifest>
  fn descriptors(&self) -> Vec<PluginDescriptor>
  fn install_assets( &self, options: &PluginInstallOptions, ) -> Result<Vec<PathBuf>>
  fn render_scaffold( &self, options: &PluginScaffoldOptions, ) -> Result<Vec<PathBuf>>
  fn contributions(&self, id: &PluginId) -> Option<&PluginContributions>
  fn is_empty(&self) -> bool
struct PluginScaffold
  fn new<I>(id: I) -> Self
  fn description(self, description: impl Into<String>) -> Self
  fn variable(self, variable: PluginScaffoldVar) -> Self
  fn file(self, path: impl Into<PathBuf>, contents: impl Into<String>) -> Self
  fn id(&self) -> &PluginScaffoldId
  fn description_text(&self) -> Option<&str>
  fn variables(&self) -> &[PluginScaffoldVar]
  fn files(&self) -> Vec<PathBuf>
struct PluginScaffoldDescriptor
struct PluginScaffoldOptions
  fn new<P, S>(plugin: P, scaffold: S) -> Self
  fn set_var(self, key: impl Into<String>, value: impl Into<String>) -> Self
  fn force(self) -> Self
  fn target_dir(self, path: impl Into<PathBuf>) -> Self
struct PluginScaffoldVar
  fn new(name: impl Into<String>) -> Self
  fn description(self, description: impl Into<String>) -> Self
  fn default(self, value: impl Into<String>) -> Self
  fn name(&self) -> &str
  fn description_text(&self) -> Option<&str>
struct PluginScaffoldVarDescriptor
  fn default_value(&self) -> Option<&str>
trait Plugin
  fn manifest(&self) -> PluginManifest
  fn register(&self, registrar: &mut PluginRegistrar) -> Result<()>
  fn boot<'life0, 'life1, 'async_trait>(
  fn shutdown<'life0, 'life1, 'async_trait>(
```
