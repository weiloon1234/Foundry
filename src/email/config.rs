use std::collections::BTreeMap;

use serde::Deserialize;

use crate::foundation::{Error, Result};
use crate::support::QueueId;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    pub default: String,
    pub queue: String,
    pub template_path: String,
    pub max_attachment_bytes: u64,
    pub max_total_attachment_bytes: u64,
    #[serde(default)]
    pub from: EmailFromConfig,
    #[serde(default)]
    pub mailers: BTreeMap<String, toml::Table>,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            default: "smtp".to_string(),
            queue: "default".to_string(),
            template_path: "templates/emails".to_string(),
            max_attachment_bytes: 25 * 1024 * 1024,
            max_total_attachment_bytes: 25 * 1024 * 1024,
            from: EmailFromConfig::default(),
            mailers: BTreeMap::new(),
        }
    }
}

impl EmailConfig {
    pub fn queue_id(&self) -> Result<QueueId> {
        let queue = self.queue.trim();
        if queue.is_empty() {
            return Err(Error::message("email.queue cannot be empty"));
        }
        Ok(QueueId::owned(queue))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EmailAttachmentLimits {
    pub max_attachment_bytes: u64,
    pub max_total_attachment_bytes: u64,
}

impl From<&EmailConfig> for EmailAttachmentLimits {
    fn from(config: &EmailConfig) -> Self {
        Self {
            max_attachment_bytes: config.max_attachment_bytes,
            max_total_attachment_bytes: config.max_total_attachment_bytes,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct EmailFromConfig {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone)]
pub struct ResolvedSmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub encryption: SmtpEncryption,
    pub timeout_secs: u64,
}

impl std::fmt::Debug for ResolvedSmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedSmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &crate::support::redaction::REDACTED)
            .field("encryption", &self.encryption)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SmtpEncryption {
    #[default]
    StartTls,
    Tls,
    None,
}

impl ResolvedSmtpConfig {
    pub fn from_table(table: &toml::Table) -> Result<Self> {
        let host = table
            .get("host")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing required field 'host' for smtp mailer"))?
            .to_string();
        let port = table
            .get("port")
            .and_then(|v| v.as_integer())
            .unwrap_or(587) as u16;
        let username = table
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let password = table
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let encryption = match table
            .get("encryption")
            .and_then(|v| v.as_str())
            .unwrap_or("starttls")
        {
            "starttls" => SmtpEncryption::StartTls,
            "tls" => SmtpEncryption::Tls,
            "none" => SmtpEncryption::None,
            other => return Err(Error::message(format!("unknown smtp encryption '{other}'"))),
        };
        let timeout_secs = table
            .get("timeout_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(30) as u64;
        Ok(Self {
            host,
            port,
            username,
            password,
            encryption,
            timeout_secs,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedLogConfig {
    pub target: String,
}

impl ResolvedLogConfig {
    pub fn from_table(table: &toml::Table) -> Self {
        let target = table
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("email.outbound")
            .to_string();
        Self { target }
    }
}

#[derive(Clone)]
pub struct ResolvedResendConfig {
    pub api_key: String,
    pub timeout_secs: u64,
}

impl std::fmt::Debug for ResolvedResendConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedResendConfig")
            .field("api_key", &crate::support::redaction::REDACTED)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl ResolvedResendConfig {
    pub fn from_table(table: &toml::Table) -> Result<Self> {
        let api_key = table
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing required field 'api_key' for resend mailer"))?
            .to_string();
        let timeout_secs = timeout_secs_from_table(table);
        Ok(Self {
            api_key,
            timeout_secs,
        })
    }
}

#[derive(Clone)]
pub struct ResolvedPostmarkConfig {
    pub server_token: String,
    pub timeout_secs: u64,
}

impl std::fmt::Debug for ResolvedPostmarkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedPostmarkConfig")
            .field("server_token", &crate::support::redaction::REDACTED)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl ResolvedPostmarkConfig {
    pub fn from_table(table: &toml::Table) -> Result<Self> {
        let server_token = table
            .get("server_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Error::message("missing required field 'server_token' for postmark mailer")
            })?
            .to_string();
        let timeout_secs = timeout_secs_from_table(table);
        Ok(Self {
            server_token,
            timeout_secs,
        })
    }
}

#[derive(Clone)]
pub struct ResolvedMailgunConfig {
    pub domain: String,
    pub api_key: String,
    pub region: MailgunRegion,
    pub timeout_secs: u64,
}

impl std::fmt::Debug for ResolvedMailgunConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedMailgunConfig")
            .field("domain", &self.domain)
            .field("api_key", &crate::support::redaction::REDACTED)
            .field("region", &self.region)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MailgunRegion {
    #[default]
    Us,
    Eu,
}

impl ResolvedMailgunConfig {
    pub fn from_table(table: &toml::Table) -> Result<Self> {
        let domain = table
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing required field 'domain' for mailgun mailer"))?
            .to_string();
        let api_key = table
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing required field 'api_key' for mailgun mailer"))?
            .to_string();
        let region = match table.get("region").and_then(|v| v.as_str()).unwrap_or("us") {
            "us" => MailgunRegion::Us,
            "eu" => MailgunRegion::Eu,
            other => return Err(Error::message(format!("unknown mailgun region '{other}'"))),
        };
        Ok(Self {
            domain,
            api_key,
            region,
            timeout_secs: timeout_secs_from_table(table),
        })
    }

    pub fn base_url(&self) -> String {
        match self.region {
            MailgunRegion::Us => format!("https://api.mailgun.net/v3/{}/messages", self.domain),
            MailgunRegion::Eu => format!("https://api.eu.mailgun.net/v3/{}/messages", self.domain),
        }
    }
}

#[derive(Clone)]
pub struct ResolvedSesConfig {
    pub key: String,
    pub secret: String,
    pub region: String,
    pub timeout_secs: u64,
}

impl std::fmt::Debug for ResolvedSesConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedSesConfig")
            .field("key", &self.key)
            .field("secret", &crate::support::redaction::REDACTED)
            .field("region", &self.region)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl ResolvedSesConfig {
    pub fn from_table(table: &toml::Table) -> Result<Self> {
        let key = table
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing required field 'key' for ses mailer"))?
            .to_string();
        let secret = table
            .get("secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing required field 'secret' for ses mailer"))?
            .to_string();
        let region = table
            .get("region")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing required field 'region' for ses mailer"))?
            .to_string();
        Ok(Self {
            key,
            secret,
            region,
            timeout_secs: timeout_secs_from_table(table),
        })
    }
}

fn timeout_secs_from_table(table: &toml::Table) -> u64 {
    table
        .get("timeout_secs")
        .and_then(|v| v.as_integer())
        .unwrap_or(30)
        .max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_config_default_values() {
        let config = EmailConfig::default();
        assert_eq!(config.default, "smtp");
        assert_eq!(config.queue, "default");
        assert_eq!(config.max_attachment_bytes, 25 * 1024 * 1024);
        assert_eq!(config.max_total_attachment_bytes, 25 * 1024 * 1024);
        assert_eq!(config.from.address, "");
        assert_eq!(config.from.name, "");
        assert!(config.mailers.is_empty());
    }

    #[test]
    fn email_config_from_toml() {
        let toml_str = r#"
default = "log"
queue = "high_priority"
max_attachment_bytes = 1024
max_total_attachment_bytes = 2048
from.address = "test@example.com"
from.name = "Test Sender"
[mailers.smtp]
host = "smtp.example.com"
[mailers.log]
target = "custom.log"
"#;
        let config: EmailConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.default, "log");
        assert_eq!(config.queue, "high_priority");
        assert_eq!(config.max_attachment_bytes, 1_024);
        assert_eq!(config.max_total_attachment_bytes, 2_048);
        assert_eq!(config.from.address, "test@example.com");
        assert_eq!(config.from.name, "Test Sender");
        assert_eq!(config.mailers.len(), 2);
        assert_eq!(
            config.mailers["smtp"]["host"],
            toml::Value::String("smtp.example.com".to_string())
        );
        assert_eq!(
            config.mailers["log"]["target"],
            toml::Value::String("custom.log".to_string())
        );
    }

    #[test]
    fn email_from_config_default_values() {
        let from = EmailFromConfig::default();
        assert_eq!(from.address, "");
        assert_eq!(from.name, "");
    }

    #[test]
    fn resolved_smtp_config_from_table() {
        let mut table = toml::Table::new();
        table.insert(
            "host".to_string(),
            toml::Value::String("smtp.example.com".to_string()),
        );
        table.insert("port".to_string(), toml::Value::Integer(587));
        table.insert(
            "username".to_string(),
            toml::Value::String("user".to_string()),
        );
        table.insert(
            "password".to_string(),
            toml::Value::String("pass".to_string()),
        );
        table.insert(
            "encryption".to_string(),
            toml::Value::String("tls".to_string()),
        );
        table.insert("timeout_secs".to_string(), toml::Value::Integer(60));

        let config = ResolvedSmtpConfig::from_table(&table).unwrap();
        assert_eq!(config.host, "smtp.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.username, "user");
        assert_eq!(config.password, "pass");
        assert_eq!(config.encryption, SmtpEncryption::Tls);
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn resolved_smtp_config_missing_host() {
        let table = toml::Table::new();
        let result = ResolvedSmtpConfig::from_table(&table);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing required field 'host' for smtp mailer"));
    }

    #[test]
    fn resolved_smtp_config_defaults() {
        let mut table = toml::Table::new();
        table.insert(
            "host".to_string(),
            toml::Value::String("smtp.example.com".to_string()),
        );

        let config = ResolvedSmtpConfig::from_table(&table).unwrap();
        assert_eq!(config.host, "smtp.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.username, "");
        assert_eq!(config.password, "");
        assert_eq!(config.encryption, SmtpEncryption::StartTls);
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn resolved_smtp_config_unknown_encryption() {
        let mut table = toml::Table::new();
        table.insert(
            "host".to_string(),
            toml::Value::String("smtp.example.com".to_string()),
        );
        table.insert(
            "encryption".to_string(),
            toml::Value::String("custom".to_string()),
        );

        let result = ResolvedSmtpConfig::from_table(&table);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unknown smtp encryption 'custom'"));
    }

    #[test]
    fn resolved_smtp_config_encryption_variants() {
        let host = "smtp.example.com".to_string();

        for (encryption_str, expected_encryption) in [
            ("starttls", SmtpEncryption::StartTls),
            ("tls", SmtpEncryption::Tls),
            ("none", SmtpEncryption::None),
        ] {
            let mut table = toml::Table::new();
            table.insert("host".to_string(), toml::Value::String(host.clone()));
            table.insert(
                "encryption".to_string(),
                toml::Value::String(encryption_str.to_string()),
            );

            let config = ResolvedSmtpConfig::from_table(&table).unwrap();
            assert_eq!(
                config.encryption, expected_encryption,
                "Failed for encryption: {}",
                encryption_str
            );
        }
    }

    #[test]
    fn resolved_http_mailer_configs_default_timeout() {
        let mut resend = toml::Table::new();
        resend.insert(
            "api_key".to_string(),
            toml::Value::String("resend-key".to_string()),
        );
        assert_eq!(
            ResolvedResendConfig::from_table(&resend)
                .unwrap()
                .timeout_secs,
            30
        );

        let mut postmark = toml::Table::new();
        postmark.insert(
            "server_token".to_string(),
            toml::Value::String("postmark-token".to_string()),
        );
        assert_eq!(
            ResolvedPostmarkConfig::from_table(&postmark)
                .unwrap()
                .timeout_secs,
            30
        );

        let mut mailgun = toml::Table::new();
        mailgun.insert(
            "domain".to_string(),
            toml::Value::String("mg.example.com".to_string()),
        );
        mailgun.insert(
            "api_key".to_string(),
            toml::Value::String("mailgun-key".to_string()),
        );
        assert_eq!(
            ResolvedMailgunConfig::from_table(&mailgun)
                .unwrap()
                .timeout_secs,
            30
        );

        let mut ses = toml::Table::new();
        ses.insert(
            "key".to_string(),
            toml::Value::String("ses-key".to_string()),
        );
        ses.insert(
            "secret".to_string(),
            toml::Value::String("ses-secret".to_string()),
        );
        ses.insert(
            "region".to_string(),
            toml::Value::String("us-east-1".to_string()),
        );
        assert_eq!(
            ResolvedSesConfig::from_table(&ses).unwrap().timeout_secs,
            30
        );
    }

    #[test]
    fn resolved_http_mailer_configs_parse_timeout() {
        let mut table = toml::Table::new();
        table.insert(
            "api_key".to_string(),
            toml::Value::String("resend-key".to_string()),
        );
        table.insert("timeout_secs".to_string(), toml::Value::Integer(45));

        assert_eq!(
            ResolvedResendConfig::from_table(&table)
                .unwrap()
                .timeout_secs,
            45
        );
    }

    #[test]
    fn resolved_log_config_from_table() {
        let mut table = toml::Table::new();
        table.insert(
            "target".to_string(),
            toml::Value::String("custom.log".to_string()),
        );

        let config = ResolvedLogConfig::from_table(&table);
        assert_eq!(config.target, "custom.log");
    }

    #[test]
    fn resolved_log_config_default_target() {
        let table = toml::Table::new();

        let config = ResolvedLogConfig::from_table(&table);
        assert_eq!(config.target, "email.outbound");
    }
}
