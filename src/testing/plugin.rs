use std::path::PathBuf;
use std::sync::Arc;

use crate::foundation::{AppContext, Error, Result};
use crate::plugin::{Plugin, PluginContributions, PluginManifest, PluginRegistry};
use crate::support::PluginId;

use super::client::{TestApp, TestAppBuilder, TestClient};

/// Builds one plugin with its dependencies inside a real Foundry test app.
///
/// The primary plugin ID is checked after bootstrap, and the resulting
/// [`PluginTestApp`] exposes its resolved manifest and contribution summary.
/// Additional plugins can be registered in any order; Foundry still resolves
/// their declared dependency order.
///
/// This is also the minimal author test template:
///
/// ```no_run
/// use foundry::prelude::*;
/// use semver::{Version, VersionReq};
///
/// const EXAMPLE_PLUGIN: PluginId = PluginId::new("example.plugin");
///
/// struct ExamplePlugin;
///
/// impl Plugin for ExamplePlugin {
///     fn manifest(&self) -> PluginManifest {
///         PluginManifest::new(
///             EXAMPLE_PLUGIN,
///             Version::new(1, 0, 0),
///             VersionReq::parse(">=0.1").unwrap(),
///         )
///     }
///
///     fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
///         registrar.register_routes(|routes| {
///             routes.route("/example", axum::routing::get(|| async { "ok" }));
///             Ok(())
///         });
///         Ok(())
///     }
/// }
///
/// # async fn author_test() -> Result<()> {
/// let app = PluginTestHarness::new(EXAMPLE_PLUGIN, ExamplePlugin)
///     .build()
///     .await?;
///
/// assert_eq!(app.manifest().id(), &EXAMPLE_PLUGIN);
/// assert_eq!(app.contributions().route_count, 1);
/// assert_eq!(
///     app.client().get("/example").send().await?.status(),
///     axum::http::StatusCode::OK,
/// );
///
/// app.shutdown().await
/// # }
/// ```
pub struct PluginTestHarness {
    primary_plugin: PluginId,
    builder: TestAppBuilder,
}

impl PluginTestHarness {
    /// Start a harness for the plugin under test.
    pub fn new<I, P>(plugin_id: I, plugin: P) -> Self
    where
        I: Into<PluginId>,
        P: Plugin,
    {
        Self {
            primary_plugin: plugin_id.into(),
            builder: TestApp::builder().register_plugin(plugin),
        }
    }

    /// Add a dependency or companion plugin to the test application.
    pub fn register_plugin<P>(mut self, plugin: P) -> Self
    where
        P: Plugin,
    {
        self.builder = self.builder.register_plugin(plugin);
        self
    }

    /// Add multiple dependency or companion plugins of the same concrete type.
    pub fn register_plugins<I, P>(mut self, plugins: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Plugin,
    {
        self.builder = self.builder.register_plugins(plugins);
        self
    }

    /// Load application config that overrides plugin defaults.
    pub fn load_config_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.builder = self.builder.load_config_dir(path);
        self
    }

    /// Apply arbitrary [`TestAppBuilder`] customization for the host side of
    /// the plugin contract.
    pub fn configure<F>(self, configure: F) -> Self
    where
        F: FnOnce(TestAppBuilder) -> TestAppBuilder,
    {
        let Self {
            primary_plugin,
            builder,
        } = self;
        Self {
            primary_plugin,
            builder: configure(builder),
        }
    }

    /// Bootstrap the plugin and return its test-facing metadata and app.
    pub async fn build(self) -> Result<PluginTestApp> {
        let app = self.builder.build().await?;
        let registry = match app.app().plugins() {
            Ok(registry) => registry,
            Err(error) => return Err(shutdown_after_build_error(app, error).await),
        };
        if let Err(error) = validate_primary_plugin(&registry, &self.primary_plugin) {
            return Err(shutdown_after_build_error(app, error).await);
        }

        Ok(PluginTestApp {
            app,
            registry,
            primary_plugin: self.primary_plugin,
        })
    }
}

fn validate_primary_plugin(registry: &PluginRegistry, plugin_id: &PluginId) -> Result<()> {
    registry.plugin(plugin_id).ok_or_else(|| {
        Error::message(format!(
            "plugin test harness expected plugin `{plugin_id}` but it was not registered"
        ))
    })?;
    registry.contributions(plugin_id).ok_or_else(|| {
        Error::message(format!(
            "plugin test harness found no contributions for `{plugin_id}`"
        ))
    })?;
    Ok(())
}

async fn shutdown_after_build_error(app: TestApp, error: Error) -> Error {
    match app.shutdown().await {
        Ok(()) => error,
        Err(shutdown_error) => Error::message(format!(
            "{error}; plugin test app shutdown also failed: {shutdown_error}"
        )),
    }
}

/// A booted plugin test app with metadata for the primary plugin.
pub struct PluginTestApp {
    app: TestApp,
    registry: Arc<PluginRegistry>,
    primary_plugin: PluginId,
}

impl PluginTestApp {
    /// The primary plugin ID supplied to the harness.
    pub fn plugin_id(&self) -> &PluginId {
        &self.primary_plugin
    }

    /// The primary plugin's resolved manifest.
    pub fn manifest(&self) -> &PluginManifest {
        self.registry
            .plugin(&self.primary_plugin)
            .expect("PluginTestHarness validates the primary plugin manifest")
    }

    /// The primary plugin's registration contribution counts.
    pub fn contributions(&self) -> &PluginContributions {
        self.registry
            .contributions(&self.primary_plugin)
            .expect("PluginTestHarness validates the primary plugin contributions")
    }

    /// The complete registry, including dependency plugins in resolved order.
    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    /// Access the underlying general-purpose test app.
    pub fn test_app(&self) -> &TestApp {
        &self.app
    }

    /// Access the booted application context for service and manager assertions.
    pub fn app(&self) -> &AppContext {
        self.app.app()
    }

    /// Resolve a service contributed by the plugin.
    pub fn resolve<T>(&self) -> Result<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.app.app().resolve::<T>()
    }

    /// Send requests directly through plugin-contributed HTTP routes.
    pub fn client(&self) -> TestClient {
        self.app.client()
    }

    /// Discard the plugin metadata view and keep the general-purpose test app.
    pub fn into_test_app(self) -> TestApp {
        self.app
    }

    /// Run plugin shutdown hooks and stop framework-managed background work.
    pub async fn shutdown(self) -> Result<()> {
        self.app.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::routing::get;
    use semver::{Version, VersionReq};
    use tempfile::tempdir;

    use super::PluginTestHarness;
    use crate::foundation::{AppContext, Result, ServiceProvider, ServiceRegistrar};
    use crate::plugin::{Plugin, PluginDependency, PluginManifest, PluginRegistrar};
    use crate::support::PluginId;

    const BASE_PLUGIN: PluginId = PluginId::new("testing.plugin.base");
    const PRIMARY_PLUGIN: PluginId = PluginId::new("testing.plugin.primary");

    #[derive(Clone)]
    struct HarnessPlugin {
        shutdown: Arc<AtomicBool>,
    }

    struct HarnessProvider;

    struct HostProvider;

    #[derive(Clone)]
    struct HarnessService(String);

    #[derive(Clone)]
    struct HostService(String);

    #[async_trait]
    impl ServiceProvider for HarnessProvider {
        async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
            let value = registrar
                .config()
                .string("plugin_harness.value")
                .unwrap_or_default();
            registrar.singleton(HarnessService(value))
        }
    }

    #[async_trait]
    impl ServiceProvider for HostProvider {
        async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
            let plugin = registrar.resolve::<HarnessService>()?;
            registrar.singleton(HostService(format!("host:{}", plugin.0)))
        }
    }

    #[async_trait]
    impl Plugin for HarnessPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new(
                PRIMARY_PLUGIN,
                Version::new(1, 0, 0),
                VersionReq::parse("*").unwrap(),
            )
        }

        fn register(&self, registrar: &mut PluginRegistrar) -> Result<()> {
            registrar
                .config_defaults(
                    toml::from_str(
                        r#"
                            [plugin_harness]
                            value = "from-plugin"
                        "#,
                    )
                    .unwrap(),
                )
                .register_provider(HarnessProvider)
                .register_routes(|routes| {
                    routes.route("/plugin-harness", get(|| async { "ready" }));
                    Ok(())
                });
            Ok(())
        }

        async fn shutdown(&self, _app: &AppContext) -> Result<()> {
            self.shutdown.store(true, Ordering::Release);
            Ok(())
        }
    }

    struct BasePlugin;

    impl Plugin for BasePlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new(
                BASE_PLUGIN,
                Version::new(1, 2, 0),
                VersionReq::parse("*").unwrap(),
            )
        }

        fn register(&self, _registrar: &mut PluginRegistrar) -> Result<()> {
            Ok(())
        }
    }

    struct DependentPlugin;

    impl Plugin for DependentPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new(
                PRIMARY_PLUGIN,
                Version::new(1, 0, 0),
                VersionReq::parse("*").unwrap(),
            )
            .dependency(PluginDependency::new(
                BASE_PLUGIN,
                VersionReq::parse("^1.2").unwrap(),
            ))
        }

        fn register(&self, _registrar: &mut PluginRegistrar) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn harness_exposes_manifest_contributions_services_routes_and_shutdown() {
        let config_dir = tempdir().unwrap();
        fs::write(
            config_dir.path().join("plugin.toml"),
            r#"
                [plugin_harness]
                value = "from-host"
            "#,
        )
        .unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let app = PluginTestHarness::new(
            PRIMARY_PLUGIN,
            HarnessPlugin {
                shutdown: shutdown.clone(),
            },
        )
        .load_config_dir(config_dir.path())
        .configure(|builder| builder.register_provider(HostProvider))
        .build()
        .await
        .unwrap();

        assert_eq!(app.manifest().id(), &PRIMARY_PLUGIN);
        assert_eq!(app.contributions().provider_count, 1);
        assert_eq!(app.contributions().route_count, 1);
        assert_eq!(app.resolve::<HarnessService>().unwrap().0, "from-host");
        assert_eq!(app.resolve::<HostService>().unwrap().0, "host:from-host");
        assert_eq!(
            app.client()
                .get("/plugin-harness")
                .send()
                .await
                .unwrap()
                .text()
                .unwrap(),
            "ready"
        );

        app.shutdown().await.unwrap();
        assert!(shutdown.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn harness_resolves_registered_plugin_dependencies() {
        let app = PluginTestHarness::new(PRIMARY_PLUGIN, DependentPlugin)
            .register_plugin(BasePlugin)
            .build()
            .await
            .unwrap();

        let ids = app
            .registry()
            .plugins()
            .iter()
            .map(|manifest| manifest.id().clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![BASE_PLUGIN, PRIMARY_PLUGIN]);
        app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn harness_rejects_a_mismatched_primary_id_and_shuts_down() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let error = PluginTestHarness::new(
            PluginId::new("testing.plugin.wrong"),
            HarnessPlugin {
                shutdown: shutdown.clone(),
            },
        )
        .build()
        .await
        .err()
        .expect("mismatched primary ID should fail");

        assert!(error
            .to_string()
            .contains("expected plugin `testing.plugin.wrong`"));
        assert!(shutdown.load(Ordering::Acquire));
    }
}
