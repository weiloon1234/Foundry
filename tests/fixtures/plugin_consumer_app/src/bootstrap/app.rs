use foundry::prelude::*;
use foundry_fixture_plugin_base::FixtureBasePlugin;
use foundry_fixture_plugin_dep::FixtureDependentPlugin;

use crate::app::providers::AppProvider;

pub fn builder() -> AppBuilder {
    App::builder()
        .load_config_dir("config")
        .register_plugin(FixtureBasePlugin)
        .register_plugin(FixtureDependentPlugin)
        .register_provider(AppProvider)
}
