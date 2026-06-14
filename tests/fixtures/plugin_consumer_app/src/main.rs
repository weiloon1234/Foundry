use foundry_plugin_fixture::bootstrap;

#[tokio::main]
async fn main() -> foundry::Result<()> {
    let _http = bootstrap::app::builder().build_http_kernel().await?;
    let _cli = bootstrap::app::builder().build_cli_kernel().await?;
    Ok(())
}
