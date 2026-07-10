# Email & Notifications Guide

Foundry provides multi-driver email sending and a multi-channel notification system. Notifications use email as one delivery channel alongside database storage and WebSocket broadcast.

---

## Email

### Quick Start

```rust
let email = EmailMessage::new("Welcome!")
    .to("user@example.com")
    .text_body("Thanks for signing up.");

app.email()?.send(email).await?;
```

### Building Messages

```rust
let msg = EmailMessage::new("Order Confirmation")
    .from(EmailAddress::with_name("noreply@shop.com", "My Shop"))
    .to("customer@example.com")
    .cc("manager@shop.com")
    .bcc("archive@shop.com")
    .reply_to("support@shop.com")
    .html_body("<h1>Order confirmed</h1><p>Your order #123 is on its way.</p>")
    .text_body("Order confirmed. Your order #123 is on its way.")
    .header("X-Priority", "1");
```

### Templates

Store templates as HTML/text files with `{{variable}}` placeholders:

```html
<!-- templates/emails/order_shipped.html -->
<h1>Your order has shipped!</h1>
<p>Order: {{order_id}}</p>
<p>Tracking: {{tracking.number}}</p>
```

```rust
let msg = EmailMessage::new("Your order shipped")
    .to("customer@example.com")
    .template("order_shipped", "templates/emails", json!({
        "order_id": "ORD-123",
        "tracking": { "number": "TRK-456" }
    }))
    .await?;
```

Dot-notation works for nested values: `{{tracking.number}}`.
Template names are safe relative names under the configured template directory.
Nested names such as `auth/welcome` are allowed, while absolute paths,
traversal segments, backslashes, and control characters are rejected.

### Attachments

```rust
// From filesystem
let msg = EmailMessage::new("Invoice")
    .to("customer@example.com")
    .attach(EmailAttachment::from_path("/tmp/invoice.pdf")
        .with_name("invoice.pdf")
        .with_content_type("application/pdf"));

// From storage disk
let msg = EmailMessage::new("Contract")
    .to("customer@example.com")
    .attach(EmailAttachment::from_storage("s3", "documents/contract.pdf")
        .with_name("contract.pdf"));
```

Outbound email is validated before delivery. Foundry rejects CR/LF injection in
subjects, headers, attachment filenames, and addresses; attachment display names
are sanitized before provider delivery. Custom headers must use valid HTTP-style
header token names, and custom attachment content types must be non-empty and
free of control characters.

Attachment payloads are bounded by `[email] max_attachment_bytes` and
`max_total_attachment_bytes` before provider delivery. Both default to
`26214400` bytes; set either value to `0` only when the app has its own stricter
delivery controls. Filesystem attachments are checked with metadata before Foundry
reads them into memory when possible. The built-in SES driver uses Amazon SES
`SendEmail`, which does not support attachments, so Foundry rejects those messages
clearly instead of silently dropping files.

### Sending vs Queueing

```rust
let email_manager = app.email()?;

// Send immediately (blocks until sent)
email_manager.send(msg).await?;

// Queue for async delivery via background job
email_manager.queue(msg).await?;

// Queue with delay (timestamp in milliseconds)
let send_at = DateTime::now().add_days(1).timestamp_millis();
email_manager.queue_later(msg, send_at).await?;
```

### Multiple Mailers

Use different drivers for different purposes:

```rust
// Default mailer (configured in [email] default = "smtp")
app.email()?.send(msg).await?;

// Specific mailer by name
let transactional = app.email()?.mailer("postmark")?;
transactional.send(msg).await?;

let marketing = app.email()?.mailer("mailgun")?;
marketing.send(msg).await?;
```

### Config

```toml
# config/email.toml
[email]
default = "smtp"
queue = "default"
template_path = "templates/emails"
max_attachment_bytes = 26214400
max_total_attachment_bytes = 26214400

[email.from]
address = "noreply@example.com"
name = "My App"

# SMTP
[email.mailers.smtp]
host = "smtp.example.com"
port = 587
username = "user"
password = "pass"
encryption = "starttls"     # "starttls", "tls", or "none"
timeout_secs = 30

# Postmark
[email.mailers.postmark]
server_token = "your-token"
timeout_secs = 30

# Resend
[email.mailers.resend]
api_key = "re_xxx"
timeout_secs = 30

# Mailgun
[email.mailers.mailgun]
domain = "mg.example.com"
api_key = "key-xxx"
region = "us"               # "us" or "eu"
timeout_secs = 30

# AWS SES
[email.mailers.ses]
key = "AKIA..."
secret = "xxx"
region = "us-east-1"
timeout_secs = 30

# Log driver (development — prints to stdout instead of sending)
[email.mailers.log]
target = "email.outbound"
```

Built-in HTTP mailers (`postmark`, `resend`, `mailgun`, and `ses`) apply
`timeout_secs = 30` by default. Set it to `0` only for local debugging where you
want reqwest's no-timeout behavior. Provider error bodies are truncated and
obvious secret fields are redacted before Foundry returns or logs the delivery
error.

### Available Drivers

| Driver | Config key | Use case |
|--------|-----------|----------|
| SMTP | `smtp` | Self-hosted or relay SMTP |
| Postmark | `postmark` | Transactional email |
| Resend | `resend` | Developer-first email |
| Mailgun | `mailgun` | Transactional + marketing |
| AWS SES | `ses` | High-volume sending |
| Log | `log` | Development — logs instead of sending |

---

## Notifications

Notifications deliver messages through multiple channels from a single dispatch call.

### Defining a Notification

```rust
struct OrderShipped {
    order_id: String,
    tracking_number: String,
}

impl Notification for OrderShipped {
    fn notification_type(&self) -> &str {
        "order_shipped"
    }

    fn via(&self) -> Vec<NotificationChannelId> {
        vec![NOTIFY_EMAIL, NOTIFY_DATABASE, NOTIFY_BROADCAST]
    }

    fn to_email(&self, notifiable: &dyn Notifiable) -> Option<EmailMessage> {
        let email = notifiable.route_notification_for("email")?;
        Some(EmailMessage::new("Your order shipped!")
            .to(email)
            .html_body(format!(
                "<p>Order <b>{}</b> is on its way.</p><p>Tracking: {}</p>",
                self.order_id, self.tracking_number
            )))
    }

    fn to_database(&self) -> Option<Value> {
        Some(json!({
            "order_id": self.order_id,
            "tracking_number": self.tracking_number,
        }))
    }

    fn to_broadcast(&self) -> Option<Value> {
        Some(json!({
            "type": "order_shipped",
            "order_id": self.order_id,
        }))
    }
}
```

### Making a Model Notifiable

```rust
impl Notifiable for User {
    fn notification_id(&self) -> String {
        self.id.to_string()
    }

    fn route_notification_for(&self, channel: &str) -> Option<String> {
        match channel {
            "email" => Some(self.email.clone()),
            "sms" => self.phone.clone(),
            _ => None,
        }
    }
}
```

### Sending Notifications

```rust
// Send immediately (all channels awaited)
app.notify(&user, &OrderShipped {
    order_id: "ORD-123".into(),
    tracking_number: "TRK-456".into(),
}).await?;

// Queue as background job
app.notify_queued(&user, &OrderShipped {
    order_id: "ORD-123".into(),
    tracking_number: "TRK-456".into(),
}).await?;
```

Immediate delivery attempts every selected channel, then returns an error containing every failed
or unregistered channel. Queued notifications pre-render before dispatch; worker delivery errors
are returned from the job so the normal retry and dead-letter policy applies. Keep queued channel
adapters idempotent because retrying a multi-channel job can replay channels that already succeeded.

### Within Transactions

Notifications dispatched after successful commit:

```rust
let mut tx = app.begin_transaction().await?;

// ... create order ...

tx.notify_after_commit(&user, &OrderShipped {
    order_id: order.id.to_string(),
    tracking_number: tracking.clone(),
})?;

tx.commit().await?;
// Notification is sent only after commit succeeds
```

`notify_after_commit` renders the queued payload immediately and returns an error if a renderer or
routing callback fails. No after-commit job is registered in that case.

### Built-in Channels

| Channel | Constant | What it does |
|---------|----------|-------------|
| Email | `NOTIFY_EMAIL` | Sends via `to_email()` using the email system |
| Database | `NOTIFY_DATABASE` | Stores `to_database()` JSON in `notifications` table |
| Broadcast | `NOTIFY_BROADCAST` | Publishes `to_broadcast()` JSON via WebSocket |

Register the broadcast WebSocket channel with the guarded helper:

```rust
fn websocket_routes(registrar: &mut WebSocketRegistrar) -> Result<()> {
    register_notification_websocket_channel(registrar, ids::AuthGuard::Api)?;
    Ok(())
}
```

Clients subscribe to `NOTIFICATION_BROADCAST_CHANNEL` with their authenticated actor ID as the
room. The helper rejects missing or different rooms, does not accept client messages, and includes
the `notification` event in generated realtime contracts. The value returned by
`Notifiable::notification_id()` must match `Actor::id`; use a custom guarded channel and ownership
authorizer when those identities intentionally differ.

### Custom Channels

Register custom channels (e.g., SMS, Slack) via ServiceProvider or Plugin:

```rust
struct SmsChannel { api_key: String }

#[async_trait]
impl NotificationChannel for SmsChannel {
    async fn send(
        &self,
        _app: &AppContext,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
    ) -> Result<()> {
        let phone = notifiable.route_notification_for("sms")
            .ok_or_else(|| Error::message("no SMS route"))?;
        let payload = notification.to_channel("sms", notifiable)
            .ok_or_else(|| Error::message("no SMS payload"))?;

        // send via SMS API...
        Ok(())
    }
}

// Register
registrar.register_notification_channel(
    NotificationChannelId::new("sms"),
    SmsChannel { api_key: "...".into() },
)?;
```

Then use in notifications:

```rust
fn via(&self) -> Vec<NotificationChannelId> {
    vec![NOTIFY_EMAIL, NotificationChannelId::new("sms")]
}

fn to_channel(&self, channel: &str, _notifiable: &dyn Notifiable) -> Option<Value> {
    match channel {
        "sms" => Some(json!({ "message": format!("Order {} shipped", self.order_id) })),
        _ => None,
    }
}
```

For queued custom channels, Foundry serializes the routing value returned by
`route_notification_for()` under its typed `NotificationChannelId`, alongside the rendered custom
payload. The worker-side adapter therefore receives the same routing value as immediate delivery.
