use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use lettre::message::header::ContentDisposition;
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::foundation::{Error, Result};

use super::address::EmailAddress;
use super::config::{ResolvedSmtpConfig, SmtpEncryption};
use super::driver::{EmailDriver, OutboundEmail};

pub struct SmtpEmailDriver {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpEmailDriver {
    pub fn from_config(config: &ResolvedSmtpConfig) -> Result<Self> {
        let creds = Credentials::new(config.username.clone(), config.password.clone());
        let mut builder = match config.encryption {
            SmtpEncryption::StartTls | SmtpEncryption::None => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                    .map_err(|e| Error::message(format!("SMTP transport error: {e}")))?
            }
            SmtpEncryption::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|e| Error::message(format!("SMTP transport error: {e}")))?,
        };
        builder = builder
            .port(config.port)
            .credentials(creds)
            .timeout(Some(Duration::from_secs(config.timeout_secs)));
        Ok(Self {
            transport: builder.build(),
        })
    }
}

#[async_trait]
impl EmailDriver for SmtpEmailDriver {
    async fn send(&self, message: &OutboundEmail) -> Result<()> {
        let lettre_msg = build_lettre_message(message)?;
        self.transport
            .send(lettre_msg)
            .await
            .map_err(|e| Error::message(format!("SMTP send failed: {e}")))?;
        Ok(())
    }
}

fn build_lettre_message(email: &OutboundEmail) -> Result<Message> {
    let from_mailbox = to_mailbox(&email.from)?;
    let builder = Message::builder()
        .from(from_mailbox)
        .date_now()
        .subject(&email.subject);

    let mut builder = builder;
    for addr in &email.reply_to {
        builder = builder.reply_to(to_mailbox(addr)?);
    }
    for addr in &email.cc {
        builder = builder.cc(to_mailbox(addr)?);
    }
    for addr in &email.bcc {
        builder = builder.bcc(to_mailbox(addr)?);
    }
    for addr in &email.to {
        builder = builder.to(to_mailbox(addr)?);
    }

    // Build MIME body
    let message = match (
        &email.text_body,
        &email.html_body,
        email.attachments.is_empty(),
    ) {
        (Some(text), None, true) => builder
            .body(text.clone())
            .map_err(|e| Error::message(format!("email build error: {e}")))?,
        (None, Some(html), true) => builder
            .body(html.clone())
            .map_err(|e| Error::message(format!("email build error: {e}")))?,
        (Some(text), Some(html), true) => builder
            .multipart(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(text.clone()))
                    .singlepart(SinglePart::html(html.clone())),
            )
            .map_err(|e| Error::message(format!("email build error: {e}")))?,
        _ => {
            // Has attachments (or no body at all — which shouldn't happen after validation)
            let mut alternative = MultiPart::alternative().build();
            if let Some(text) = &email.text_body {
                alternative = alternative.singlepart(SinglePart::plain(text.clone()));
            }
            if let Some(html) = &email.html_body {
                alternative = alternative.singlepart(SinglePart::html(html.clone()));
            }

            let mut mixed = MultiPart::mixed().multipart(alternative);
            for att in &email.attachments {
                let ct = lettre::message::header::ContentType::parse(&att.content_type)
                    .or_else(|_| {
                        lettre::message::header::ContentType::parse("application/octet-stream")
                    })
                    .map_err(|e| Error::message(format!("email content type error: {e}")))?;
                mixed = mixed.singlepart(
                    SinglePart::builder()
                        .header(ct)
                        .header(ContentDisposition::attachment(&att.name))
                        .body(att.content.clone()),
                );
            }
            builder
                .multipart(mixed)
                .map_err(|e| Error::message(format!("email build error: {e}")))?
        }
    };

    Ok(message)
}

fn to_mailbox(addr: &EmailAddress) -> Result<Mailbox> {
    let email = lettre::Address::from_str(addr.address())
        .map_err(|e| Error::message(format!("invalid email address '{}': {e}", addr.address())))?;
    Ok(match addr.name() {
        Some(name) => Mailbox::new(Some(name.to_string()), email),
        None => Mailbox::new(None, email),
    })
}
