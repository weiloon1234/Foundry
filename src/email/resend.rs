use async_trait::async_trait;

use crate::foundation::{Error, Result};

use super::address::format_address;
use super::config::ResolvedResendConfig;
use super::driver::{EmailDriver, OutboundEmail};

pub struct ResendEmailDriver {
    client: reqwest::Client,
    api_key: String,
}

impl ResendEmailDriver {
    pub fn from_config(config: &ResolvedResendConfig) -> Self {
        Self {
            client: super::http::client("Resend", config.timeout_secs),
            api_key: config.api_key.clone(),
        }
    }
}

#[async_trait]
impl EmailDriver for ResendEmailDriver {
    async fn send(&self, message: &OutboundEmail) -> Result<()> {
        let mut body = serde_json::Map::new();
        body.insert(
            "from".into(),
            serde_json::Value::String(format_address(&message.from)),
        );
        body.insert(
            "to".into(),
            serde_json::Value::Array(
                message
                    .to
                    .iter()
                    .map(|a| serde_json::Value::String(a.address().to_string()))
                    .collect(),
            ),
        );
        if !message.cc.is_empty() {
            body.insert(
                "cc".into(),
                serde_json::Value::Array(
                    message
                        .cc
                        .iter()
                        .map(|a| serde_json::Value::String(a.address().to_string()))
                        .collect(),
                ),
            );
        }
        if !message.bcc.is_empty() {
            body.insert(
                "bcc".into(),
                serde_json::Value::Array(
                    message
                        .bcc
                        .iter()
                        .map(|a| serde_json::Value::String(a.address().to_string()))
                        .collect(),
                ),
            );
        }
        if let Some(ref text) = message.text_body {
            body.insert("text".into(), serde_json::Value::String(text.clone()));
        }
        if let Some(ref html) = message.html_body {
            body.insert("html".into(), serde_json::Value::String(html.clone()));
        }
        body.insert(
            "subject".into(),
            serde_json::Value::String(message.subject.clone()),
        );
        if !message.reply_to.is_empty() {
            body.insert(
                "reply_to".into(),
                serde_json::Value::String(message.reply_to[0].address().to_string()),
            );
        }
        if !message.headers.is_empty() {
            let headers: serde_json::Map<String, serde_json::Value> = message
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            body.insert("headers".into(), serde_json::Value::Object(headers));
        }
        if !message.attachments.is_empty() {
            let attachments: Vec<serde_json::Value> = message
                .attachments
                .iter()
                .map(|att| {
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "filename".into(),
                        serde_json::Value::String(att.name.clone()),
                    );
                    map.insert(
                        "content".into(),
                        serde_json::Value::String(base64_encode(&att.content)),
                    );
                    if !att.content_type.is_empty() {
                        // Resend doesn't use content_type field directly, but we include it
                    }
                    serde_json::Value::Object(map)
                })
                .collect();
            body.insert("attachments".into(), serde_json::Value::Array(attachments));
        }

        let response = self
            .client
            .post("https://api.resend.com/emails")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("Resend request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(super::http::provider_error("Resend", status, response).await);
        }

        Ok(())
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::EmailAddress;

    #[test]
    fn format_address_with_name() {
        let addr = EmailAddress::with_name("test@example.com", "Test User");
        assert_eq!(format_address(&addr), "Test User <test@example.com>");
    }

    #[test]
    fn format_address_without_name() {
        let addr = EmailAddress::new("test@example.com");
        assert_eq!(format_address(&addr), "test@example.com");
    }
}
