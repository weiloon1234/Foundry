use std::future::Future;

use crate::foundation::{AppContext, Result};
use crate::jobs::Worker;

pub struct WorkerKernel {
    worker: Worker,
}

impl WorkerKernel {
    pub fn new(app: AppContext) -> Result<Self> {
        Ok(Self {
            worker: Worker::from_app(app)?,
        })
    }

    pub fn app(&self) -> &AppContext {
        self.worker.app()
    }

    pub async fn run(self) -> Result<()> {
        self.run_until(super::shutdown::shutdown_signal()).await
    }

    async fn run_until<S>(self, shutdown: S) -> Result<()>
    where
        S: Future<Output = ()> + Send + 'static,
    {
        self.worker.run_until(shutdown).await
    }

    pub async fn run_once(&self) -> Result<bool> {
        self.worker.run_once().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[tokio::test]
    async fn worker_run_exits_when_shutdown_future_completes_while_idle() {
        let kernel = crate::App::builder().build_worker_kernel().await.unwrap();

        tokio::time::timeout(
            Duration::from_millis(50),
            kernel.run_until(async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
