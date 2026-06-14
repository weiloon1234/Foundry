use async_trait::async_trait;

use crate::foundation::{Error, Result};

use super::address::format_address;
use super::config::ResolvedPostmarkConfig;
use super::driver::{EmailDriver, OutboundEmail};

pub struct PostmarkEmailDriver {
    client: reqwest::Client,
    server_token: String,
}

impl PostmarkEmailDriver {
    pub fn from_config(config: &ResolvedPostmarkConfig) -> Self {
        Self {
            client: super::http::client("Postmark", config.timeout_secs),
            server_token: config.server_token.clone(),
        }
    }
}

#[async_trait]
impl EmailDriver for PostmarkEmailDriver {
    async fn send(&self, message: &OutboundEmail) -> Result<()> {
        let mut body = serde_json::Map::new();
        body.insert(
            "From".into(),
            serde_json::Value::String(format_address(&message.from)),
        );
        body.insert(
            "To".into(),
            serde_json::Value::String(
                message
                    .to
                    .iter()
                    .map(|a| a.address())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        );
        body.insert(
            "Subject".into(),
            serde_json::Value::String(message.subject.clone()),
        );
        if !message.cc.is_empty() {
            body.insert(
                "Cc".into(),
                serde_json::Value::String(
                    message
                        .cc
                        .iter()
                        .map(|a| a.address())
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            );
        }
        if !message.bcc.is_empty() {
            body.insert(
                "Bcc".into(),
                serde_json::Value::String(
                    message
                        .bcc
                        .iter()
                        .map(|a| a.address())
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            );
        }
        if !message.reply_to.is_empty() {
            body.insert(
                "ReplyTo".into(),
                serde_json::Value::String(message.reply_to[0].address().to_string()),
            );
        }
        if let Some(ref text) = message.text_body {
            body.insert("TextBody".into(), serde_json::Value::String(text.clone()));
        }
        if let Some(ref html) = message.html_body {
            body.insert("HtmlBody".into(), serde_json::Value::String(html.clone()));
        }
        if !message.headers.is_empty() {
            let headers: serde_json::Map<String, serde_json::Value> = message
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            body.insert("Headers".into(), serde_json::Value::Object(headers));
        }
        if !message.attachments.is_empty() {
            let attachments: Vec<serde_json::Value> = message
                .attachments
                .iter()
                .map(|att| {
                    let mut map = serde_json::Map::new();
                    map.insert("Name".into(), serde_json::Value::String(att.name.clone()));
                    map.insert(
                        "Content".into(),
                        serde_json::Value::String(base64_encode(&att.content)),
                    );
                    map.insert(
                        "ContentType".into(),
                        serde_json::Value::String(att.content_type.clone()),
                    );
                    serde_json::Value::Object(map)
                })
                .collect();
            body.insert("Attachments".into(), serde_json::Value::Array(attachments));
        }

        let response = self
            .client
            .post("https://api.postmarkapp.com/email")
            .header("X-Postmark-Server-Token", &self.server_token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("Postmark request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(super::http::provider_error("Postmark", status, response).await);
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
