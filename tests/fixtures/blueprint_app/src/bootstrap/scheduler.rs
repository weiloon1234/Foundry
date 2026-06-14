use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    super::app::builder().register_schedule(app::schedules::register)
}
