use foundry_blueprint_fixture::{app, bootstrap};

#[tokio::test]
async fn split_bootstrap_builds_public_kernels() {
    let log = app::providers::shared_log();
    log.lock().unwrap().clear();

    let http = bootstrap::http::builder().build_http_kernel().await.unwrap();
    let websocket = bootstrap::websocket::builder()
        .build_websocket_kernel()
        .await
        .unwrap();
    let cli = bootstrap::cli::builder().build_cli_kernel().await.unwrap();
    let scheduler = bootstrap::scheduler::builder()
        .build_scheduler_kernel()
        .await
        .unwrap();

    cli.run_with_args(["foundry-blueprint-fixture", "ping"])
        .await
        .unwrap();

    let now = foundry::DateTime::parse("2026-04-09T12:00:00Z").unwrap();
    let executed = scheduler.run_once_at(now).await.unwrap();

    assert_eq!(executed, vec![app::ids::HEARTBEAT_SCHEDULE]);

    // The scheduler handler runs in a spawned task — yield to let it complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(http.app().config().server().unwrap().port, 0);
    assert_eq!(websocket.app().config().websocket().unwrap().path, "/ws");

    let entries = log.lock().unwrap().clone();
    assert!(entries.iter().any(|entry| entry == "provider:boot"));
    assert!(entries.iter().any(|entry| entry == "command:ping"));
    assert!(entries.iter().any(|entry| entry == "schedule:heartbeat"));
}
