use serde::{Deserialize, Serialize};

use crate::foundation::Result;
use crate::jobs::{Job, JobContext};
use crate::support::JobId;

use super::mailer::EmailMailer;
use super::message::EmailMessage;

#[derive(Debug, Serialize, Deserialize)]
pub struct SendQueuedEmailJob {
    pub mailer_name: Option<String>,
    pub message: EmailMessage,
}

#[async_trait::async_trait]
impl Job for SendQueuedEmailJob {
    const ID: JobId = JobId::new("foundry.send_queued_email");

    async fn handle(&self, context: JobContext) -> Result<()> {
        let mailer = EmailMailer::new(context.app().clone(), self.mailer_name.clone());
        mailer.send(self.message.clone()).await
    }

    fn max_retries(&self) -> Option<u32> {
        Some(3)
    }
}
