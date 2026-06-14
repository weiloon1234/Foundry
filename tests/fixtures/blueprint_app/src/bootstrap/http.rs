use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder()
        .register_routes(app::portals::router)
        .enable_observability()
}
