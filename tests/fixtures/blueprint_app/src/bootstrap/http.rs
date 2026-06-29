use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder()
        .register_routes(app::portals::router)
        .enable_observability_with(
            ObservabilityOptions::new()
                .guard(app::ids::AuthGuard::Api)
                .permission(app::ids::Ability::DashboardView),
        )
}
