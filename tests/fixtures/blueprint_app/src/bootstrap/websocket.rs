use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder().register_websocket_routes(app::realtime::register)
}
