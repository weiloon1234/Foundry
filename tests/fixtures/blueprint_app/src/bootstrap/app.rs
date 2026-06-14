use foundry::prelude::*;

use crate::app;

pub fn builder() -> AppBuilder {
    App::builder()
        .load_config_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/config"))
        .register_provider(app::providers::AppServiceProvider)
        .register_validation_rule(app::ids::MOBILE_RULE, app::validation::MobileRule)
}
