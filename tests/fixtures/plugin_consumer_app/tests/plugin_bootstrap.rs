use foundry_fixture_plugin_base::{BASE_COMMAND, BASE_PLUGIN_ID};
use foundry_fixture_plugin_dep::{FixtureDependentService, DEPENDENT_PLUGIN_ID};
use foundry_plugin_fixture::app::providers::AppReady;
use foundry_plugin_fixture::bootstrap;

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
