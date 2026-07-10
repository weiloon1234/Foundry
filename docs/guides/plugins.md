# Plugin Examples

Real-world use cases showing what Foundry plugins can do. Each example is a standalone plugin demonstrating different framework capabilities.

---

## 1. Slack Notification Channel

A simple infrastructure plugin that adds Slack as a notification delivery channel.

**Capabilities used:** `register_notification_channel`, `config_defaults`, `boot()`

```rust
use foundry::prelude::*;
use semver::{Version, VersionReq};

struct SlackPlugin;

struct SlackChannel {
    webhook_url: String,
}

#[async_trait]
impl NotificationChannel for SlackChannel {
    async fn send(
        &self,
        _app: &AppContext,
        notifiable: &dyn Notifiable,
        notification: &dyn Notification,
    ) -> Result<()> {
        let payload = notification
            .to_channel("slack", notifiable)
            .unwrap_or_default();

        reqwest::Client::new()
            .post(&self.webhook_url)
            .json(&serde_json::json!({ "text": payload.to_string() }))
            .send()
            .await
            .map_err(Error::other)?;

        Ok(())
    }
}

const NOTIFY_SLACK: NotificationChannelId = NotificationChannelId::new("slack");

impl Plugin for SlackPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("slack", Version::new(1, 0, 0), VersionReq::parse(">=0.1").unwrap())
            .description("Slack notification channel")
    }

    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        r.config_defaults(toml::from_str(r#"
            [plugins.slack]
            webhook_url = ""
        "#).unwrap());

        r.register_notification_channel(NOTIFY_SLACK, SlackChannel {
            webhook_url: String::new(), // resolved at boot
        });

        Ok(())
    }
}
```

**What the consumer sees:**

```toml
# config/plugins.toml
[plugins.slack]
webhook_url = "https://hooks.slack.com/services/T00/B00/xxxx"
```

```rust
// In a notification:
impl Notification for OrderPlaced {
    fn via(&self) -> Vec<NotificationChannelId> {
        vec![NOTIFY_EMAIL, NOTIFY_SLACK]  // sends to both email and Slack
    }

    fn to_channel(&self, channel: &str, _notifiable: &dyn Notifiable) -> Option<Value> {
        match channel {
            "slack" => Some(json!({ "text": format!("New order #{}", self.order_id) })),
            _ => None,
        }
    }
}
```

---

## 2. Audit Review Plugin

Foundry now ships first-party audit capture, so a plugin should extend the built-in `audit_logs`
table instead of reimplementing event listeners and writers. A plugin is still a good fit for
review APIs, exports, retention policy, or project-specific admin tooling.

**Capabilities used:** `register_routes`, `register_schedule`

```rust
use foundry::prelude::*;
use foundry::audit::AuditLog;
use semver::{Version, VersionReq};

struct AuditReviewPlugin;

// ── Schedule: purge old entries ──

const AUDIT_CLEANUP: ScheduleId = ScheduleId::new("audit.cleanup");

// ── Plugin registration ──

impl Plugin for AuditReviewPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("audit", Version::new(1, 0, 0), VersionReq::parse(">=0.1").unwrap())
            .description("Routes and retention policy for built-in audit logs")
    }

    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        // API to view audit logs
        r.register_routes(|r| {
            r.route_with_options(
                "/audit/logs",
                get(list_audit_logs),
                HttpRouteOptions::new().guard(Guard::Admin),
            );
            Ok(())
        });

        // Cleanup schedule
        r.register_schedule(|s| {
            s.daily(AUDIT_CLEANUP, |inv| async move {
                let db = inv.app().database()?;
                db.raw_execute(
                    "DELETE FROM audit_logs WHERE created_at < NOW() - INTERVAL '90 days'",
                    &[],
                ).await?;
                Ok(())
            })
        });

        Ok(())
    }
}

async fn list_audit_logs(State(app): State<AppContext>) -> impl IntoResponse {
    let db = app.database().unwrap();
    let logs = AuditLog::query()
        .where_(AuditLog::AREA.eq(Some("admin".to_string())))
        .order_by(AuditLog::CREATED_AT.desc())
        .limit(50)
        .all(&*db)
        .await
        .unwrap();
    Json(logs)
}
```

```
GET /audit/logs          → JSON list of recent audit entries
Schedule: daily cleanup  → purges entries older than 90 days
```

If your project uses area-gated auditing, plugins can use `AuditLog::AREA` to expose separate admin,
support, or operations review screens without reimplementing audit capture.

---

## 3. Cloudflare R2 Storage Driver

An infrastructure plugin that adds Cloudflare R2 (S3-compatible) as a storage backend.

**Capabilities used:** `register_storage_driver`, `config_defaults`

```rust
use foundry::prelude::*;
use semver::{Version, VersionReq};

struct R2StoragePlugin;

// The actual adapter would use the S3 client with R2-specific endpoint
struct R2Adapter {
    bucket: String,
    account_id: String,
}

#[async_trait]
impl StorageAdapter for R2Adapter {
    async fn put_bytes(&self, path: &str, bytes: &[u8]) -> Result<()> {
        // Use S3-compatible API with endpoint: https://{account_id}.r2.cloudflarestorage.com
        todo!()
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> { todo!() }
    async fn delete(&self, path: &str) -> Result<()> { todo!() }
    async fn exists(&self, path: &str) -> Result<bool> { todo!() }
    async fn copy(&self, from: &str, to: &str) -> Result<()> { todo!() }
    async fn move_to(&self, from: &str, to: &str) -> Result<()> { todo!() }
    async fn url(&self, path: &str) -> Result<String> { todo!() }
    async fn temporary_url(&self, path: &str, _expires_at: DateTime) -> Result<String> { todo!() }
    async fn put_file(&self, path: &str, temp_path: &std::path::Path, content_type: Option<&str>) -> Result<()> { todo!() }
}

impl Plugin for R2StoragePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("storage-r2", Version::new(1, 0, 0), VersionReq::parse(">=0.1").unwrap())
            .description("Cloudflare R2 storage driver")
    }

    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        r.register_storage_driver("r2", Arc::new(|config, table| {
            Box::pin(async move {
                let account_id = table.get("account_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let bucket = table.get("bucket")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                Ok(Arc::new(R2Adapter { bucket, account_id }) as Arc<dyn StorageAdapter>)
            })
        }));

        Ok(())
    }
}
```

**What the consumer sees:**

```toml
# config/storage.toml
[storage]
default = "r2"

[storage.disks.r2]
driver = "r2"              # ← matches the registered driver name
account_id = "abc123"
bucket = "my-uploads"
```

```rust
// Usage is identical to any other storage driver:
let storage = app.storage()?;
storage.put("avatars/user-1.jpg", &image_bytes).await?;
let url = storage.url("avatars/user-1.jpg")?;
```

---

## 4. Admin Dashboard Plugin

A full-featured plugin with auth, routes, datatables, assets, scaffolds, and middleware — depends on a base UI plugin.

**Capabilities used:** `register_routes`, `register_guard`, `register_policy`, `register_datatable`, `register_middleware`, `register_commands`, `register_assets`, `register_scaffolds`, dependency declaration

```rust
use foundry::prelude::*;
use semver::{Version, VersionReq};

struct AdminPlugin;

// ── Auth: admin-only guard ──

const ADMIN_GUARD: GuardId = GuardId::new("admin");
const ADMIN_ACCESS: PolicyId = PolicyId::new("admin.access");

struct AdminPolicy;

#[async_trait]
impl Policy for AdminPolicy {
    async fn evaluate(&self, actor: &Actor, _app: &AppContext) -> Result<bool> {
        Ok(actor.has_role(RoleId::new("admin")))
    }
}

// ── Plugin ──

impl Plugin for AdminPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("admin", Version::new(1, 0, 0), VersionReq::parse(">=0.1").unwrap())
            .description("Admin dashboard with user management")
    }

    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        // Auth: admin policy
        r.register_policy(ADMIN_ACCESS, AdminPolicy);

        // Routes: admin panel
        r.register_routes(|r| {
            r.group("/admin", |r| {
                r.route_with_options("/dashboard", get(admin_dashboard),
                    HttpRouteOptions::new().guard(ADMIN_GUARD).permission(PermissionId::new("admin.access")));
                r.route_with_options("/users", get(admin_users),
                    HttpRouteOptions::new().guard(ADMIN_GUARD));
                r.route_with_options("/users/:id", get(admin_user_detail),
                    HttpRouteOptions::new().guard(ADMIN_GUARD));
                Ok(())
            })
        });

        // CLI: admin management commands
        r.register_commands(|reg| {
            reg.command(
                CommandId::new("admin:create"),
                Command::new("admin:create").about("Create an admin user"),
                |inv| async move {
                    println!("Creating admin user...");
                    Ok(())
                },
            )?;
            Ok(())
        });

        // Assets: admin config template
        r.register_assets(vec![
            PluginAsset::text(
                PluginAssetId::new("admin-config"),
                PluginAssetKind::Config,
                "config/admin.toml",
                r#"[plugins.admin]
dashboard_title = "Admin Panel"
items_per_page = 25
"#,
            ),
        ])?;

        // Scaffolds: generate admin pages
        r.register_scaffolds(vec![
            PluginScaffold::new(PluginScaffoldId::new("admin-page"))
                .description("Generate an admin CRUD page")
                .variable(PluginScaffoldVar::new("model").description("Model name (e.g. Product)"))
                .file(
                    "src/admin/{{model}}.rs",
                    "pub fn {{model}}_routes(r: &mut HttpRegistrar) -> Result<()> {\n    // Generated CRUD for {{model}}\n    Ok(())\n}\n",
                ),
        ])?;

        Ok(())
    }
}

// ── Handlers ──

async fn admin_dashboard(State(app): State<AppContext>) -> impl IntoResponse {
    Json(json!({ "title": "Admin Dashboard", "status": "ok" }))
}

async fn admin_users(State(app): State<AppContext>) -> impl IntoResponse {
    Json(json!({ "users": [] }))
}

async fn admin_user_detail(State(app): State<AppContext>) -> impl IntoResponse {
    Json(json!({ "user": null }))
}
```

**What the consumer sees:**

```bash
# Install admin config template
cargo run -- plugin:install-assets --plugin admin

# Generate a CRUD page for Product
cargo run -- plugin:scaffold --plugin admin --template admin-page --set model=product

# Create an admin user
cargo run -- admin:create
```

Plugin asset and scaffold paths are always resolved inside the selected target directory. Foundry
rejects absolute paths, `..` traversal, Windows-style separators, control characters, and symlinked
descendants before writing files. The selected target directory itself may be a symlink.

```
GET /admin/dashboard     → Admin dashboard (requires admin guard)
GET /admin/users         → User listing
GET /admin/users/:id     → User detail
```

---

## 5. Webhook Dispatcher Plugin

An event-driven plugin that captures domain events, queues webhook deliveries, retries on failure, and provides CLI management.

**Capabilities used:** `listen_event`, `register_job`, `register_commands`, `register_readiness_check`, `register_schedule`, `config_defaults`, `shutdown()`

```rust
use foundry::prelude::*;
use semver::{Version, VersionReq};
use std::sync::Arc;
use tokio::sync::Mutex;

struct WebhookPlugin {
    pending: Arc<Mutex<Vec<String>>>,
}

// ── Job: deliver a webhook ──

#[derive(Debug, Serialize, Deserialize)]
struct DeliverWebhook {
    url: String,
    event_type: String,
    payload: Value,
    attempt: u32,
}

const WEBHOOK_JOB: JobId = JobId::new("webhook.deliver");

#[async_trait]
impl Job for DeliverWebhook {
    const ID: JobId = WEBHOOK_JOB;

    async fn handle(&self, _ctx: JobContext) -> Result<()> {
        let response = reqwest::Client::new()
            .post(&self.url)
            .header("X-Webhook-Event", &self.event_type)
            .json(&self.payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(Error::other)?;

        if !response.status().is_success() {
            return Err(Error::message(format!("webhook delivery failed: {}", response.status())));
        }

        Ok(())
    }

    fn max_retries(&self) -> Option<u32> {
        Some(5) // retry up to 5 times with exponential backoff
    }
}

// ── Event listener: forward all events as webhooks ──

struct WebhookForwarder;

#[async_trait]
impl<E: Event> EventListener<E> for WebhookForwarder {
    async fn handle(&self, ctx: &EventContext, event: &E) -> Result<()> {
        let config = ctx.app().config();
        let urls: Vec<String> = config
            .string("plugins.webhooks.endpoints")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        let payload = serde_json::to_value(event).unwrap_or_default();

        for url in urls {
            ctx.app().jobs()?.dispatch(DeliverWebhook {
                url,
                event_type: E::ID.to_string(),
                payload: payload.clone(),
                attempt: 0,
            })?;
        }

        Ok(())
    }
}

// ── Readiness check: verify webhook endpoint is reachable ──

struct WebhookEndpointCheck;

#[async_trait]
impl ReadinessCheck for WebhookEndpointCheck {
    async fn run(&self, app: &AppContext) -> Result<ProbeResult> {
        let url = app.config().string("plugins.webhooks.health_check_url");
        match url {
            Some(url) => {
                match reqwest::get(&url).await {
                    Ok(_) => Ok(ProbeResult::healthy("webhook.endpoint")),
                    Err(e) => Ok(ProbeResult::unhealthy("webhook.endpoint", e.to_string())),
                }
            }
            None => Ok(ProbeResult::healthy("webhook.endpoint")),
        }
    }
}

// ── Schedule: retry stuck webhooks ──

const WEBHOOK_RETRY: ScheduleId = ScheduleId::new("webhook.retry_stuck");

// ── Plugin ──

impl Plugin for WebhookPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("webhooks", Version::new(1, 0, 0), VersionReq::parse(">=0.1").unwrap())
            .description("Event-driven webhook dispatcher with retry")
    }

    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        r.config_defaults(toml::from_str(r#"
            [plugins.webhooks]
            endpoints = ""
            max_retries = 5
            timeout_seconds = 10
        "#).unwrap());

        r.register_job::<DeliverWebhook>();
        r.register_readiness_check(ProbeId::new("webhook.endpoint"), WebhookEndpointCheck);

        r.register_commands(|reg| {
            reg.command(
                CommandId::new("webhook:list"),
                Command::new("webhook:list").about("List configured webhook endpoints"),
                |inv| async move {
                    let endpoints = inv.app().config()
                        .string("plugins.webhooks.endpoints")
                        .unwrap_or_default();
                    for url in endpoints.split(',').filter(|s| !s.is_empty()) {
                        println!("  {url}");
                    }
                    Ok(())
                },
            )?;
            reg.command(
                CommandId::new("webhook:test"),
                Command::new("webhook:test").about("Send a test webhook to all endpoints"),
                |inv| async move {
                    let endpoints = inv.app().config()
                        .string("plugins.webhooks.endpoints")
                        .unwrap_or_default();
                    for url in endpoints.split(',').filter(|s| !s.is_empty()) {
                        inv.app().jobs()?.dispatch(DeliverWebhook {
                            url: url.to_string(),
                            event_type: "webhook.test".into(),
                            payload: json!({"test": true}),
                            attempt: 0,
                        })?;
                        println!("  queued → {url}");
                    }
                    Ok(())
                },
            )?;
            Ok(())
        });

        r.register_schedule(|s| {
            s.hourly(WEBHOOK_RETRY, |inv| async move {
                // Re-queue any webhooks stuck in "processing" for over 10 minutes
                let db = inv.app().database()?;
                db.raw_execute(
                    "UPDATE webhook_deliveries SET status = 'pending' WHERE status = 'processing' AND updated_at < NOW() - INTERVAL '10 minutes'",
                    &[],
                ).await?;
                Ok(())
            })
        });

        Ok(())
    }

    async fn shutdown(&self, app: &AppContext) -> Result<()> {
        // Flush any pending webhook deliveries before shutdown
        let pending = self.pending.lock().await;
        if !pending.is_empty() {
            tracing::info!(count = pending.len(), "flushing pending webhooks before shutdown");
        }
        Ok(())
    }
}
```

**What the consumer sees:**

```toml
# config/plugins.toml
[plugins.webhooks]
endpoints = "https://example.com/webhook,https://backup.example.com/webhook"
max_retries = 3
```

```bash
cargo run -- webhook:list      # List configured endpoints
cargo run -- webhook:test      # Send test webhook to all endpoints
```

```
Readiness probe: webhook.endpoint  → checks endpoint reachability
Schedule: hourly                   → retries stuck deliveries
Job: webhook.deliver               → async delivery with 5 retries + exponential backoff
Shutdown: flushes pending webhooks
```

---

## Capability Reference

Quick lookup — which example demonstrates which capability:

| Capability | Slack | Audit | R2 Storage | Admin | Webhook |
|------------|:-----:|:-----:|:----------:|:-----:|:-------:|
| `register_notification_channel` | x | | | | |
| `listen_event` | | x | | | x |
| `register_job` | | x | | | x |
| `register_routes` | | x | | x | |
| `register_guard` | | | | x | |
| `register_policy` | | | | x | |
| `register_commands` | | | | x | x |
| `register_schedule` | | x | | | x |
| `register_middleware` | | | | | |
| `register_datatable` | | | | | |
| `register_readiness_check` | | | | | x |
| `register_storage_driver` | | | x | | |
| `register_email_driver` | | | | | |
| `register_assets` | | | | x | |
| `register_scaffolds` | | | | x | |
| `config_defaults` | x | x | | | x |
| `boot()` | | | | | |
| `shutdown()` | | | | | x |
| Plugin dependencies | | | | | |
