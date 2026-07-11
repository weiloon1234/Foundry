use foundry::prelude::PluginTestHarness;
use foundry_fixture_plugin_base::{
    FixtureBasePlugin, FixtureBaseService, BASE_COMMAND, BASE_PLUGIN_ID,
};
use foundry_fixture_plugin_dep::{FixtureDependentService, DEPENDENT_PLUGIN_ID};
use foundry_plugin_fixture::app::providers::AppReady;
use foundry_plugin_fixture::bootstrap;

#[tokio::test]
async fn plugin_author_template_tests_one_plugin_in_isolation() {
    let app = PluginTestHarness::new(BASE_PLUGIN_ID, FixtureBasePlugin)
        .build()
        .await
        .unwrap();

    assert_eq!(app.manifest().id(), &BASE_PLUGIN_ID);
    assert_eq!(app.contributions().provider_count, 1);
    assert_eq!(app.contributions().command_count, 1);
    assert_eq!(
        app.resolve::<FixtureBaseService>().unwrap().0,
        "base-plugin"
    );

    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn consumer_app_builds_with_compile_time_plugins() {
    let cli = bootstrap::app::builder().build_cli_kernel().await.unwrap();
    cli.run_with_args(["foundry-plugin-fixture", BASE_COMMAND.as_str()])
        .await
        .unwrap();

    let http = bootstrap::app::builder().build_http_kernel().await.unwrap();
    assert_eq!(
        http.app().resolve::<FixtureDependentService>().unwrap().0,
        "dep:from-app"
    );
    assert_eq!(http.app().resolve::<AppReady>().unwrap().0, "dep:from-app");

    let plugin_ids = http
        .app()
        .plugins()
        .unwrap()
        .plugins()
        .iter()
        .map(|plugin| plugin.id().clone())
        .collect::<Vec<_>>();
    assert_eq!(plugin_ids, vec![BASE_PLUGIN_ID, DEPENDENT_PLUGIN_ID]);
}
