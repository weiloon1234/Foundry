use foundry::prelude::*;
use foundry_blueprint_fixture::bootstrap;

async fn compile_contract() -> Result<()> {
    let _http = bootstrap::http::builder().build_http_kernel().await?;
    let _cli = bootstrap::cli::builder().build_cli_kernel().await?;
    let _scheduler = bootstrap::scheduler::builder().build_scheduler_kernel().await?;
    let _websocket = bootstrap::websocket::builder().build_websocket_kernel().await?;
    Ok(())
}

fn main() -> Result<()> {
    let _ = compile_contract;
    let _ = bootstrap::app::builder();
    Ok(())
}
