# Rust Email System Blueprint (Framework-Level)

## Overview

This document defines the design of a **framework-level outbound email system** for Foundry.

Goal:

> Provide a configurable, multi-mailer email subsystem with a first-class app API, a public driver interface for future providers, and a queue-aware delivery model that fits Foundry’s existing builder, provider, and jobs architecture.

This is a **design blueprint only**. It does not mean the subsystem is already implemented.

---

# Objective

Build an outbound email system that:

- supports **multiple named mailers**
- uses a **configurable default mailer**
- exposes one public API regardless of underlying transport
- ships with **SMTP** and **log** as first-class v1 drivers
- supports **custom email drivers** through the existing provider system
- integrates naturally with Foundry `AppContext`
- supports both **immediate send** and **queued send**
- reuses Foundry jobs for async delivery instead of inventing a second queueing system
- limits v1 scope to **transport + message composition**

Explicitly not part of v1:

- email templates/views
- notifications
- inbound email handling

---

# Core Philosophy

1. **One email API, many mailers**
2. **Mailer selection is config-driven**
3. **Default mailer is automatic; explicit mailer selection stays available**
4. **AppBuilder stays thin; subsystem extensibility flows through providers**
5. **Drivers own transport behavior; app code should not branch on SMTP vs other providers**
6. **Queued delivery should reuse Foundry jobs**
7. **Message payloads must be queue-safe and serializable**
8. **Templates and notifications are separate future layers, not hidden inside the transport layer**

---

# Module Shape

Introduce a new framework module:

```text
src/email/
```

Primary public types:

- `EmailManager`
- `EmailMailer`
- `EmailDriver`
- `EmailMessage`
- `EmailAddress`
- `EmailAttachment`
- `EmailConfig`
- `EmailMailerConfig`

Primary app entrypoint:

```rust
AppContext::email() -> Result<Arc<EmailManager>>
```

This should be a first-class app service, like `app.database()?`, `app.redis()?`, and the planned `app.storage()?`.

---

# Naming and Builder Pattern

Use `email` consistently as the subsystem name:

- `AppContext::email()`
- `EmailManager`
- `EmailMailer`
- `EmailDriver`
- `EmailMessage`
- `EmailAddress`
- `EmailAttachment`

Do **not** add a direct builder API like:

```rust
App::builder().register_email_driver(...)
```

Keep the current Foundry bootstrap shape intact:

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
```

Custom email drivers must be registered through the existing provider flow:

```rust
registrar.register_email_driver("resend", driver_factory)?;
```

The interface that makes this extensible is:

- the public `EmailDriver` trait
- driver factory registration on `ServiceRegistrar`

---

# Config Model

Add a new top-level typed config section:

```toml
[email]
default = "smtp"
queue = "default"

[email.from]
address = "hello@example.com"
name = "Foundry App"

[email.mailers.smtp]
driver = "smtp"
host = "127.0.0.1"
port = 1025
username = ""
password = ""
encryption = "starttls"
timeout_secs = 10

[email.mailers.log]
driver = "log"
target = "email.outbound"
```

## Typed Config Shape

```rust
pub struct EmailConfig {
    pub default: String,
    pub queue: Option<QueueId>,
    pub from: Option<EmailFromConfig>,
    pub mailers: BTreeMap<String, EmailMailerConfig>,
}
```

```rust
pub struct EmailFromConfig {
    pub address: String,
    pub name: Option<String>,
}
```

```rust
pub enum EmailMailerConfig {
    Smtp(SmtpMailerConfig),
    Log(LogMailerConfig),
    Custom(CustomMailerConfig),
}
```

```rust
pub struct SmtpMailerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub encryption: Option<String>,
    pub timeout_secs: u64,
}
```

```rust
pub struct LogMailerConfig {
    pub target: String,
}
```

```rust
pub struct CustomMailerConfig {
    pub driver: String,
    pub options: toml::Table,
}
```

## Rules

- `email.default` must point to a configured mailer
- every mailer must have a `driver`
- unknown driver is a startup/config error
- built-in v1 drivers:
  - `smtp`
  - `log`
- `email.queue` overrides the queue used for queued email delivery
- when `email.queue` is absent, queued email uses the normal Foundry jobs default queue

## Sender Precedence

Sender resolution must be:

1. explicit message `from`
2. config `email.from`
3. error if still missing

---

# Public API

## AppContext

```rust
pub fn email(&self) -> Result<Arc<EmailManager>>
```

## EmailManager

`EmailManager` is the main app-facing service.

Suggested methods:

- `default_mailer(&self) -> Result<EmailMailer>`
- `mailer(&self, name: &str) -> Result<EmailMailer>`
- `default_mailer_name(&self) -> &str`
- `configured_mailers(&self) -> Vec<String>`
- `send(&self, message: EmailMessage) -> Result<()>`
- `queue(&self, message: EmailMessage) -> Result<()>`
- `queue_later(&self, message: EmailMessage, run_at_millis: i64) -> Result<()>`

Behavior:

- `send(...)`, `queue(...)`, and `queue_later(...)` target the configured default mailer
- `mailer(name)` provides explicit named mailer selection

## EmailMailer

`EmailMailer` is a cheap cloneable handle around a resolved driver and mailer metadata.

Suggested methods:

- `name(&self) -> &str`
- `send(&self, message: EmailMessage) -> Result<()>`
- `queue(&self, message: EmailMessage) -> Result<()>`
- `queue_later(&self, message: EmailMessage, run_at_millis: i64) -> Result<()>`

## EmailAddress

`EmailAddress` is a serializable email identity value.

```rust
pub struct EmailAddress {
    pub address: String,
    pub name: Option<String>,
}
```

Convenience constructors should support both:

- `EmailAddress::new("user@example.com")`
- `EmailAddress::named("user@example.com", "Foundry User")`

## EmailMessage

`EmailMessage` is a serializable, builder-friendly outbound email value object.

```rust
pub struct EmailMessage {
    pub from: Option<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub subject: String,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub attachments: Vec<EmailAttachment>,
}
```

Rules:

- at least one of `text_body` or `html_body` is required
- templates/views are not part of v1
- `to` must not be empty
- the type must remain safe to serialize for queued delivery

Suggested builder-style usage:

```rust
EmailMessage::new()
    .to("user@example.com")
    .subject("Welcome to Foundry")
    .text("Your account is ready.")
```

## EmailAttachment

Attachments in v1 should be queue-safe only.

Supported sources:

- file path attachment
- storage-backed attachment (`disk`, `path`)

Suggested shape:

```rust
pub enum EmailAttachment {
    Path {
        path: PathBuf,
        name: Option<String>,
        content_type: Option<String>,
    },
    Storage {
        disk: String,
        path: String,
        name: Option<String>,
        content_type: Option<String>,
    },
}
```

Rules:

- raw in-memory byte attachments are **not** part of v1
- this keeps queued delivery stable and predictable

---

# Goal Usage

Default mailer send:

```rust
let email = app.email()?;

email.send(
    EmailMessage::new()
        .to("user@example.com")
        .subject("Welcome to Foundry")
        .text("Your account is ready.")
).await?;
```

Explicit named mailer + queued send:

```rust
app.email()?
    .mailer("marketing")?
    .queue(
        EmailMessage::new()
            .to("ops@example.com")
            .subject("Export Ready")
            .text("The export is ready.")
            .attach(EmailAttachment::from_storage("exports", "reports/users.xlsx"))
    )
    .await?;
```

Sender override:

```rust
app.email()?
    .send(
        EmailMessage::new()
            .from(EmailAddress::named("billing@example.com", "Billing"))
            .to("customer@example.com")
            .subject("Invoice Paid")
            .html("<strong>Thanks for your payment.</strong>")
    )
    .await?;
```

---

# Driver Model

## EmailDriver Trait

Define a public async transport interface:

```rust
#[async_trait]
pub trait EmailDriver: Send + Sync + 'static {
    async fn send(&self, mailer: &ResolvedEmailMailer, message: &ResolvedEmailMessage) -> Result<()>;
}
```

`ResolvedEmailMailer` and `ResolvedEmailMessage` represent the validated, sender-resolved form the driver receives after framework-level normalization.

## Built-in Drivers

v1 first-class drivers:

- `SmtpEmailDriver`
- `LogEmailDriver`

### SMTP

Purpose:

- real outbound delivery

Expected behavior:

- build a proper email envelope and MIME message
- support text-only, html-only, and multipart alternative content
- include attachments from supported sources

### Log

Purpose:

- local development
- non-delivery preview
- safe testing without a real transport

Expected behavior:

- log normalized outbound email details to a structured target such as `email.outbound`
- work with both immediate send and queued send

## Driver Registration Model

Built-in drivers are registered automatically by the framework.

Custom drivers are registered through the normal provider flow:

```rust
registrar.register_email_driver("resend", driver_factory)?;
```

Suggested factory shape:

```rust
type EmailDriverFactory =
    Arc<dyn Fn(&ConfigRepository, &toml::Table) -> Result<Arc<dyn EmailDriver>> + Send + Sync>;
```

Rules:

- driver lookup is by `driver = "..."`
- smtp and log are always available
- custom drivers consume their own raw config from `options`
- app code does not resolve drivers directly; it resolves mailers through `EmailManager`

Custom driver example:

```rust
pub struct AppServiceProvider;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_email_driver("resend", Arc::new(|config, options| {
            Ok(Arc::new(ResendEmailDriver::from_config(config, options)?))
        }))?;
        Ok(())
    }
}
```

---

# Queue Integration

Queued email is a first-class part of the subsystem.

Do **not** invent a second queueing system. Reuse Foundry jobs.

## Internal Job Shape

Define an internal queued-email job payload containing:

- mailer name
- serialized `EmailMessage`
- optional scheduled send time metadata if needed for diagnostics

Suggested queued job shape:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct SendQueuedEmailJob {
    pub mailer: String,
    pub message: EmailMessage,
}
```

The job queue used should be:

- `email.queue` when configured
- otherwise the normal Foundry jobs default queue

## Worker Requirements

The blueprint should explicitly lock this rule:

- worker processes must boot with the same config and providers as the HTTP/CLI app
- otherwise custom email drivers registered through providers will not exist when queued email is processed

## Retry Behavior

When queued:

- SMTP delivery failures rely on normal Foundry job retry/backoff behavior
- log driver should still succeed normally
- delivery policy lives at the job layer, not as a separate email-specific retry engine

---

# Future Layers (Not in v1)

This subsystem is the transport/message layer only.

Future work can build on top of it:

- email templates/views
- markdown/component email rendering
- notification system
- multi-channel notifications (email + SMS + push)
- provider-specific metadata and analytics

These should remain separate layers over `EmailManager`, not hidden inside the transport foundation.

---

# Test Plan

Implementation should include coverage for:

- config parsing for default sender and named mailers
- default mailer resolution
- named mailer lookup
- immediate send through the default mailer
- immediate send through an explicit named mailer
- queued send dispatch through Foundry jobs
- delayed send dispatch through Foundry jobs
- worker-side queued email delivery using registered drivers
- sender precedence:
  - message `from`
  - config `email.from`
  - error when missing
- validation that at least one of text/html body exists
- validation that `to` is not empty
- SMTP driver envelope/message building
- log driver capturing email preview without real delivery
- path-backed attachments
- storage-backed attachments
- custom driver registration through `ServiceProvider`
- unknown mailer failures
- unknown driver failures
- missing default mailer failures
- queue-safe serialization of `EmailMessage` and `EmailAttachment`

---

# Assumptions and Defaults

- root file name: `rust_email_system_blueprint_framework_level.md`
- this is a **blueprint only**, not an implementation/status update
- scope is **outbound transport + message composition**
- templates/views are future work
- notifications are future work
- built-in v1 drivers are `smtp` and `log`
- named configured units are called **mailers**
- public app entrypoint is `app.email()?`
- extension happens through `ServiceProvider` + `ServiceRegistrar`, not a direct `AppBuilder` mail-driver API
- queued delivery is part of the intended subsystem, built on Foundry jobs
- attachment sources in v1 are path/storage references only, not raw in-memory bytes

---

# One-Line Goal

> A Foundry app should be able to send or queue outbound email through `app.email()?` with a default mailer, explicit named mailers, and future provider drivers, without breaking the existing thin builder + provider architecture.
