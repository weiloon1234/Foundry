use async_trait::async_trait;

use crate::foundation::{Error, Result};

use super::address::format_address;
use super::config::ResolvedMailgunConfig;
use super::driver::{EmailDriver, OutboundEmail};

pub struct MailgunEmailDriver {
    client: reqwest::Client,
    config: ResolvedMailgunConfig,
}

impl MailgunEmailDriver {
    pub fn from_config(config: &ResolvedMailgunConfig) -> Self {
        Self {
            client: super::http::client("Mailgun", config.timeout_secs),
            config: config.clone(),
        }
    }
}

#[async_trait]
impl EmailDriver for MailgunEmailDriver {
    async fn send(&self, message: &OutboundEmail) -> Result<()> {
        let mut form: Vec<(String, String)> = vec![
            ("from".into(), format_address(&message.from)),
            ("subject".into(), message.subject.clone()),
        ];

        // Add recipients
        for addr in &message.to {
            form.push(("to".into(), addr.address().to_string()));
        }
        for addr in &message.cc {
            form.push(("cc".into(), addr.address().to_string()));
        }
        for addr in &message.bcc {
            form.push(("bcc".into(), addr.address().to_string()));
        }

        if let Some(ref text) = message.text_body {
            form.push(("text".into(), text.clone()));
        }
        if let Some(ref html) = message.html_body {
            form.push(("html".into(), html.clone()));
        }
        if !message.reply_to.is_empty() {
            form.push((
                "h:Reply-To".into(),
                message.reply_to[0].address().to_string(),
            ));
        }

        // Custom headers
        for (key, value) in &message.headers {
            form.push((format!("h:{key}"), value.clone()));
        }

        // Always use multipart/form-data (Mailgun supports it for all messages)
        let mut multipart = reqwest::multipart::Form::new();
        for (key, value) in form {
            multipart = multipart.text(key, value);
        }
        for att in &message.attachments {
            let ct = att.content_type.clone();
            let part = match reqwest::multipart::Part::bytes(att.content.clone())
                .file_name(att.name.clone())
                .mime_str(&ct)
            {
                Ok(p) => p,
                Err(_) => {
                    reqwest::multipart::Part::bytes(att.content.clone()).file_name(att.name.clone())
                }
            };
            multipart = multipart.part("attachment", part);
        }

        let response = self
            .client
            .post(self.config.base_url())
            .basic_auth("api", Some(&self.config.api_key))
            .multipart(multipart)
            .send()
            .await
            .map_err(|e| Error::message(format!("Mailgun request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(super::http::provider_error("Mailgun", status, response).await);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::config::MailgunRegion;

    #[test]
    fn mailgun_base_url_us() {
        let config = ResolvedMailgunConfig {
            domain: "mg.example.com".to_string(),
            api_key: "key-xxx".to_string(),
            region: MailgunRegion::Us,
            timeout_secs: 30,
        };
        assert_eq!(
            config.base_url(),
            "https://api.mailgun.net/v3/mg.example.com/messages"
        );
    }

    #[test]
    fn mailgun_base_url_eu() {
        let config = ResolvedMailgunConfig {
            domain: "mg.example.com".to_string(),
            api_key: "key-xxx".to_string(),
            region: MailgunRegion::Eu,
            timeout_secs: 30,
        };
        assert_eq!(
            config.base_url(),
            "https://api.eu.mailgun.net/v3/mg.example.com/messages"
        );
    }
}
