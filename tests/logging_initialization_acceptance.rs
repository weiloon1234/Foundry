use foundry::prelude::*;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::test]
async fn subscriber_conflicts_fail_bootstrap_unless_host_ownership_is_explicit() {
    tracing_subscriber::registry().try_init().unwrap();

    let error = App::builder()
        .build_cli_kernel()
        .await
        .err()
        .expect("implicit subscriber conflict must fail bootstrap");
    assert!(error
        .to_string()
        .contains("failed to install Foundry tracing subscriber"));
    assert!(error
        .to_string()
        .contains("use_external_tracing_subscriber"));

    let kernel = App::builder()
        .use_external_tracing_subscriber()
        .build_cli_kernel()
        .await
        .expect("consumer-hosted subscriber opt-out should bootstrap");
    kernel.app().shutdown().await.unwrap();
}
