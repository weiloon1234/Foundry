use std::net::SocketAddr;
use std::path::PathBuf;

use tokio::net::TcpListener;

use crate::config::ServerConfig;
use crate::foundation::{AppContext, Error, Result};
use crate::http::middleware::MiddlewareConfig;
use crate::http::PreparedHttpRoutes;
use crate::logging::ObservabilityOptions;

pub struct HttpKernel {
    app: AppContext,
    routes: PreparedHttpRoutes,
    middlewares: Vec<MiddlewareConfig>,
    observability: Option<ObservabilityOptions>,
    spa_dir: Option<PathBuf>,
}

impl HttpKernel {
    pub(crate) fn new(
        app: AppContext,
        routes: PreparedHttpRoutes,
        middlewares: Vec<MiddlewareConfig>,
        observability: Option<ObservabilityOptions>,
        spa_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            app,
            routes,
            middlewares,
            observability,
            spa_dir,
        }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn build_router(&self) -> Result<axum::Router> {
        let mut registrar = self.routes.registrar();
        if let Some(options) = &self.observability {
            let obs_config = self.app.config().observability()?;
            if obs_config.enabled {
                if options.is_public() && self.app.config().app()?.environment.is_production_like()
                {
                    tracing::warn!(
                        base_path = %obs_config.base_path,
                        "foundry: public observability diagnostics are enabled in a production-like environment"
                    );
                }
                // Collect documented routes and publish OpenAPI spec
                let documented = registrar.collect_documented_routes();
                if !documented.is_empty() {
                    crate::logging::set_openapi_spec("API", "1.0.0", &documented);
                }
                crate::logging::register_openapi_route(&mut registrar, &obs_config, options)?;
                crate::logging::register_observability_routes(
                    &mut registrar,
                    &obs_config,
                    options,
                )?;
            }
        }

        let http_config = self.app.config().http()?;
        let mut middlewares = crate::http::middleware::configured_global_middlewares(
            &http_config,
            &self.middlewares,
        )?;
        middlewares.extend(self.middlewares.clone());

        let mut router = registrar.into_router_with_middlewares(self.app.clone(), middlewares)?;

        if let Some(ref spa_dir) = self.spa_dir {
            router = router.fallback_service(crate::http::spa::spa_fallback(spa_dir.clone()));
        }

        Ok(router)
    }

    pub async fn bind(self) -> Result<BoundHttpServer> {
        let server = self.app.config().server()?;
        let listener = bind_listener(&server).await?;
        let local_addr = listener.local_addr().map_err(Error::other)?;
        let router = self.build_router()?;

        Ok(BoundHttpServer {
            listener,
            router,
            local_addr,
        })
    }

    pub async fn serve(self) -> Result<()> {
        self.bind().await?.serve().await
    }
}

pub struct BoundHttpServer {
    listener: TcpListener,
    router: axum::Router,
    local_addr: SocketAddr,
}

impl BoundHttpServer {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn serve(self) -> Result<()> {
        axum::serve(
            self.listener,
            self.router
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(super::shutdown::shutdown_signal())
        .await
        .map_err(Error::other)
    }
}

async fn bind_listener(server: &ServerConfig) -> Result<TcpListener> {
    TcpListener::bind((server.host.as_str(), server.port))
        .await
        .map_err(Error::other)
}
