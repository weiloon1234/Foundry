use async_trait::async_trait;

use crate::foundation::Result;

use super::config::ResolvedLogConfig;
use super::driver::{EmailDriver, OutboundEmail};

#[derive(Debug)]
pub struct LogEmailDriver {
    target: String,
}

impl LogEmailDriver {
    pub fn from_config(config: &ResolvedLogConfig) -> Self {
        Self {
            target: config.target.clone(),
        }
    }
}

#[async_trait]
impl EmailDriver for LogEmailDriver {
    async fn send(&self, message: &OutboundEmail) -> Result<()> {
        tracing::info!(
            target: "foundry::email",
            mailer_target = %self.target,
            from = %message.from,
            to = ?message.to.iter().map(|a| a.address()).collect::<Vec<_>>(),
            cc = ?message.cc.iter().map(|a| a.address()).collect::<Vec<_>>(),
            subject = %message.subject,
            has_text = message.text_body.is_some(),
            has_html = message.html_body.is_some(),
            attachments = message.attachments.len(),
            "Email sent (log driver)"
        );
        if let Some(ref text) = message.text_body {
            tracing::debug!(target: "foundry::email", "Text body:\n{}", text);
        }
        if let Some(ref html) = message.html_body {
            tracing::debug!(target: "foundry::email", "HTML body:\n{}", html);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::email::address::EmailAddress;
    use crate::email::attachment::ResolvedAttachment;

    fn make_outbound_email() -> OutboundEmail {
        OutboundEmail {
            from: EmailAddress::new("sender@example.com"),
            to: vec![EmailAddress::new("recipient@example.com")],
            cc: vec![],
            bcc: vec![],
            reply_to: vec![],
            subject: "Test Subject".to_string(),
            text_body: Some("Hello world".to_string()),
            html_body: None,
            headers: BTreeMap::new(),
            attachments: vec![],
        }
    }

    #[tokio::test]
    async fn log_driver_send_returns_ok() {
        let config = ResolvedLogConfig {
            target: "test.email".to_string(),
        };
        let driver = LogEmailDriver::from_config(&config);
        let message = make_outbound_email();
        let result = driver.send(&message).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn log_driver_send_with_html_and_attachments() {
        let config = ResolvedLogConfig {
            target: "test.email".to_string(),
        };
        let driver = LogEmailDriver::from_config(&config);
        let mut message = make_outbound_email();
        message.html_body = Some("<p>Hello</p>".to_string());
        message.attachments.push(ResolvedAttachment {
            content: vec![1, 2, 3],
            name: "test.pdf".to_string(),
            content_type: "application/pdf".to_string(),
        });
        let result = driver.send(&message).await;
        assert!(result.is_ok());
    }
}
