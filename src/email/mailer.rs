use std::path::Path;

use crate::foundation::{AppContext, Error, Result};
use crate::storage::StorageManager;

use crate::support::filename::sanitize_filename;

use super::address::{validate_address, EmailAddress};
use super::attachment::{EmailAttachment, ResolvedAttachment};
use super::config::{EmailAttachmentLimits, EmailFromConfig};
use super::driver::OutboundEmail;
use super::message::EmailMessage;

#[derive(Clone)]
pub struct EmailMailer {
    app: AppContext,
    mailer_name: Option<String>,
}

impl EmailMailer {
    pub(crate) fn new(app: AppContext, mailer_name: Option<String>) -> Self {
        Self { app, mailer_name }
    }

    pub fn name(&self) -> Option<&str> {
        self.mailer_name.as_deref()
    }

    /// Send immediately: resolve sender + attachments, then call driver.
    pub async fn send(&self, message: EmailMessage) -> Result<()> {
        let manager = self.app.resolve::<super::EmailManager>()?;
        let outbound = self
            .resolve_message(message, manager.from_address(), manager.attachment_limits())
            .await?;
        let mailer_name = self
            .mailer_name
            .as_deref()
            .unwrap_or(manager.default_mailer_name())
            .to_string();
        let driver = manager.driver(self.mailer_name.as_deref())?;
        super::callback::send_driver(&mailer_name, driver.as_ref(), &outbound).await
    }

    /// Queue for async delivery via Foundry jobs.
    pub async fn queue(&self, message: EmailMessage) -> Result<()> {
        let manager = self.app.resolve::<super::EmailManager>()?;
        let job = super::job::SendQueuedEmailJob {
            mailer_name: self.mailer_name.clone(),
            message,
        };
        let dispatcher = self.app.jobs()?;
        dispatcher
            .dispatch_on(job, manager.queue_id().clone())
            .await
    }

    /// Queue for delayed delivery.
    pub async fn queue_later(&self, message: EmailMessage, run_at_millis: i64) -> Result<()> {
        let manager = self.app.resolve::<super::EmailManager>()?;
        let job = super::job::SendQueuedEmailJob {
            mailer_name: self.mailer_name.clone(),
            message,
        };
        let dispatcher = self.app.jobs()?;
        dispatcher
            .dispatch_later_on(job, run_at_millis, manager.queue_id().clone())
            .await
    }

    /// Resolve sender fallback: message.from > config email.from > error.
    /// Resolve attachments to bytes. Validate message.
    async fn resolve_message(
        &self,
        message: EmailMessage,
        from_config: &EmailFromConfig,
        limits: EmailAttachmentLimits,
    ) -> Result<OutboundEmail> {
        if message.to.is_empty() {
            return Err(Error::message("email message has no recipients"));
        }
        if message.text_body.is_none() && message.html_body.is_none() {
            return Err(Error::message(
                "email message has no body (text or html required)",
            ));
        }
        let from = message
            .from
            .or_else(|| {
                if from_config.address.is_empty() {
                    None
                } else {
                    Some(EmailAddress::with_name(
                        &from_config.address,
                        &from_config.name,
                    ))
                }
            })
            .ok_or_else(|| {
                Error::message("no sender address: set message.from or configure [email.from]")
            })?;

        let reply_to = message.reply_to.map(|addr| vec![addr]).unwrap_or_default();

        let mut attachments = Vec::with_capacity(message.attachments.len());
        let mut total_attachment_bytes = 0u64;
        for att in &message.attachments {
            let attachment = self
                .resolve_attachment(att, limits.max_attachment_bytes)
                .await?;
            total_attachment_bytes = total_attachment_bytes
                .checked_add(attachment.content.len() as u64)
                .ok_or_else(|| Error::message("email attachment payload size overflowed"))?;
            ensure_attachment_size(
                "total email attachments",
                total_attachment_bytes,
                limits.max_total_attachment_bytes,
            )?;
            attachments.push(attachment);
        }

        let outbound = OutboundEmail {
            from,
            to: message.to,
            cc: message.cc,
            bcc: message.bcc,
            reply_to,
            subject: message.subject,
            text_body: message.text_body,
            html_body: message.html_body,
            headers: message.headers,
            attachments,
        };
        validate_outbound_email(&outbound)?;
        Ok(outbound)
    }

    async fn resolve_attachment(
        &self,
        att: &EmailAttachment,
        max_attachment_bytes: u64,
    ) -> Result<ResolvedAttachment> {
        let (content, fallback_name) = match att {
            EmailAttachment::Path { path, .. } => {
                let bytes = read_path_attachment(path, max_attachment_bytes).await?;
                (
                    bytes,
                    Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("attachment")
                        .to_string(),
                )
            }
            EmailAttachment::Storage { disk, path, .. } => {
                let storage = self.app.resolve::<StorageManager>()?;
                let bytes = match disk {
                    Some(d) => storage.disk(d)?.get(path).await?,
                    None => storage.default_disk()?.get(path).await?,
                };
                ensure_attachment_size(path, bytes.len() as u64, max_attachment_bytes)?;
                (
                    bytes,
                    Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("attachment")
                        .to_string(),
                )
            }
        };
        let name = sanitize_filename(att.name().unwrap_or(&fallback_name), "attachment", 255);
        let content_type = match att.content_type() {
            Some(content_type) => validate_content_type(content_type)?.to_string(),
            None => infer_content_type(&name),
        };
        Ok(ResolvedAttachment {
            content,
            name,
            content_type,
        })
    }
}

async fn read_path_attachment(path: &str, max_attachment_bytes: u64) -> Result<Vec<u8>> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::message(format!("failed to read attachment '{}': {e}", path)))?;
    if !metadata.is_file() {
        return Err(Error::message(format!(
            "email attachment '{}' is not a regular file",
            path
        )));
    }
    ensure_attachment_size(path, metadata.len(), max_attachment_bytes)?;

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| Error::message(format!("failed to read attachment '{}': {e}", path)))?;
    ensure_attachment_size(path, bytes.len() as u64, max_attachment_bytes)?;
    Ok(bytes)
}

fn ensure_attachment_size(label: &str, size: u64, limit: u64) -> Result<()> {
    if limit > 0 && size > limit {
        return Err(Error::message(format!(
            "email attachment `{label}` is {size} bytes, exceeding configured limit of {limit} bytes"
        )));
    }
    Ok(())
}

fn validate_outbound_email(email: &OutboundEmail) -> Result<()> {
    validate_address(&email.from, "from")?;
    for addr in &email.to {
        validate_address(addr, "to")?;
    }
    for addr in &email.cc {
        validate_address(addr, "cc")?;
    }
    for addr in &email.bcc {
        validate_address(addr, "bcc")?;
    }
    for addr in &email.reply_to {
        validate_address(addr, "reply-to")?;
    }

    validate_header_value("subject", &email.subject)?;
    for (name, value) in &email.headers {
        validate_header_name(name)?;
        validate_header_value(name, value)?;
    }
    for attachment in &email.attachments {
        validate_header_value("attachment filename", &attachment.name)?;
        validate_content_type(&attachment.content_type)?;
    }

    Ok(())
}

fn validate_header_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::message("email header name cannot be empty"));
    }
    if !name
        .bytes()
        .all(|byte| matches!(byte, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'^' | b'_' | b'`' | b'a'..=b'z' | b'|' | b'~'))
    {
        return Err(Error::message(format!(
            "invalid email header name `{name}`: names must be ASCII header token characters"
        )));
    }
    Ok(())
}

fn validate_header_value(field: &str, value: &str) -> Result<()> {
    if value.chars().any(|ch| matches!(ch, '\r' | '\n')) {
        return Err(Error::message(format!(
            "email {field} cannot contain CR/LF characters"
        )));
    }
    if value.chars().any(|ch| ch.is_control() && ch != '\t') {
        return Err(Error::message(format!(
            "email {field} cannot contain control characters"
        )));
    }
    Ok(())
}

fn validate_content_type(content_type: &str) -> Result<&str> {
    validate_header_value("content type", content_type)?;
    let content_type = content_type.trim();
    if content_type.is_empty() {
        return Err(Error::message("email content type cannot be empty"));
    }
    Ok(content_type)
}

fn infer_content_type(name: &str) -> String {
    match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("csv") => "text/csv",
        Some("txt") => "text/plain",
        Some("html") => "text/html",
        Some("json") => "application/json",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{AppContext, Container};
    use crate::validation::RuleRegistry;
    use std::collections::BTreeMap;

    fn outbound_email() -> OutboundEmail {
        OutboundEmail {
            from: EmailAddress::with_name("sender@example.com", "Sender"),
            to: vec![EmailAddress::new("recipient@example.com")],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: "Hello".to_string(),
            text_body: Some("Body".to_string()),
            html_body: None,
            headers: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn infer_content_type_known_extensions() {
        assert_eq!(infer_content_type("report.pdf"), "application/pdf");
        assert_eq!(infer_content_type("photo.png"), "image/png");
        assert_eq!(infer_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(infer_content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(infer_content_type("data.csv"), "text/csv");
        assert_eq!(infer_content_type("readme.txt"), "text/plain");
        assert_eq!(infer_content_type("data.json"), "application/json");
        assert_eq!(infer_content_type("archive.zip"), "application/zip");
    }

    #[test]
    fn infer_content_type_unknown_extension() {
        assert_eq!(infer_content_type("file.xyz"), "application/octet-stream");
        assert_eq!(infer_content_type("file"), "application/octet-stream");
    }

    #[test]
    fn outbound_email_validation_rejects_header_injection() {
        let mut email = outbound_email();
        email.subject = "Hello\r\nBcc: victim@example.com".to_string();
        assert!(validate_outbound_email(&email)
            .unwrap_err()
            .to_string()
            .contains("subject"));

        let mut email = outbound_email();
        email
            .headers
            .insert("X-Bad\r\nHeader".to_string(), "value".to_string());
        assert!(validate_outbound_email(&email)
            .unwrap_err()
            .to_string()
            .contains("invalid email header name"));

        let mut email = outbound_email();
        email
            .headers
            .insert("X-Custom".to_string(), "ok\r\nbad".to_string());
        assert!(validate_outbound_email(&email)
            .unwrap_err()
            .to_string()
            .contains("X-Custom"));
    }

    #[test]
    fn outbound_email_validation_rejects_invalid_addresses() {
        let mut email = outbound_email();
        email.to = vec![EmailAddress::new("bad\r\nto@example.com")];

        assert!(validate_outbound_email(&email)
            .unwrap_err()
            .to_string()
            .contains("email to address"));
    }

    #[test]
    fn attachment_names_are_sanitized_and_custom_content_type_is_respected() {
        let path = std::env::temp_dir().join(format!(
            "foundry-email-attachment-{}.pdf",
            std::process::id()
        ));
        std::fs::write(&path, b"pdf").unwrap();
        let attachment = EmailAttachment::from_path(path.to_string_lossy().to_string())
            .with_name("../unsafe\r\nname.PDF")
            .with_content_type("application/custom");

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let resolved = runtime.block_on(async {
            let app = AppContext::new(
                Container::new(),
                crate::config::ConfigRepository::empty(),
                RuleRegistry::new(),
            )
            .unwrap();
            EmailMailer::new(app, None)
                .resolve_attachment(&attachment, 0)
                .await
        });

        let resolved = resolved.unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(resolved.name, "unsafename.PDF");
        assert_eq!(resolved.content_type, "application/custom");
    }

    #[tokio::test]
    async fn path_attachment_rejects_files_above_single_attachment_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.pdf");
        std::fs::write(&path, vec![1u8; 8]).unwrap();
        let attachment = EmailAttachment::from_path(path.to_string_lossy().to_string());
        let app = AppContext::new(
            Container::new(),
            crate::config::ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();

        let error = EmailMailer::new(app, None)
            .resolve_attachment(&attachment, 4)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("exceeding configured limit"));
    }

    #[tokio::test]
    async fn total_attachment_limit_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        std::fs::write(&first, b"one").unwrap();
        std::fs::write(&second, b"two").unwrap();
        let message = EmailMessage::new("Hello")
            .to("user@example.com")
            .text_body("Body")
            .attach(EmailAttachment::from_path(
                first.to_string_lossy().to_string(),
            ))
            .attach(EmailAttachment::from_path(
                second.to_string_lossy().to_string(),
            ));
        let app = AppContext::new(
            Container::new(),
            crate::config::ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();

        let error = EmailMailer::new(app, None)
            .resolve_message(
                message,
                &EmailFromConfig {
                    address: "sender@example.com".to_string(),
                    name: "Sender".to_string(),
                },
                EmailAttachmentLimits {
                    max_attachment_bytes: 0,
                    max_total_attachment_bytes: 4,
                },
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("total email attachments"));
    }
}
