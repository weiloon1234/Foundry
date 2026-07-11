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
            SmtpEncryption::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                    .map_err(|e| Error::message(format!("SMTP transport error: {e}")))?
            }
            SmtpEncryption::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|e| Error::message(format!("SMTP transport error: {e}")))?,
            SmtpEncryption::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn none_encryption_sends_over_plaintext_without_starttls() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let transcript = Arc::new(Mutex::new(Vec::new()));
        let server_transcript = transcript.clone();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream.write_all(b"220 localhost ESMTP\r\n").unwrap();
            stream.flush().unwrap();

            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut in_data = false;
            loop {
                let mut line = String::new();
                let Ok(read) = reader.read_line(&mut line) else {
                    break;
                };
                if read == 0 {
                    break;
                }

                let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
                server_transcript.lock().unwrap().push(trimmed.clone());

                if in_data {
                    if trimmed == "." {
                        in_data = false;
                        stream.write_all(b"250 queued\r\n").unwrap();
                        stream.flush().unwrap();
                    }
                    continue;
                }

                let command = trimmed
                    .split_ascii_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                match command.as_str() {
                    "EHLO" => {
                        stream
                            .write_all(b"250-localhost\r\n250 AUTH PLAIN LOGIN\r\n")
                            .unwrap();
                    }
                    "HELO" | "MAIL" | "RCPT" => {
                        stream.write_all(b"250 localhost\r\n").unwrap();
                    }
                    "AUTH" => {
                        stream.write_all(b"235 authenticated\r\n").unwrap();
                    }
                    "DATA" => {
                        stream.write_all(b"354 end with <CRLF>.<CRLF>\r\n").unwrap();
                        in_data = true;
                    }
                    "QUIT" => {
                        stream.write_all(b"221 bye\r\n").unwrap();
                        stream.flush().unwrap();
                        break;
                    }
                    _ => {
                        stream.write_all(b"250 ok\r\n").unwrap();
                    }
                }
                stream.flush().unwrap();
            }
        });

        let driver = SmtpEmailDriver::from_config(&ResolvedSmtpConfig {
            host: "127.0.0.1".to_string(),
            port,
            username: "user".to_string(),
            password: "password".to_string(),
            encryption: SmtpEncryption::None,
            timeout_secs: 5,
        })
        .unwrap();
        let message = OutboundEmail {
            from: EmailAddress::new("sender@example.com"),
            to: vec![EmailAddress::new("recipient@example.com")],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: "Plaintext SMTP".to_string(),
            text_body: Some("hello".to_string()),
            html_body: None,
            headers: BTreeMap::new(),
            attachments: Vec::new(),
        };

        driver.send(&message).await.unwrap();
        drop(driver);
        server.join().unwrap();

        let transcript = transcript.lock().unwrap();
        assert!(transcript.iter().any(|line| line.starts_with("MAIL FROM:")));
        assert!(!transcript.iter().any(|line| line == "STARTTLS"));
    }
}
