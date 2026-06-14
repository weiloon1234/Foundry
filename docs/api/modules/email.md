# email

Multi-driver email: SMTP, Mailgun, Postmark, Resend, SES

[Back to index](../index.md)

## foundry::email

```rust
pub type EmailDriverFactory = Arc<dyn Fn(&ConfigRepository, &Table) -> Result<Arc<dyn EmailDriver>> + Send + Sync>;
struct EmailManager
  fn mailer(&self, name: &str) -> Result<EmailMailer>
  fn default_mailer(&self) -> Result<EmailMailer>
  fn default_mailer_name(&self) -> &str
  fn from_address(&self) -> &EmailFromConfig
  fn configured_mailers(&self) -> Vec<String>
  async fn send(&self, message: EmailMessage) -> Result<()>
  async fn queue(&self, message: EmailMessage) -> Result<()>
  async fn queue_later( &self, message: EmailMessage, run_at_millis: i64, ) -> Result<()>
```

## foundry::email::address

```rust
struct EmailAddress
  fn new(address: impl Into<String>) -> Self
  fn with_name(address: impl Into<String>, name: impl Into<String>) -> Self
  fn address(&self) -> &str
  fn name(&self) -> Option<&str>
```

## foundry::email::attachment

```rust
enum EmailAttachment { Path, Storage }
  fn from_path(path: impl Into<String>) -> Self
  fn from_storage(disk: impl Into<String>, path: impl Into<String>) -> Self
  fn with_name(self, name: impl Into<String>) -> Self
  fn with_content_type(self, ct: impl Into<String>) -> Self
  fn name(&self) -> Option<&str>
  fn content_type(&self) -> Option<&str>
  fn path(&self) -> &str
struct ResolvedAttachment
```

## foundry::email::config

```rust
enum MailgunRegion { Us, Eu }
enum SmtpEncryption { StartTls, Tls, None }
struct EmailConfig
struct EmailFromConfig
struct ResolvedLogConfig
  fn from_table(table: &Table) -> Self
struct ResolvedMailgunConfig
  fn from_table(table: &Table) -> Result<Self>
  fn base_url(&self) -> String
struct ResolvedPostmarkConfig
  fn from_table(table: &Table) -> Result<Self>
struct ResolvedResendConfig
  fn from_table(table: &Table) -> Result<Self>
struct ResolvedSesConfig
  fn from_table(table: &Table) -> Result<Self>
struct ResolvedSmtpConfig
  fn from_table(table: &Table) -> Result<Self>
```

## foundry::email::driver

```rust
struct OutboundEmail
trait EmailDriver
  fn send<'life0, 'life1, 'async_trait>(
```

## foundry::email::job

```rust
struct SendQueuedEmailJob
```

## foundry::email::log

```rust
struct LogEmailDriver
  fn from_config(config: &ResolvedLogConfig) -> Self
```

## foundry::email::mailer

```rust
struct EmailMailer
  fn name(&self) -> Option<&str>
  async fn send(&self, message: EmailMessage) -> Result<()>
  async fn queue(&self, message: EmailMessage) -> Result<()>
  async fn queue_later( &self, message: EmailMessage, run_at_millis: i64, ) -> Result<()>
```

## foundry::email::mailgun

```rust
struct MailgunEmailDriver
  fn from_config(config: &ResolvedMailgunConfig) -> Self
```

## foundry::email::message

```rust
struct EmailMessage
  fn new(subject: impl Into<String>) -> Self
  fn from(self, addr: impl Into<EmailAddress>) -> Self
  fn to(self, addr: impl Into<EmailAddress>) -> Self
  fn cc(self, addr: impl Into<EmailAddress>) -> Self
  fn bcc(self, addr: impl Into<EmailAddress>) -> Self
  fn reply_to(self, addr: impl Into<EmailAddress>) -> Self
  fn text_body(self, body: impl Into<String>) -> Self
  fn html_body(self, body: impl Into<String>) -> Self
  fn template( self, template_name: &str, template_path: &str, variables: Value, ) -> Result<Self>
  fn header(self, key: impl Into<String>, value: impl Into<String>) -> Self
  fn attach(self, attachment: EmailAttachment) -> Self
```

## foundry::email::postmark

```rust
struct PostmarkEmailDriver
  fn from_config(config: &ResolvedPostmarkConfig) -> Self
```

## foundry::email::resend

```rust
struct ResendEmailDriver
  fn from_config(config: &ResolvedResendConfig) -> Self
```

## foundry::email::ses

```rust
struct SesEmailDriver
  fn from_config(config: &ResolvedSesConfig) -> Self
```

## foundry::email::smtp

```rust
struct SmtpEmailDriver
  fn from_config(config: &ResolvedSmtpConfig) -> Result<Self>
```

## foundry::email::template

```rust
struct RenderedTemplate
struct TemplateRenderer
  fn new(base_path: impl Into<PathBuf>) -> Self
  fn render( &self, template_name: &str, variables: &Value, ) -> Result<RenderedTemplate>
  fn exists(&self, template_name: &str) -> bool
```

## Notes

- Built-in HTTP mailers use `timeout_secs = 30` by default; `0` disables the reqwest timeout for local debugging.
- `EmailConfig.max_attachment_bytes` and `max_total_attachment_bytes` bound resolved attachment payloads before provider delivery; `0` disables each cap.
- The built-in SES driver uses the SES SendEmail API and rejects attachments clearly instead of silently dropping them.
- Provider error bodies are truncated and obvious secret fields are redacted before they are returned or logged.

