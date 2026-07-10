use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use foundry::prelude::*;
use serde_json::Value;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        #[derive(Clone, Copy)]
        pub enum AuthGuard {
            Admin,
        }

        impl From<AuthGuard> for GuardId {
            fn from(value: AuthGuard) -> Self {
                match value {
                    AuthGuard::Admin => GuardId::new("admin"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum PolicyKey {
            DeveloperOnly,
        }

        impl From<PolicyKey> for PolicyId {
            fn from(value: PolicyKey) -> Self {
                match value {
                    PolicyKey::DeveloperOnly => PolicyId::new("developer_only"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum RoleKey {
            Developer,
        }

        impl From<RoleKey> for RoleId {
            fn from(value: RoleKey) -> Self {
                match value {
                    RoleKey::Developer => RoleId::new("developer"),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub enum Ability {
            ReportsView,
            ObservabilityView,
        }

        impl From<Ability> for PermissionId {
            fn from(value: Ability) -> Self {
                match value {
                    Ability::ReportsView => PermissionId::new("reports:view"),
                    Ability::ObservabilityView => PermissionId::new("observability:view"),
                }
            }
        }
    }

    pub mod providers {
        use super::*;

        #[derive(Clone)]
        pub struct AppServiceProvider;

        pub struct DeveloperOnlyPolicy;

        #[async_trait]
        impl Policy for DeveloperOnlyPolicy {
            async fn evaluate(&self, actor: &Actor, _app: &AppContext) -> Result<bool> {
                Ok(actor.has_role(ids::RoleKey::Developer))
            }
        }

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.register_guard(
                    ids::AuthGuard::Admin,
                    StaticBearerAuthenticator::new()
                        .token("guest-token", Actor::new("guest-1", ids::AuthGuard::Admin))
                        .token(
                            "ops-token",
                            Actor::new("ops-1", ids::AuthGuard::Admin).with_permissions([
                                ids::Ability::ReportsView,
                                ids::Ability::ObservabilityView,
                            ]),
                        )
                        .token(
                            "developer-token",
                            Actor::new("developer-1", ids::AuthGuard::Admin)
                                .with_roles([ids::RoleKey::Developer])
                                .with_permissions([
                                    ids::Ability::ReportsView,
                                    ids::Ability::ObservabilityView,
                                ]),
                        ),
                )?;
                registrar.register_policy(ids::PolicyKey::DeveloperOnly, DeveloperOnlyPolicy)?;
                Ok(())
            }
        }
    }
}

fn reports_routes(
    counter: Arc<AtomicUsize>,
) -> impl Fn(&mut HttpRegistrar) -> Result<()> + Send + Sync + 'static {
    move |registrar| {
        let counter = counter.clone();
        registrar.route_with_options(
            "/reports",
            get(reports),
            HttpRouteOptions::new()
                .guard(app::ids::AuthGuard::Admin)
                .permission(app::ids::Ability::ReportsView)
                .authorize(move |ctx| {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        let allowed = ctx
                            .app()
                            .authorizer()?
                            .allows_policy(ctx.actor(), app::ids::PolicyKey::DeveloperOnly)
                            .await?;

                        if allowed {
                            Ok(())
                        } else {
                            Err(Error::http(403, "Forbidden by project policy"))
                        }
                    }
                }),
        );
        Ok(())
    }
}

fn always_unauthorized_routes() -> impl Fn(&mut HttpRegistrar) -> Result<()> + Send + Sync + 'static
{
    move |registrar| {
        registrar.route_with_options(
            "/session-check",
            get(session_check),
            HttpRouteOptions::new()
                .guard(app::ids::AuthGuard::Admin)
                .permission(app::ids::Ability::ReportsView)
                .authorize(|_ctx| async {
                    Err(AuthError::unauthorized("Re-authentication required").into())
                }),
        );
        Ok(())
    }
}

fn panicking_authorizer_routes() -> impl Fn(&mut HttpRegistrar) -> Result<()> + Send + Sync + 'static
{
    move |registrar| {
        registrar.route_with_options(
            "/panic-auth",
            get(session_check),
            HttpRouteOptions::new()
                .guard(app::ids::AuthGuard::Admin)
                .permission(app::ids::Ability::ReportsView)
                .authorize(|_ctx| async {
                    if std::hint::black_box(true) {
                        panic!("route auth exploded");
                    }
                    Ok(())
                }),
        );
        Ok(())
    }
}

async fn reports(actor: CurrentActor) -> impl IntoResponse {
    Json(serde_json::json!({
        "actor_id": actor.id,
    }))
}

async fn session_check(actor: CurrentActor) -> impl IntoResponse {
    Json(serde_json::json!({
        "actor_id": actor.id,
    }))
}

async fn public_status() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

fn grouped_typed_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.group_with_options(
        "/typed",
        HttpRouteOptions::new()
            .guard(app::ids::AuthGuard::Admin)
            .permission(app::ids::Ability::ReportsView),
        |routes| {
            routes.get(RouteId::new("typed.reports"), "/reports", reports, |_| {});
            routes.get(
                RouteId::new("typed.public"),
                "/public",
                public_status,
                |route| {
                    route.public();
                },
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[tokio::test]
async fn typed_routes_in_guarded_groups_enforce_defaults_and_allow_explicit_public_routes() {
    let app = TestApp::builder()
        .register_provider(app::providers::AppServiceProvider)
        .register_routes(grouped_typed_routes)
        .build()
        .await
        .unwrap();

    let unauthorized = app.client().get("/typed/reports").send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let forbidden = app
        .client()
        .get("/typed/reports")
        .bearer_auth("guest-token")
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let allowed = app
        .client()
        .get("/typed/reports")
        .bearer_auth("ops-token")
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);

    let public = app.client().get("/typed/public").send().await.unwrap();
    assert_eq!(public.status(), StatusCode::OK);
}

#[tokio::test]
async fn http_route_authorizer_runs_after_permissions_and_can_return_forbidden() {
    let counter = Arc::new(AtomicUsize::new(0));
    let app = TestApp::builder()
        .register_provider(app::providers::AppServiceProvider)
        .register_routes(reports_routes(counter.clone()))
        .build()
        .await
        .unwrap();

    let unauthorized = app.client().get("/reports").send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    let forbidden_by_permission = app
        .client()
        .get("/reports")
        .bearer_auth("guest-token")
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden_by_permission.status(), StatusCode::FORBIDDEN);
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    let forbidden_by_authorizer = app
        .client()
        .get("/reports")
        .bearer_auth("ops-token")
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden_by_authorizer.status(), StatusCode::FORBIDDEN);
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    let allowed = app
        .client()
        .get("/reports")
        .bearer_auth("developer-token")
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    let payload: Value = allowed.json().unwrap();
    assert_eq!(payload["actor_id"], "developer-1");
}

#[tokio::test]
async fn http_route_authorizer_can_return_unauthorized() {
    let app = TestApp::builder()
        .register_provider(app::providers::AppServiceProvider)
        .register_routes(always_unauthorized_routes())
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/session-check")
        .bearer_auth("developer-token")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload: Value = response.json().unwrap();
    assert_eq!(payload["message"], "Re-authentication required");
}

#[tokio::test]
async fn http_route_authorizer_panic_returns_internal_error() {
    let app = TestApp::builder()
        .register_provider(app::providers::AppServiceProvider)
        .register_routes(panicking_authorizer_routes())
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/panic-auth")
        .bearer_auth("developer-token")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let payload: Value = response.json().unwrap();
    assert_eq!(payload["message"], "Internal server error");
}

#[tokio::test]
async fn observability_authorizer_applies_to_all_routes_and_can_hide_with_not_found() {
    const OBSERVABILITY_ROUTES: [&str; 13] = [
        "/_foundry/health",
        "/_foundry/ready",
        "/_foundry/runtime",
        "/_foundry/http/stats",
        "/_foundry/metrics",
        "/_foundry/jobs/stats",
        "/_foundry/jobs/failed",
        "/_foundry/sql",
        "/_foundry/ws/channels",
        "/_foundry/ws/stats",
        "/_foundry/ws/presence/team",
        "/_foundry/ws/history/team",
        "/_foundry/openapi.json",
    ];

    let counter = Arc::new(AtomicUsize::new(0));
    let authorize_counter = counter.clone();
    let app = TestApp::builder()
        .register_provider(app::providers::AppServiceProvider)
        .enable_observability_with(
            ObservabilityOptions::new()
                .guard(app::ids::AuthGuard::Admin)
                .permission(app::ids::Ability::ObservabilityView)
                .authorize(move |ctx| {
                    let counter = authorize_counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        let allowed = ctx
                            .app()
                            .authorizer()?
                            .allows_policy(ctx.actor(), app::ids::PolicyKey::DeveloperOnly)
                            .await?;

                        if allowed {
                            Ok(())
                        } else {
                            Err(Error::not_found("Not found"))
                        }
                    }
                }),
        )
        .build()
        .await
        .unwrap();

    let unauthorized = app.client().get("/_foundry/health").send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    for path in OBSERVABILITY_ROUTES {
        let response = app
            .client()
            .get(path)
            .bearer_auth("guest-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "route {path}");
    }
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    for path in OBSERVABILITY_ROUTES {
        let response = app
            .client()
            .get(path)
            .bearer_auth("ops-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "route {path}");
    }
    assert_eq!(counter.load(Ordering::SeqCst), OBSERVABILITY_ROUTES.len());

    let allowed = app
        .client()
        .get("/_foundry/health")
        .bearer_auth("developer-token")
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
}

#[tokio::test]
async fn observability_authorizer_panic_returns_internal_error() {
    let app = TestApp::builder()
        .register_provider(app::providers::AppServiceProvider)
        .enable_observability_with(
            ObservabilityOptions::new()
                .guard(app::ids::AuthGuard::Admin)
                .permission(app::ids::Ability::ObservabilityView)
                .authorize(|_ctx| async {
                    if std::hint::black_box(true) {
                        panic!("observability auth exploded");
                    }
                    Ok(())
                }),
        )
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .get("/_foundry/health")
        .bearer_auth("developer-token")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let payload: Value = response.json().unwrap();
    assert_eq!(payload["message"], "Internal server error");
}
