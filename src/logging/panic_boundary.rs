use std::any::Any;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};

use futures_util::FutureExt;

pub(crate) type PanicPayload = Box<dyn Any + Send>;

pub(crate) fn catch_sync_panic<T, F>(callback: F) -> Result<T, PanicPayload>
where
    F: FnOnce() -> T,
{
    catch_unwind(AssertUnwindSafe(callback))
}

pub(crate) async fn catch_future_panic<T, Fut>(future: Fut) -> Result<T, PanicPayload>
where
    Fut: Future<Output = T>,
{
    AssertUnwindSafe(future).catch_unwind().await
}

pub(crate) async fn catch_async_panic<T, F, Fut>(callback: F) -> Result<T, PanicPayload>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let future = catch_sync_panic(callback)?;
    catch_future_panic(future).await
}
