use std::fmt::Write as _;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::foundation::{Error, Result};
use crate::support::sha256::{hex_encode, sha256_hex};

use super::address::format_address;
use super::config::ResolvedSesConfig;
use super::driver::{EmailDriver, OutboundEmail};

type HmacSha256 = Hmac<Sha256>;

pub struct SesEmailDriver {
    client: reqwest::Client,
    config: ResolvedSesConfig,
}

impl SesEmailDriver {
    pub fn from_config(config: &ResolvedSesConfig) -> Self {
        Self {
            client: super::http::client("SES", config.timeout_secs),
            config: config.clone(),
        }
    }
}

#[async_trait]
impl EmailDriver for SesEmailDriver {
    async fn send(&self, message: &OutboundEmail) -> Result<()> {
        if !message.attachments.is_empty() {
            return Err(Error::message(
                "SES mailer does not support attachments through the SendEmail driver; use SMTP, Mailgun, Postmark, Resend, or a custom raw SES driver",
            ));
        }

        let endpoint = format!("https://email.{}.amazonaws.com/", self.config.region);

        // Build the SendEmail action parameters
        let mut params: Vec<(String, String)> = vec![
            ("Action".into(), "SendEmail".into()),
            ("Version".into(), "2010-12-01".into()),
            ("Source".into(), format_address(&message.from)),
            ("Message.Subject.Data".into(), message.subject.clone()),
        ];

        // Destination
        for (i, addr) in message.to.iter().enumerate() {
            params.push((
                format!("Destination.ToAddresses.member.{}", i + 1),
                addr.address().to_string(),
            ));
        }
        for (i, addr) in message.cc.iter().enumerate() {
            params.push((
                format!("Destination.CcAddresses.member.{}", i + 1),
                addr.address().to_string(),
            ));
        }
        for (i, addr) in message.bcc.iter().enumerate() {
            params.push((
                format!("Destination.BccAddresses.member.{}", i + 1),
                addr.address().to_string(),
            ));
        }

        // Reply-To
        if !message.reply_to.is_empty() {
            params.push((
                "ReplyToAddresses.member.1".into(),
                message.reply_to[0].address().to_string(),
            ));
        }

        // Body
        if let Some(ref text) = message.text_body {
            params.push(("Message.Body.Text.Data".into(), text.clone()));
            params.push(("Message.Body.Text.Charset".into(), "UTF-8".into()));
        }
        if let Some(ref html) = message.html_body {
            params.push(("Message.Body.Html.Data".into(), html.clone()));
            params.push(("Message.Body.Html.Charset".into(), "UTF-8".into()));
        }

        // Sort parameters for canonical request
        params.sort_by(|a, b| a.0.cmp(&b.0));

        // Build query string
        let query_string: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        // Sign the request
        let now = chrono::Utc::now();
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let host = format!("email.{}.amazonaws.com", self.config.region);

        let canonical_headers = format!("host:{}\nx-amz-date:{}\n", host, amz_date);
        let signed_headers = "host;x-amz-date";

        let payload_hash = sha256_hex(query_string.as_bytes());
        let canonical_request = format!(
            "POST\n/\n{}\n{}\n{}\n{}",
            query_string, canonical_headers, signed_headers, payload_hash
        );

        let credential_scope = format!("{}/{}/ses/aws4_request", date_stamp, self.config.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date,
            credential_scope,
            sha256_hex(canonical_request.as_bytes())
        );

        // Derive signing key
        let signing_key =
            derive_signing_key(&self.config.secret, &date_stamp, &self.config.region, "ses");

        // Calculate signature
        let signature = hmac_hex(&signing_key, string_to_sign.as_bytes());

        // Build authorization header
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.config.key, credential_scope, signed_headers, signature
        );

        let response = self
            .client
            .post(&endpoint)
            .header("Host", &host)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("X-Amz-Date", &amz_date)
            .header("Authorization", &authorization)
            .body(query_string)
            .send()
            .await
            .map_err(|e| Error::message(format!("SES request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(super::http::provider_error("SES", status, response).await);
        }

        Ok(())
    }
}

fn hmac_hex(key: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length is valid");
    mac.update(data);
    hex_encode(&mac.finalize().into_bytes())
}

fn derive_signing_key(secret: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_secret = format!("AWS4{}", secret);
    let k_date = hmac_raw(k_secret.as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_raw(&k_date, region.as_bytes());
    let k_service = hmac_raw(&k_region, service.as_bytes());
    hmac_raw(&k_service, b"aws4_request")
}

fn hmac_raw(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length is valid");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn url_encode(s: &str) -> String {
    // AWS expects uppercase hex encoding for SigV4
    let mut encoded = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'~' | b'.' => {
                encoded.push(byte as char);
            }
            _ => {
                write!(encoded, "%{byte:02X}").unwrap();
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::attachment::ResolvedAttachment;
    use crate::email::driver::OutboundEmail;
    use crate::email::EmailAddress;
    use std::collections::BTreeMap;

    #[test]
    fn sha256_hex_produces_correct_hash() {
        let hash = sha256_hex(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn url_encode_preserves_safe_chars() {
        assert_eq!(url_encode("hello-world_test~.123"), "hello-world_test~.123");
    }

    #[test]
    fn url_encode_encodes_special_chars() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
    }

    #[test]
    fn derive_signing_key_produces_deterministic_result() {
        let key1 = derive_signing_key("secret", "20260101", "us-east-1", "ses");
        let key2 = derive_signing_key("secret", "20260101", "us-east-1", "ses");
        assert_eq!(key1, key2);
    }

    #[tokio::test]
    async fn ses_rejects_attachments_instead_of_dropping_them() {
        let driver = SesEmailDriver::from_config(&ResolvedSesConfig {
            key: "key".to_string(),
            secret: "secret".to_string(),
            region: "us-east-1".to_string(),
            timeout_secs: 1,
        });
        let message = OutboundEmail {
            from: EmailAddress::new("sender@example.com"),
            to: vec![EmailAddress::new("recipient@example.com")],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: "Hello".to_string(),
            text_body: Some("Body".to_string()),
            html_body: None,
            headers: BTreeMap::new(),
            attachments: vec![ResolvedAttachment {
                content: b"pdf".to_vec(),
                name: "report.pdf".to_string(),
                content_type: "application/pdf".to_string(),
            }],
        };

        let error = driver.send(&message).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("SES mailer does not support attachments"));
    }
}
