use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder().register_commands(app::commands::register)
}
