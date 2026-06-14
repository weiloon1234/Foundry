use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::foundation::Result;

use super::address::EmailAddress;
use super::attachment::ResolvedAttachment;

/// Fully resolved outbound email ready for driver consumption.
/// `from` is guaranteed populated (resolved from message or config).
#[derive(Clone, Debug)]
pub struct OutboundEmail {
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub subject: String,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub attachments: Vec<ResolvedAttachment>,
}

/// Transport driver trait for email delivery.
#[async_trait]
pub trait EmailDriver: Send + Sync + 'static {
    async fn send(&self, message: &OutboundEmail) -> Result<()>;
}
