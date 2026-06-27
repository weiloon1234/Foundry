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

After `types:export`, frontend preview/test-send forms can import
`EmailRuntimeManifest`, `emailMaxAttachmentBytes()`,
`emailMaxTotalAttachmentBytes()`, `emailAttachmentLimits()`,
`EmailMaxAttachmentBytes`, and `EmailMaxTotalAttachmentBytes` instead of copying
these backend caps.

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

`types:export` also emits `EmailManifest.ts` for configured mailers. The
manifest includes logical mailer names, driver keys, and the default mailer flag
plus `EmailRuntimeManifest` queue and attachment-limit metadata without exposing
provider endpoints, sender settings, templates, or credentials, so admin tooling
and preview/test-send forms can import `EmailMailerIds`, `DefaultEmailMailer`,
`ConfiguredDefaultEmailMailer`, `EmailDefaultQueue`, `emailDefaultMailerName()`,
`emailConfiguredDefaultMailerName()`, `emailDefaultQueue()`, `emailMailers()`,
`emailMailerNames()`, `isEmailMailerName()`, `emailMailerNameOrNull()`,
`emailDefaultMailerManifestEntry()`, `emailMailerDriverNames()`,
`isEmailMailerDriverName()`, `emailMailerDriverNameOrNull()`,
`emailMailerNamesByDriver()`, `emailMailersByDriver()`, `emailMailerDriver()`,
`emailMaxAttachmentBytes()`, `emailMaxTotalAttachmentBytes()`, and
`emailAttachmentLimits()` instead of copying mailer names, default-mailer
lookups, queue names, driver guards, driver filters, or attachment caps.
Generated email constants are frozen at runtime, while email selector helpers
return cloned mailer entries and attachment-limit metadata for local preview or
delivery-state annotations. Runtime manifest export requires a non-empty
trimmed configured default mailer, a non-empty trimmed queue, matching default
mailer metadata when mailer descriptors are present, attachment caps within
JavaScript's safe integer range, and a per-file attachment cap no larger than
the total attachment cap when both are enabled. `max_attachment_bytes = 0` and
`max_total_attachment_bytes = 0` remain valid no-cap sentinels.

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
            "order_id": self.order_id,
        }))
    }
}
```

Use `make:notification --name OrderShipped` when the notification payload should
also be a generated frontend contract. The scaffold derives `serde::Serialize`,
`serde::Deserialize`, `foundry::ts_rs::TS`, `foundry::TS`, and `ApiSchema`,
implements `Notification`, defaults to database and broadcast channels, and
registers `TsNotification` beside the backend-owned notification type. Existing
hand-written notifications can follow the same pattern before regenerating
frontend types.

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

### Within Transactions

Notifications dispatched after successful commit:

```rust
let mut tx = app.begin_transaction().await?;

// ... create order ...

tx.notify_after_commit(&user, &OrderShipped {
    order_id: order.id.to_string(),
    tracking_number: tracking.clone(),
});

tx.commit().await?;
// Notification is sent only after commit succeeds
```

### Built-in Channels

| Channel | Constant | What it does |
|---------|----------|-------------|
| Email | `NOTIFY_EMAIL` | Sends via `to_email()` using the email system |
| Database | `NOTIFY_DATABASE` | Stores `to_database()` JSON in `notifications` table |
| Broadcast | `NOTIFY_BROADCAST` | Publishes `to_broadcast()` JSON via WebSocket |

Broadcast notifications use the canonical WebSocket channel
`NOTIFICATION_BROADCAST_CHANNEL` (`"notifications"`), the canonical event
`NOTIFICATION_BROADCAST_EVENT` (`"notification"`), and the notifiable's
`notification_id()` as the WebSocket room. The WebSocket payload is the
framework-owned `NotificationBroadcastPayload` envelope:
`{ notification_type, data }`, where `data` is the JSON returned by
`to_broadcast()`. Register that channel in your WebSocket routes and have
clients subscribe to the room for the current user:

```rust
registrar.channel_with_options(
    NOTIFICATION_BROADCAST_CHANNEL,
    |_context, _payload| async { Ok(()) },
    WebSocketChannelOptions::new().server_event(NOTIFICATION_BROADCAST_EVENT),
)?;
```

Register broadcast payload DTOs with `TsNotification` when frontend clients
should receive a typed `data` contract from `types:export`:

Notifications generated by `make:notification` already include this registration
and use the notification struct itself as the typed `data` payload.

```rust
#[derive(Serialize, TS, ApiSchema)]
pub struct OrderShippedBroadcastPayload {
    pub order_id: String,
    pub tracking_number: String,
}

foundry::inventory::submit! {
    TsNotification {
        notification_type: "order_shipped",
        payload: "OrderShippedBroadcastPayload",
    }
}

fn to_broadcast(&self) -> Option<Value> {
    serde_json::to_value(OrderShippedBroadcastPayload {
        order_id: self.order_id.clone(),
        tracking_number: self.tracking_number.clone(),
    }).ok()
}
```

Generated `NotificationManifest.ts` exports `NotificationManifest`,
`NotificationChannelIds`, `NotificationBroadcastChannel`,
`NotificationBroadcastEvent`, `NotificationTypes`, `NotificationPayloadMap`,
`NotificationPayloadName`, `notificationTypes()`, `notificationChannelNames()`,
`notificationEmailChannelName()`, `notificationDatabaseChannelName()`,
`notificationBroadcastDeliveryChannelName()`, `notificationBroadcastChannel()`,
`notificationBroadcastEvent()`, `notificationEntries()`,
`notificationPayloadNames()`, `notificationsWithPayload()`,
`notificationTypesWithPayload()`, `notificationUsesPayload()`,
`isNotificationPayloadName()`, `isNotificationType()`,
`notificationTypeOrNull()`, `notificationChannelNameOrNull()`,
`notificationPayloadNameOrNull()`,
`notificationChannelIsEmail()`, `notificationChannelIsDatabase()`,
`notificationChannelIsBroadcast()`, `notificationIsBroadcastChannel()`,
`notificationIsBroadcastEvent()`, `isRegisteredNotificationBroadcastPayload()`,
`notificationBroadcastPayloadType()`, `notificationBroadcastPayloadManifestEntry()`,
`notificationBroadcastPayloadName()`, `isTypedNotificationBroadcastPayload()`,
and `TypedNotificationBroadcastPayload`.
Use `NotificationBroadcastChannel` / `NotificationBroadcastEvent` when subscribing, then
`isRegisteredNotificationBroadcastPayload(payload)` for unknown incoming
envelopes, `notificationBroadcastPayloadType(payload)` /
`notificationBroadcastPayloadName(payload)` for dashboard metadata, or
`isTypedNotificationBroadcastPayload("order_shipped", payload)` /
`TypedNotificationBroadcastPayload<"order_shipped">` to narrow the existing
`NotificationBroadcastPayload` envelope to a specific backend-owned payload DTO.
Notification dashboards can group/filter registered notifications by backend
payload contract without copying notification type strings or payload schema
names.
Generated notification manifests, channel id maps, and notification type lists
are frozen at runtime, so direct mutation cannot change backend-owned
notification metadata. Notification selector helpers clone manifest entries
before returning them, so dashboards can add local delivery status or grouping
state to selector results.

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
