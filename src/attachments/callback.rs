use std::future::Future;

use crate::foundation::{Error, Result};
use crate::logging::{catch_async_panic, catch_sync_panic, panic_payload_message};

pub(crate) fn run_attachment_sync<T, F>(subject: &str, run: F) -> Result<T>
where
    F: FnOnce() -> T,
{
    catch_sync_panic(run).map_err(|panic| attachment_panic_error(subject, panic))
}

pub(crate) async fn run_attachment_callback<F, Fut>(subject: &str, run: F) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    match catch_async_panic(run).await {
        Ok(result) => result,
        Err(panic) => Err(attachment_panic_error(subject, panic)),
    }
}

fn attachment_panic_error(subject: &str, panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.attachments",
        subject = subject,
        panic = %message,
        "attachment callback panicked"
    );
    Error::message(format!("attachment {subject} panicked: {message}"))
}
