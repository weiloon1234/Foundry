use crate::foundation::{Error, Result};
use crate::logging::panic_payload_message;

/// Run blocking or CPU-heavy work on Tokio's blocking thread pool.
///
/// This keeps expensive synchronous work from occupying async runtime worker
/// threads while preserving Foundry's unified error surface.
pub async fn run_blocking<T, F>(label: impl Into<String>, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let label = label.into();
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) if error.is_panic() => {
            let message = panic_payload_message(error.into_panic());
            tracing::error!(
                target: "foundry.runtime",
                task = %label,
                panic = %message,
                "blocking task panicked"
            );
            Err(Error::message(format!(
                "{label} blocking task panicked: {message}"
            )))
        }
        Err(error) => Err(Error::message(format!(
            "{label} blocking task failed: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::run_blocking;
    use crate::foundation::Result;

    #[tokio::test]
    async fn run_blocking_returns_successful_result() {
        let value = run_blocking("test.success", || Ok(42)).await.unwrap();

        assert_eq!(value, 42);
    }

    #[tokio::test]
    async fn run_blocking_panic_becomes_framework_error() {
        let error = run_blocking("test.panic", || -> Result<()> {
            panic!("blocking boom");
        })
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "test.panic blocking task panicked: blocking boom"
        );
    }
}
