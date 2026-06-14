use tokio::signal;

/// Wait for SIGTERM or SIGINT (Ctrl+C) to initiate graceful shutdown.
pub(crate) async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = signal::ctrl_c().await {
            tracing::warn!(
                error = %error,
                "foundry: failed to listen for SIGINT shutdown signal"
            );
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "foundry: failed to listen for SIGTERM shutdown signal"
                );
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("foundry: received SIGINT, shutting down gracefully"); }
        _ = terminate => { tracing::info!("foundry: received SIGTERM, shutting down gracefully"); }
    }
}
