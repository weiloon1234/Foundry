pub mod address;
pub mod attachment;
pub(crate) mod callback;
pub mod config;
pub mod driver;
mod http;
pub mod job;
pub mod log;
pub mod mailer;
pub mod mailgun;
pub mod message;
pub mod postmark;
pub mod resend;
pub mod ses;
pub mod smtp;
pub mod template;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::config::ConfigRepository;
use crate::foundation::{AppContext, Error, Result};
use crate::support::sync::lock_unpoisoned;

// Public re-exports — also available for internal use within this module
pub use address::EmailAddress;
pub use attachment::{EmailAttachment, ResolvedAttachment};
pub use config::{
    EmailConfig, EmailFromConfig, MailgunRegion, ResolvedLogConfig, ResolvedMailgunConfig,
    ResolvedPostmarkConfig, ResolvedResendConfig, ResolvedSesConfig, ResolvedSmtpConfig,
    SmtpEncryption,
};
pub use driver::{EmailDriver, OutboundEmail};
pub use log::LogEmailDriver;
pub use mailer::EmailMailer;
pub use mailgun::MailgunEmailDriver;
pub use message::EmailMessage;
pub use postmark::PostmarkEmailDriver;
pub use resend::ResendEmailDriver;
pub use ses::SesEmailDriver;
pub use smtp::SmtpEmailDriver;
pub use template::{RenderedTemplate, TemplateRenderer};

// --- Driver Registry (mirrors StorageDriverRegistryBuilder) ---

pub type EmailDriverFactory =
    Arc<dyn Fn(&ConfigRepository, &toml::Table) -> Result<Arc<dyn EmailDriver>> + Send + Sync>;

const BUILT_IN_EMAIL_DRIVERS: &[&str] = &["smtp", "log", "resend", "postmark", "mailgun", "ses"];

pub(crate) type EmailDriverRegistryHandle = Arc<Mutex<EmailDriverRegistryBuilder>>;

pub(crate) struct EmailDriverRegistryBuilder {
    drivers: HashMap<String, EmailDriverFactory>,
}

impl EmailDriverRegistryBuilder {
    pub(crate) fn shared() -> EmailDriverRegistryHandle {
        Arc::new(Mutex::new(Self {
            drivers: HashMap::new(),
        }))
    }

    pub(crate) fn register(&mut self, name: String, factory: EmailDriverFactory) -> Result<()> {
        if self.drivers.contains_key(&name) {
            return Err(Error::message(format!(
                "email driver `{name}` already registered"
            )));
        }
        self.drivers.insert(name, factory);
        Ok(())
    }

    pub(crate) fn freeze_shared(
        handle: EmailDriverRegistryHandle,
    ) -> HashMap<String, EmailDriverFactory> {
        let mut builder = lock_unpoisoned(&handle, "email driver registry");
        std::mem::take(&mut builder.drivers)
    }
}

// --- EmailManager ---

#[derive(Clone)]
pub struct EmailManager {
    default: String,
    from_config: EmailFromConfig,
    attachment_limits: config::EmailAttachmentLimits,
    drivers: Arc<HashMap<String, Arc<dyn EmailDriver>>>,
    app: AppContext,
}

impl std::fmt::Debug for EmailManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailManager")
            .field("default", &self.default)
            .field("mailers", &self.drivers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl EmailManager {
    /// Construct from config + custom drivers. **Synchronous** (not async like StorageManager).
    pub(crate) fn from_config(
        config: &ConfigRepository,
        custom_drivers: HashMap<String, EmailDriverFactory>,
        app: AppContext,
    ) -> Result<Self> {
        let email_config = config.email()?;

        if email_config.mailers.is_empty() {
            let attachment_limits = config::EmailAttachmentLimits::from(&email_config);
            return Ok(Self {
                default: email_config.default,
                from_config: email_config.from,
                attachment_limits,
                drivers: Arc::new(HashMap::new()),
                app,
            });
        }

        let mut drivers = HashMap::new();
        for (name, table) in &email_config.mailers {
            let driver_key = table
                .get("driver")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    BUILT_IN_EMAIL_DRIVERS
                        .contains(&name.as_str())
                        .then_some(name.as_str())
                })
                .ok_or_else(|| {
                    Error::message(format!("mailer `{name}` missing required 'driver' field"))
                })?;

            let driver: Arc<dyn EmailDriver> = match driver_key {
                "smtp" => Arc::new(smtp::SmtpEmailDriver::from_config(
                    &ResolvedSmtpConfig::from_table(table)?,
                )?),
                "log" => Arc::new(log::LogEmailDriver::from_config(
                    &ResolvedLogConfig::from_table(table),
                )),
                "resend" => Arc::new(resend::ResendEmailDriver::from_config(
                    &config::ResolvedResendConfig::from_table(table)?,
                )),
                "postmark" => Arc::new(postmark::PostmarkEmailDriver::from_config(
                    &config::ResolvedPostmarkConfig::from_table(table)?,
                )),
                "mailgun" => Arc::new(mailgun::MailgunEmailDriver::from_config(
                    &config::ResolvedMailgunConfig::from_table(table)?,
                )),
                "ses" => Arc::new(ses::SesEmailDriver::from_config(
                    &config::ResolvedSesConfig::from_table(table)?,
                )),
                custom_name => {
                    let factory = custom_drivers.get(custom_name).ok_or_else(|| {
                        Error::message(format!("unknown email driver `{custom_name}`"))
                    })?;
                    callback::build_email_driver(custom_name, factory, config, table)?
                }
            };
            drivers.insert(name.clone(), driver);
        }

        // Validate default mailer exists
        if !drivers.contains_key(&email_config.default) && !email_config.mailers.is_empty() {
            return Err(Error::message(format!(
                "default mailer `{}` is not configured",
                email_config.default
            )));
        }

        let attachment_limits = config::EmailAttachmentLimits::from(&email_config);
        Ok(Self {
            default: email_config.default,
            from_config: email_config.from,
            attachment_limits,
            drivers: Arc::new(drivers),
            app,
        })
    }

    pub fn mailer(&self, name: &str) -> Result<EmailMailer> {
        self.drivers
            .get(name)
            .ok_or_else(|| Error::message(format!("mailer `{name}` is not configured")))?;
        Ok(EmailMailer::new(self.app.clone(), Some(name.to_string())))
    }

    pub fn default_mailer(&self) -> Result<EmailMailer> {
        Ok(EmailMailer::new(self.app.clone(), None))
    }

    pub fn default_mailer_name(&self) -> &str {
        &self.default
    }

    pub fn from_address(&self) -> &EmailFromConfig {
        &self.from_config
    }

    pub(crate) fn attachment_limits(&self) -> config::EmailAttachmentLimits {
        self.attachment_limits
    }

    pub fn configured_mailers(&self) -> Vec<String> {
        let mut names: Vec<String> = self.drivers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get the driver for a mailer (used by EmailMailer internally).
    pub(crate) fn driver(&self, name: Option<&str>) -> Result<Arc<dyn EmailDriver>> {
        let key = name.unwrap_or(&self.default);
        self.drivers
            .get(key)
            .cloned()
            .ok_or_else(|| Error::message(format!("mailer `{}` is not configured", key)))
    }

    // Convenience methods — delegate to default mailer

    pub async fn send(&self, message: EmailMessage) -> Result<()> {
        self.default_mailer()?.send(message).await
    }

    pub async fn queue(&self, message: EmailMessage) -> Result<()> {
        self.default_mailer()?.queue(message).await
    }

    pub async fn queue_later(&self, message: EmailMessage, run_at_millis: i64) -> Result<()> {
        self.default_mailer()?
            .queue_later(message, run_at_millis)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use async_trait::async_trait;
    use tempfile::TempDir;

    use super::*;
    use crate::foundation::Container;
    use crate::validation::RuleRegistry;

    fn config_from_toml(raw: &str) -> ConfigRepository {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("email.toml");
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(raw.as_bytes()).unwrap();
        ConfigRepository::from_dir(dir.path()).unwrap()
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    struct PanickingEmailDriver;

    #[async_trait]
    impl EmailDriver for PanickingEmailDriver {
        async fn send(&self, _message: &OutboundEmail) -> Result<()> {
            panic!("driver send exploded")
        }
    }

    fn panic_driver_config() -> ConfigRepository {
        config_from_toml(
            r#"
            [email]
            default = "panic"
            from.address = "noreply@example.com"

            [email.mailers.panic]
            driver = "panic"
        "#,
        )
    }

    // --- Config-only tests (no AppContext needed) ---

    #[test]
    fn email_config_default_values() {
        let config = EmailConfig::default();
        assert_eq!(config.default, "smtp");
        assert_eq!(config.queue, "default");
        assert_eq!(config.from.address, "");
        assert_eq!(config.from.name, "");
        assert!(config.mailers.is_empty());
    }

    #[test]
    fn email_config_from_toml_full() {
        let raw = r#"
            default = "log"
            queue = "emails"
            max_attachment_bytes = 100
            max_total_attachment_bytes = 200
            from.address = "noreply@example.com"
            from.name = "Foundry App"
            [mailers.log]
            driver = "log"
            target = "email.outbound"
            [mailers.smtp]
            driver = "smtp"
            host = "smtp.example.com"
            port = 587
        "#;
        let config: config::EmailConfig = toml::from_str(raw).unwrap();
        assert_eq!(config.default, "log");
        assert_eq!(config.queue, "emails");
        assert_eq!(config.max_attachment_bytes, 100);
        assert_eq!(config.max_total_attachment_bytes, 200);
        assert_eq!(config.from.address, "noreply@example.com");
        assert_eq!(config.from.name, "Foundry App");
        assert_eq!(config.mailers.len(), 2);
    }

    #[test]
    fn email_manager_infers_builtin_driver_from_mailer_name() {
        let config = config_from_toml(
            r#"
            [email]
            default = "resend"
            from.address = "noreply@example.com"

            [email.mailers.resend]
            api_key = "test-key"
        "#,
        );
        let manager = EmailManager::from_config(&config, HashMap::new(), test_app())
            .expect("resend mailer should infer driver from mailer name");

        assert_eq!(manager.default_mailer_name(), "resend");
        assert_eq!(manager.configured_mailers(), vec!["resend"]);
    }

    #[test]
    fn email_manager_custom_mailer_still_requires_driver() {
        let config = config_from_toml(
            r#"
            [email]
            default = "transactional"

            [email.mailers.transactional]
            api_key = "test-key"
        "#,
        );
        let err = EmailManager::from_config(&config, HashMap::new(), test_app())
            .expect_err("custom mailer names must declare a driver");

        assert!(err.to_string().contains("missing required 'driver' field"));
    }

    // --- Driver registry tests ---

    #[test]
    fn email_driver_registry_register_and_freeze() {
        let handle = EmailDriverRegistryBuilder::shared();
        let factory: EmailDriverFactory = Arc::new(|_config, _table| {
            Ok(Arc::new(log::LogEmailDriver::from_config(
                &ResolvedLogConfig {
                    target: "test".to_string(),
                },
            )))
        });
        handle
            .lock()
            .expect("lock")
            .register("custom".to_string(), factory)
            .unwrap();

        let drivers = EmailDriverRegistryBuilder::freeze_shared(handle);
        assert!(drivers.contains_key("custom"));
    }

    #[test]
    fn email_driver_registry_duplicate_returns_error() {
        let handle = EmailDriverRegistryBuilder::shared();
        let factory: EmailDriverFactory = Arc::new(|_config, _table| {
            Ok(Arc::new(log::LogEmailDriver::from_config(
                &ResolvedLogConfig {
                    target: "test".to_string(),
                },
            )))
        });
        handle
            .lock()
            .expect("lock")
            .register("dup".to_string(), factory.clone())
            .unwrap();
        let result = handle
            .lock()
            .expect("lock")
            .register("dup".to_string(), factory);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("already registered"));
    }

    #[test]
    fn email_driver_factory_panic_becomes_error() {
        let factory: EmailDriverFactory =
            Arc::new(|_config, _table| panic!("driver factory exploded"));
        let mut custom = HashMap::new();
        custom.insert("panic".to_string(), factory);

        let error = EmailManager::from_config(&panic_driver_config(), custom, test_app())
            .expect_err("panicking driver factory should become an error");

        assert!(error
            .to_string()
            .contains("email driver `panic` factory panicked: driver factory exploded"));
    }

    #[tokio::test]
    async fn email_driver_send_panic_becomes_error() {
        let app = test_app();
        let factory: EmailDriverFactory =
            Arc::new(|_config, _table| Ok(Arc::new(PanickingEmailDriver)));
        let mut custom = HashMap::new();
        custom.insert("panic".to_string(), factory);
        let manager = EmailManager::from_config(&panic_driver_config(), custom, app.clone())
            .expect("panic driver should be configured");
        app.container().singleton_arc(Arc::new(manager)).unwrap();

        let message = EmailMessage::new("Hello")
            .to("user@example.com")
            .text_body("Hi");
        let error = app.email().unwrap().send(message).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("email driver `panic` send panicked: driver send exploded"));
    }

    // --- Log driver tests ---

    #[tokio::test]
    async fn log_driver_send_returns_ok() {
        use address::EmailAddress;
        use driver::OutboundEmail;

        let driver = log::LogEmailDriver::from_config(&ResolvedLogConfig {
            target: "test.email".to_string(),
        });
        let message = OutboundEmail {
            from: EmailAddress::new("sender@example.com"),
            to: vec![EmailAddress::new("recipient@example.com")],
            cc: vec![],
            bcc: vec![],
            reply_to: vec![],
            subject: "Test".to_string(),
            text_body: Some("Hello".to_string()),
            html_body: None,
            headers: Default::default(),
            attachments: vec![],
        };
        let result = driver.send(&message).await;
        assert!(result.is_ok());
    }
}
