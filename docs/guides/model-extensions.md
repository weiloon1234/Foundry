# Model Extensions Guide

Five building blocks you add to models for out-of-the-box functionality: file attachments, key-value metadata, multi-locale translations, typed enums, and country reference data.

---

## AppEnum — Typed Enums with DB + Serde + OpenAPI

Define enums that automatically serialize, store in the database, validate, and generate OpenAPI schemas.

### String-Backed (Default)

```rust
#[derive(Clone, Copy, AppEnum)]
enum OrderStatus {
    Pending,          // key: "pending",  label_key: "enum.order_status.pending",  DB: TEXT
    Processing,       // key: "processing"
    Shipped,          // key: "shipped"
    Delivered,        // key: "delivered"
    Cancelled,        // key: "cancelled"
}
```

### Int-Backed

```rust
#[derive(Clone, Copy, AppEnum)]
enum Priority {
    Low = 1,          // key: 1,  label_key: "enum.priority.low",  DB: INT4
    Medium = 2,
    High = 3,
    Critical = 4,
}
```

### Customization

```rust
#[derive(Clone, Copy, AppEnum)]
#[foundry(label_prefix = "admin.tickets.statuses")] // override the default enum.{id} namespace
enum TicketStatus {
    Open,                                         // label_key: "admin.tickets.statuses.open"
    Reviewing,
    #[foundry(key = "resolved")]                    // override stored key
    #[foundry(aliases = ["done", "finished"])]      // parse alternatives
    Resolved,
    #[foundry(label_key = "legacy.ticket_status.closed")] // override one variant only
    Closed,
}

// TicketStatus::parse_key("done")    → Some(TicketStatus::Resolved)
// TicketStatus::Open.label_key()     → "admin.tickets.statuses.open"
```

### What `#[derive(AppEnum)]` Gives You

| Feature | Automatic |
|---------|-----------|
| DB storage | `ToDbValue` / `FromDbValue` (TEXT or INT4) |
| JSON serialization | `Serialize` / `Deserialize` (key as value) |
| OpenAPI schema | `ApiSchema` impl with correct enum values |
| Validation | `.app_enum::<OrderStatus>()` rule |
| Metadata | `FoundryAppEnum` trait (id, keys, labels, options) |
| Typed IDs | `#[foundry(id_type = PermissionId)]` generates `From<Enum>`, `as_str`, `typed_id`, `AsRef<str>`, and `Display` for string-backed enums |

### Usage in Models

```rust
#[derive(Model)]
#[foundry(table = "orders")]
struct Order {
    id: ModelId<Self>,
    status: OrderStatus,     // stored as TEXT "pending" in DB
    priority: Priority,      // stored as INT4 2 in DB
}

// Create
Order::model_create()
    .set(Order::STATUS, OrderStatus::Pending)
    .set(Order::PRIORITY, Priority::Medium)
    .execute(&*db).await?;

// Query
let pending = Order::model_query()
    .where_col(Order::STATUS, OrderStatus::Pending)
    .all(&*db).await?;
```

### Usage in Validation

```rust
#[derive(Deserialize, ApiSchema, Validate)]
struct UpdateOrderRequest {
    #[validate(required, app_enum)]
    status: OrderStatus,     // validates "pending" ✓, "invalid" ✗
}
```

### Metadata API

```rust
OrderStatus::id()                  // "order_status"
OrderStatus::keys()                // Collection<EnumKey>
OrderStatus::options()             // Collection<EnumOption> with key + label
OrderStatus::meta()                // EnumMeta { id, key_kind, options }
OrderStatus::key_kind()            // EnumKeyKind::String
OrderStatus::Pending.key()         // EnumKey::String("pending")
OrderStatus::Pending.label_key()   // "enum.order_status.pending"
OrderStatus::parse_key("shipped")  // Some(OrderStatus::Shipped)
```

---

## HasAttachments — File Attachments on Models

Attach files to any model with collection organization and image processing.

### Setup

```rust
impl HasAttachments for Product {
    fn attachable_type() -> &'static str { "products" }
    fn attachable_id(&self) -> String { self.id.to_string() }
}
```

### Collection Specs

Define specs on the model when a collection should always follow the same rules:

```rust
impl HasAttachments for Voucher {
    fn attachable_type() -> &'static str { "vouchers" }
    fn attachable_id(&self) -> String { self.id.to_string() }

    fn attachment_specs() -> Vec<AttachmentSpec<Self>> {
        vec![
            AttachmentSpec::image("main")
                .single()
                .resize_to_fill(1200, 630)
                .format(ImageFormat::WebP)
                .quality(85)
                .upscale(true)
                .hook(VoucherMainImageHook),
        ]
    }
}
```

Specs are the SSOT for a collection. Any upload to `main` applies the declared policy. Localized collections such as `main:ms` reuse the base `main` spec.

Specs can also attach lifecycle hooks:

```rust
struct VoucherMainImageHook;

#[async_trait]
impl AttachmentSpecHook<Voucher> for VoucherMainImageHook {
    async fn before_store(&self, ctx: AttachmentBeforeStoreContext<'_, Voucher>) -> Result<()> {
        // Validate or inspect ctx.file before storage.
        Ok(())
    }

    async fn after_store(&self, ctx: AttachmentAfterStoreContext<'_, Voucher>) -> Result<()> {
        // React to ctx.attachment after the DB row exists.
        Ok(())
    }
}
```

### Attaching Files

```rust
// Simple attachment
product.attach(&app, "images", uploaded_file).await?;

// Replace a single-file collection
voucher.replace_attachment(&app, "main", uploaded_file).await?;

// With image processing
Attachment::upload(uploaded_file)
    .collection("thumbnail")
    .disk("s3")
    .resize_to_fill(300, 300)
    .format(ImageFormat::WebP)
    .quality(80)
    .store(&app, "products", &product.id.to_string())
    .await?;
```

Image specs reject invalid image uploads with `invalid_attachment_image`. If `.upscale(false)` is set on a fixed resize and the uploaded image is too small, Foundry returns `attachment_image_too_small`.

Foundry also applies storage-level image decode safety limits from `[storage]` before attachment image transforms run. The defaults are intentionally generous; apps can lower them for stricter upload policies without changing `HasAttachments` or model code.

### Querying Attachments

```rust
// Single attachment (first in collection)
let avatar = user.attachment(&app, "avatar").await?;

// All in a collection
let images = product.attachments(&app, "images").await?;
for img in &images {
    println!("{} — {} ({})", img.name, img.human_size(), img.mime_type.as_deref().unwrap_or("unknown"));
}
```

### Localized Attachments

Use localized attachment helpers when an asset needs one file per available locale, such as campaign banners:

```rust
const BANNER_IMAGE: &str = "banner_image";

let locales = available_attachment_locales(&app)?;
for locale in &locales {
    if let Some(file) = files.remove(locale) {
        banner
            .replace_localized_attachment(&app, BANNER_IMAGE, locale, file)
            .await?;
    }
}

let image = banner
    .current_localized_attachment(&app, BANNER_IMAGE)
    .await?;
```

Localized helpers validate `locale` against `app.i18n()?.locale_list()`. Internally they store files in normal attachment collections using `localized_attachment_collection("banner_image", "ms")`, which resolves to `banner_image:ms`.

### Eager Loading Attachments

Use explicit eager loading when a response needs attachment data for every model in a list:

```rust
let products = Product::model_query()
    .with_attachments("images")
    .get(&app)
    .await?;

for product in &products {
    let images = product.attachments(&app, "images").await?;
    // Served from the request cache; no per-product attachment query.
}
```

Attachment eager loading also works on nested relations:

```rust
let users = User::model_query()
    .with(User::products().with_attachments("thumbnail"))
    .get(&app)
    .await?;
```

For localized assets, eager-load the concrete locale collection:

```rust
let locale = current_locale(&app);
let banners = Banner::model_query()
    .with_attachments(localized_attachment_collection("banner_image", &locale))
    .get(&app)
    .await?;
```

Foundry HTTP requests automatically run inside a model-extension cache scope. For CLI jobs, workers, or tests, wrap the full load-and-access flow when you want the same batching behavior:

```rust
app.with_model_batching(async {
    let products = Product::model_query()
        .with_attachments("images")
        .get(&app)
        .await?;

    // Access attachment helpers inside this block.
    Ok::<_, foundry::Error>(products)
}).await?;
```

If you forget to call `with_attachments(...)`, `attachment()` and `attachments()` still use a lazy batch safety net inside the active scope: the first access loads the requested collection for all known sibling models.

### Attachment Methods

```rust
let attachment: Attachment = /* ... */;

// Type checks
attachment.is_image()       // image/*
attachment.is_video()       // video/*
attachment.is_audio()       // audio/*
attachment.is_document()    // PDF, Word, Excel, etc.

// Info
attachment.extension()      // Some("jpg")
attachment.human_size()     // "2.5 MB"

// URLs
let url = attachment.url(&app).await?;
let signed = attachment.temporary_url(&app, DateTime::now().add_days(1)).await?;

// Image processing (from stored file)
let processor = attachment.image(&app).await?;
let thumb = processor.resize_to_fit(150, 150).to_bytes(ImageFormat::WebP)?;
```

### Removing Attachments

```rust
// Delete attachment + file from storage
product.detach(&app, &attachment.id).await?;

// Delete record only (keep file)
product.detach_keep_file(&app, &attachment.id).await?;

// Delete all in a collection
product.detach_all(&app, "images").await?;
```

Foundry worker maintenance audits old objects under `storage.attachment_orphan_prefix` and compares them with `attachments.disk/path`. Candidates are logged by default; deletion requires `storage.attachment_orphan_delete_enabled = true` so shared buckets are not cleaned accidentally. Operators can run the same audit with `cargo run -- attachment:orphans`.

### Collections

Organize attachments by purpose:

```rust
user.attach(&app, "avatar", avatar_file).await?;       // single avatar
user.attach(&app, "documents", id_scan).await?;         // multiple docs
product.attach(&app, "gallery", photo1).await?;          // product gallery
product.attach(&app, "gallery", photo2).await?;
```

---

## HasMetadata — Key-Value Store on Models

Attach arbitrary key-value data to any model without schema changes.

### Setup

```rust
impl HasMetadata for User {
    fn metadatable_type() -> &'static str { "users" }
    fn metadatable_id(&self) -> String { self.id.to_string() }
}
```

### Usage

```rust
// Set (upsert — creates or updates)
user.set_meta(&app, "theme", "dark").await?;
user.set_meta(&app, "preferences", json!({
    "notifications": true,
    "language": "en",
    "timezone": "Asia/Kuala_Lumpur",
})).await?;

// Get (typed deserialization)
let theme: Option<String> = user.get_meta(&app, "theme").await?;
let prefs: Option<UserPrefs> = user.get_meta(&app, "preferences").await?;

// Get as raw JSON
let raw: Option<Value> = user.get_meta_raw(&app, "preferences").await?;

// Check existence
if user.has_meta(&app, "onboarding_completed").await? {
    // ...
}

// Delete
user.forget_meta(&app, "theme").await?;

// List all metadata for this model
let all: Vec<ModelMeta> = user.all_meta(&app).await?;
for meta in &all {
    println!("{}: {:?}", meta.key, meta.value);
}
```

### Use Cases

- User preferences and settings
- Feature flags per model
- Onboarding state tracking
- Custom fields without migrations
- A/B test variant assignments

---

## HasTranslations — Multi-Locale Field Values

Store translated field values for any model across multiple locales.

> For app-level translation catalogs (UI strings, validation messages), see [i18n Guide](i18n.md). This module is for **per-model field translations** — e.g., product name in English, Malay, and Chinese.

### Setup

```rust
impl HasTranslations for Product {
    fn translatable_type() -> &'static str { "products" }
    fn translatable_id(&self) -> String { self.id.to_string() }
}
```

### Setting Translations

```rust
// Single field
product.set_translation(&app, "en", "name", "Red Shirt").await?;
product.set_translation(&app, "ms", "name", "Baju Merah").await?;
product.set_translation(&app, "zh", "name", "红色衬衫").await?;

// Multiple fields at once
product.set_translations(&app, "ms", &[
    ("name", "Baju Merah"),
    ("description", "Baju merah yang cantik"),
]).await?;
```

### Reading Translations

```rust
// Specific locale
let name_ms: Option<String> = product.translation(&app, "ms", "name").await?;

// All fields for a locale
let ms_fields: HashMap<String, String> = product.translations_for(&app, "ms").await?;
// { "name": "Baju Merah", "description": "Baju merah yang cantik" }

// Auto-resolve by current request locale
let translated = product.translated_field(&app, "name").await?;
translated.translated;          // "Baju Merah" (if current locale is "ms")
translated.get("en");           // Some("Red Shirt")
translated.get("zh");           // Some("红色衬衫")

// All translations for this model (all locales, all fields)
let all: Vec<ModelTranslation> = product.all_translations(&app).await?;
```

### Eager Loading Translations

Use explicit eager loading for translated fields or locale maps that a response will read for every model:

```rust
let products = Product::model_query()
    .with_translated_field("name")
    .with_translations_for("ms")
    .get(&app)
    .await?;

for product in &products {
    let name = product.translated_field(&app, "name").await?;
    let ms = product.translations_for(&app, "ms").await?;
}
```

Nested relation and many-to-many builders support the same methods:

```rust
let merchants = Merchant::model_query()
    .with_many_to_many(Merchant::natures().with_translated_field("name"))
    .with(Merchant::products().with_all_translations())
    .get(&app)
    .await?;
```

Like attachments, translation helpers use the active model-extension cache. Foundry HTTP requests are scoped automatically; non-HTTP code can use `app.with_model_batching(...)`. If a helper is accessed without explicit eager loading, Foundry lazily batch-loads the same access shape for all known sibling models in the scope.

### Deleting Translations

```rust
// Delete all translations for a specific locale
product.delete_translations(&app, "zh").await?;
```

### Locale Resolution

`translated_field()` resolves the "current" locale in this order:
1. Task-local `CURRENT_LOCALE` (set by request middleware)
2. i18n default locale from config
3. First available locale in the translations

---

## Countries — Reference Data

250 built-in countries with currencies, timezones, calling codes, and more.

### Seeding

Run once (or on every deploy — it's idempotent):

```bash
cargo run -- seed:countries
```

If you want the framework seeder file inside your app:

```bash
cargo run -- seed:publish
cargo build
cargo run -- db:seed --id 000000000001_countries_seeder
```

Or programmatically:

```rust
let count = seed_countries(&app).await?;  // 250 upserted
```

### Querying

```rust
// Find by ISO2 code
let malaysia = Country::find(&app, "MY").await?;
if let Some(country) = malaysia {
    println!("{} {}", country.flag_emoji.unwrap_or_default(), country.name);
    // 🇲🇾 Malaysia
    println!("Capital: {}", country.capital.unwrap_or_default());
    println!("Currency: {}", country.primary_currency_code.unwrap_or_default());
    println!("Calling: {}", country.calling_code.unwrap_or_default());
}

// All countries
let all = Country::all(&app).await?;

// Only enabled countries (for dropdowns)
let enabled = Country::enabled(&app).await?;

// Filter by status (typed enum)
let disabled = Country::by_status(&app, CountryStatus::Disabled).await?;

// Check existence
if Country::exists(&app, "US").await? {
    // ...
}
```

### Country Fields

| Field | Type | Example |
|-------|------|---------|
| `iso2` | String (PK) | `"MY"` |
| `iso3` | String | `"MYS"` |
| `name` | String | `"Malaysia"` |
| `official_name` | Option | `"Malaysia"` |
| `capital` | Option | `"Kuala Lumpur"` |
| `region` | Option | `"Asia"` |
| `subregion` | Option | `"South-Eastern Asia"` |
| `primary_currency_code` | Option | `"MYR"` |
| `currencies` | JSON array | `[{"code":"MYR","name":"Malaysian ringgit","symbol":"RM"}]` |
| `calling_code` | Option | `"+60"` |
| `timezones` | JSON array | `["Asia/Kuala_Lumpur"]` |
| `latitude` / `longitude` | Option\<f64\> | `2.5` / `112.5` |
| `flag_emoji` | Option | `"🇲🇾"` |
| `status` | `CountryStatus` | `CountryStatus::Enabled` or `CountryStatus::Disabled` |
| `is_default` | `bool` | `false` |
| `conversion_rate` | Option\<f64\> | `4.47` (relative to base currency) |

### Common Patterns

**Country dropdown:**

```rust
async fn country_options(State(app): State<AppContext>) -> impl IntoResponse {
    let countries = Country::enabled(&app).await?;
    Json(countries.iter().map(|c| json!({
        "value": c.iso2,
        "label": c.name,
        "flag": c.flag_emoji,
    })).collect::<Vec<_>>())
}
```

**Validate country code:**

```rust
validator
    .field("country", &input.country)
    .required()
    .exists("countries", "iso2")    // DB check
    .apply()
    .await?;
```

---

## Settings — Admin-Ready Key-Value Store

A typed key-value store with form metadata, designed for admin panel CRUD. Each setting carries its input type, validation parameters, grouping, and display information so the frontend can dynamically render forms.

### Creating Settings (Seeder / Setup)

Use `NewSetting` builder to define settings with full metadata:

```rust
use foundry::settings::{NewSetting, Setting, SettingType};

// Text input
Setting::create(&app, NewSetting::new("app.name", "Application Name")
    .value(json!("My App"))
    .setting_type(SettingType::Text)
    .parameters(json!({"max_length": 255, "placeholder": "Enter app name"}))
    .group("general")
    .description("Displayed in browser title and emails")
    .sort_order(1)
    .is_public(true)
).await?;

// Boolean toggle
Setting::create(&app, NewSetting::new("app.maintenance", "Maintenance Mode")
    .value(json!(false))
    .setting_type(SettingType::Boolean)
    .group("general")
    .sort_order(2)
).await?;

// Select dropdown
Setting::create(&app, NewSetting::new("app.theme", "Theme")
    .value(json!("light"))
    .setting_type(SettingType::Select)
    .parameters(json!({"options": [
        {"value": "light", "label": "Light"},
        {"value": "dark", "label": "Dark"},
        {"value": "auto", "label": "System"}
    ]}))
    .group("appearance")
    .sort_order(1)
    .is_public(true)
).await?;

// Number input
Setting::create(&app, NewSetting::new("upload.max_size_kb", "Max Upload Size (KB)")
    .value(json!(5120))
    .setting_type(SettingType::Number)
    .parameters(json!({"min": 512, "max": 102400, "step": 512}))
    .group("uploads")
    .sort_order(1)
).await?;

// Image upload
Setting::create(&app, NewSetting::new("app.logo", "Site Logo")
    .setting_type(SettingType::Image)
    .parameters(json!({
        "allowed_mimes": ["image/png", "image/jpeg", "image/svg+xml"],
        "max_size_kb": 2048,
        "max_width": 512,
        "max_height": 512
    }))
    .group("appearance")
    .sort_order(2)
).await?;

// Email input
Setting::create(&app, NewSetting::new("mail.from_address", "From Address")
    .value(json!("hello@example.com"))
    .setting_type(SettingType::Email)
    .group("mail")
    .sort_order(1)
).await?;

// Code editor
Setting::create(&app, NewSetting::new("app.custom_css", "Custom CSS")
    .value(json!(""))
    .setting_type(SettingType::Code)
    .parameters(json!({"language": "css"}))
    .group("appearance")
    .sort_order(10)
).await?;
```

### Reading Values

```rust
// Quick value access (most common)
let name = Setting::get(&app, "app.name").await?;         // Option<Value>
let theme = Setting::get_or(&app, "app.theme", json!("light")).await?;

// Typed access
let maintenance: Option<bool> = Setting::get_as(&app, "app.maintenance").await?;
let max_kb: Option<i64> = Setting::get_as(&app, "upload.max_size_kb").await?;

// Full setting record (includes metadata — for admin detail view)
let setting = Setting::find(&app, "app.name").await?;
```

### Updating Values

```rust
// Update an existing setting's value
Setting::set(&app, "app.name", json!("New Name")).await?;

// Upsert — creates with defaults if key doesn't exist, updates value if it does
Setting::upsert(&app, "app.name", json!("New Name")).await?;

// Delete
Setting::remove(&app, "app.name").await?;
```

### Admin Panel Queries

```rust
// All settings grouped and sorted (admin list page)
let all = Setting::all(&app).await?;

// Settings for a specific group (admin tab/section)
let mail = Setting::by_group(&app, "mail").await?;

// All distinct group names (admin sidebar/tabs)
let groups = Setting::groups(&app).await?;
// ["appearance", "general", "mail", "uploads"]

// Public settings only (safe for frontend API)
let public = Setting::public(&app).await?;

// By key prefix
let app_settings = Setting::by_prefix(&app, "app.").await?;
```

### Setting Types (`SettingType` enum)

| Type | Form Widget | Parameters |
|------|-------------|------------|
| `Text` | Single-line input | `max_length`, `placeholder` |
| `Textarea` | Multi-line input | `max_length`, `rows` |
| `Number` | Numeric input | `min`, `max`, `step` |
| `Boolean` | Toggle/checkbox | — |
| `Select` | Dropdown | `options: [{value, label}]` |
| `Multiselect` | Multi-select | `options: [{value, label}]` |
| `Email` | Email input | — |
| `Url` | URL input | — |
| `Color` | Color picker | — |
| `Date` | Date picker | — |
| `Datetime` | Datetime picker | — |
| `File` | File upload | `allowed_mimes`, `max_size_kb` |
| `Image` | Image upload | `allowed_mimes`, `max_size_kb`, `max_width`, `max_height` |
| `Json` | JSON editor | — |
| `Password` | Masked input | Presentation only; do not store credentials or secrets |
| `Code` | Code editor | `language` |

`SettingType::Password` only selects a masked admin widget. Settings values are JSON and are not
encrypted automatically. Keep credentials in your deployment's secret manager, or explicitly
encrypt application-owned sensitive values with `app.crypt()` before persistence. Never mark a
sensitive setting public.

### Setting Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Auto-generated primary key |
| `key` | String | Unique setting key (dot-notation recommended) |
| `value` | Option\<JSON\> | The stored value (any JSON type) |
| `setting_type` | `SettingType` | Input widget type for admin forms |
| `parameters` | JSON | Constraints and options for the input widget |
| `group_name` | String | Admin panel section/tab grouping |
| `label` | String | Human-readable display name |
| `description` | Option\<String\> | Help text shown below the input |
| `sort_order` | i32 | Ordering within a group |
| `is_public` | bool | Whether exposed to unauthenticated API |
| `created_at` | Timestamp | Creation time |
| `updated_at` | Option\<Timestamp\> | Last update time |

### Common Patterns

**Admin settings API (list + update):**

```rust
// GET /admin/settings — list all settings grouped
async fn list_settings(State(app): State<AppContext>) -> impl IntoResponse {
    let groups = Setting::groups(&app).await?;
    let mut result = json!({});
    for group in groups {
        let settings = Setting::by_group(&app, &group).await?;
        result[&group] = json!(settings);
    }
    Json(result)
}

// PUT /admin/settings/:key — update a setting value
async fn update_setting(
    State(app): State<AppContext>,
    Path(key): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    Setting::set(&app, &key, body["value"].clone()).await?;
    StatusCode::NO_CONTENT
}
```

**Public settings API (for frontend config):**

```rust
// GET /api/settings — only public settings
async fn public_settings(State(app): State<AppContext>) -> impl IntoResponse {
    let settings = Setting::public(&app).await?;
    Json(settings.iter().map(|s| json!({
        "key": s.key,
        "value": s.value,
    })).collect::<Vec<_>>())
}
```

**Feature flags:**

```rust
let enabled = Setting::get_as::<bool>(&app, "feature.new_dashboard")
    .await?
    .unwrap_or(false);
```

---

## Summary

| Extension | Trait | What you add to your model | Storage |
|-----------|-------|---------------------------|---------|
| AppEnum | `FoundryAppEnum` (derive) | Typed enum field | TEXT or INT4 column |
| Attachments | `HasAttachments` | File uploads | `attachments` table + storage disk |
| Metadata | `HasMetadata` | Key-value pairs | `metadata` table |
| Translations | `HasTranslations` | Multi-locale fields | `model_translations` table |
| Countries | (static methods) | Reference data | `countries` table |
| Settings | (static methods) | Admin-ready key-value store | `settings` table |

All except Countries and Settings use polymorphic tables — one table serves all models via `type` + `id` columns. No per-model migrations needed.

### Required Migrations

Run `cargo run -- migrate:publish` to get the framework migration files:

```
000000000005_create_attachments.rs
000000000006_create_metadata.rs
000000000007_create_model_translations.rs
000000000008_create_countries.rs
000000000009_create_settings.rs
```

Run `cargo run -- seed:publish` if you also want the framework countries seeder in your app:

```text
000000000001_countries_seeder.rs
```
