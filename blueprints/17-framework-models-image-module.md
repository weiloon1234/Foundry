# Framework-Provided Models, Image Module & always_with

> **Status:** ✅ Complete
> **Created:** 2026-04-13
> **Completed:** 2026-04-13
> **Purpose:** Add 3 polymorphic framework models (attachments, metadata, translations), a full image processing module, `always_with` derive prep, HasToken trait, and migrate:publish CLI.

---

# What Already Exists

- `#[derive(Model)]` with column constants, lifecycle hooks, `Loaded<T>` for relations
- `has_many`, `has_one`, `belongs_to`, `many_to_many` relation constructors — all explicit via `.with()`
- `StorageManager` with local + S3 adapters, `UploadedFile::store()`, `StoredFile` metadata
- `image` crate 0.25 in Cargo.toml (used only for dimension reading in validation)
- `I18nManager` with filesystem-discovered locales, `Accept-Language` resolution, `t!` macro
- `Locale` extension set per-request, `I18n` extractor in handlers
- `ModelLifecycle` trait with creating/created/updating/updated/deleting/deleted hooks
- No polymorphic `_type`/`_id` pattern exists yet — this is new
- No auto-eager-loading mechanism — all `.with()` calls are explicit

---

# Phase 1: Image Module

Full image processing pipeline for resize, crop, convert, watermark, and more.

## ImageProcessor API

```rust
pub struct ImageProcessor {
    inner: DynamicImage,
    quality: u8,          // default: 85
    format: Option<ImageFormat>,
}

impl ImageProcessor {
    // --- Load ---
    pub fn open(path: impl AsRef<Path>) -> Result<Self>
    pub fn from_bytes(bytes: &[u8]) -> Result<Self>
    pub fn from_uploaded(file: &UploadedFile) -> Result<Self>

    // --- Transform (chainable) ---
    pub fn resize(self, width: u32, height: u32) -> Self
    pub fn resize_to_fit(self, max_width: u32, max_height: u32) -> Self
    pub fn resize_to_fill(self, width: u32, height: u32) -> Self
    pub fn crop(self, x: u32, y: u32, width: u32, height: u32) -> Self
    pub fn quality(self, quality: u8) -> Self
    pub fn blur(self, sigma: f32) -> Self
    pub fn grayscale(self) -> Self
    pub fn rotate(self, rotation: Rotation) -> Self
    pub fn flip_horizontal(self) -> Self
    pub fn flip_vertical(self) -> Self
    pub fn brightness(self, value: i32) -> Self
    pub fn contrast(self, value: f32) -> Self
    pub fn watermark_text(self, text: &str, options: WatermarkOptions) -> Self

    // --- Output ---
    pub fn save(self, path: impl AsRef<Path>) -> Result<()>
    pub fn save_as(self, path: impl AsRef<Path>, format: ImageFormat) -> Result<()>
    pub fn to_bytes(self, format: ImageFormat) -> Result<Vec<u8>>

    // --- Info ---
    pub fn width(&self) -> u32
    pub fn height(&self) -> u32
    pub fn format(&self) -> Option<ImageFormat>
}
```

## Format Support

JPEG, PNG, WebP, GIF, BMP, TIFF, AVIF, ICO — all via `image` crate 0.25 defaults.

```rust
pub enum ImageFormat { Jpeg, Png, WebP, Gif, Bmp, Tiff, Avif, Ico }
pub enum Rotation { Deg90, Deg180, Deg270 }
pub enum WatermarkPosition { TopLeft, TopRight, BottomLeft, BottomRight, Center }

pub struct WatermarkOptions {
    pub position: WatermarkPosition,
    pub color: [u8; 4],     // RGBA
    pub font_size: f32,
    pub opacity: f32,       // 0.0 - 1.0
}
```

## DX

```rust
ImageProcessor::open("photo.jpg")?
    .resize_to_fit(1200, 800)
    .quality(80)
    .watermark_text("(c) 2026", WatermarkOptions::bottom_right())
    .save_as("photo_web.webp", ImageFormat::WebP)?;

// From upload:
let processed = ImageProcessor::from_uploaded(&file)?
    .resize_to_fill(400, 400)
    .grayscale()
    .to_bytes(ImageFormat::Png)?;
```

## Dependencies

- `image = "0.25"` (already in Cargo.toml)
- `imageproc = "0.25"` (new — for text rendering on images)
- `rusttype` or `ab_glyph` (transitive via imageproc — for font loading)

## Files

- `src/image/mod.rs` — `ImageProcessor`, `ImageFormat`, `Rotation`, `WatermarkOptions`
- `src/image/watermark.rs` — text overlay implementation
- `Cargo.toml` — add `imageproc`

---

# Phase 2: `always_with` Derive Attribute

Add auto-eager-loading to model queries. When `#[foundry(always_with = "...")]` is set, specified relations load automatically without explicit `.with()`.

## Derive Macro Change

```rust
#[derive(Model)]
#[foundry(model = "products", always_with = "translations,category")]
struct Product {
    id: ModelId<Self>,
    name: String,
    translations: Loaded<Vec<ModelTranslation>>,
    category: Loaded<Option<Category>>,
}
```

The macro generates:
```rust
impl Product {
    pub fn default_with_relations(query: ModelQuery<Self>) -> ModelQuery<Self> {
        query
            .with(Self::translations())
            .with(Self::category())
    }
}
```

And `ModelQuery::new()` calls `M::default_with_relations()` unless opted out.

## Opt-Out

```rust
Product::query().without_defaults().get(&app).await?;
```

## Changes

- `foundry-macros/src/model.rs` — parse `always_with` attribute, generate `default_with_relations` method
- `src/database/model.rs` — add `fn apply_default_relations(query: ModelQuery<Self>) -> ModelQuery<Self>` to `Model` trait with default no-op impl
- `src/database/query.rs` — add `without_defaults: bool` field to `ModelQuery`, skip auto-relations when true. Add `without_defaults()` method

---

# Phase 3: Attachments

Polymorphic file attachment table. Any model can have files (avatars, invoices, galleries).

## Migration

```sql
CREATE TABLE IF NOT EXISTS attachments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    attachable_type TEXT NOT NULL,
    attachable_id UUID NOT NULL,
    collection TEXT NOT NULL DEFAULT 'default',
    disk TEXT NOT NULL,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    original_name TEXT,
    mime_type TEXT,
    size BIGINT NOT NULL DEFAULT 0,
    sort_order INT NOT NULL DEFAULT 0,
    custom_properties JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ
);
CREATE INDEX idx_attachments_poly ON attachments (attachable_type, attachable_id, collection);
```

## Attachment Model

```rust
#[derive(Model)]
#[foundry(model = "attachments")]
pub struct Attachment { ... }
```

Fields: id, attachable_type, attachable_id, collection, disk, path, name, original_name, mime_type, size, sort_order, custom_properties, created_at, updated_at.

## Useful Aliases

```rust
impl Attachment {
    pub fn url(&self, app: &AppContext) -> Result<String>
    pub fn temporary_url(&self, app: &AppContext, ttl: Duration) -> Result<String>
    pub fn is_image(&self) -> bool
    pub fn is_video(&self) -> bool
    pub fn is_audio(&self) -> bool
    pub fn is_document(&self) -> bool
    pub fn extension(&self) -> Option<&str>
    pub fn human_size(&self) -> String                    // "2.5 MB"
    pub fn image(&self, app: &AppContext) -> Result<ImageProcessor>
}
```

## Upload Pipeline Builder

```rust
Attachment::upload(uploaded_file)
    .collection("avatar")
    .disk("s3")
    .resize(800, 600)
    .quality(80)
    .store(&app, "users", &user.id)
    .await?;
```

The pipeline builder:
1. If the file is an image and transform methods are called → processes via `ImageProcessor`
2. Stores the (possibly transformed) file via `StorageManager`
3. Creates the `Attachment` DB record with all metadata

## HasAttachments Trait

```rust
pub trait HasAttachments: Model {
    fn attachable_type() -> &'static str;

    async fn attach(&self, app: &AppContext, collection: &str, file: UploadedFile) -> Result<Attachment>
    async fn attachment(&self, app: &AppContext, collection: &str) -> Result<Option<Attachment>>
    async fn attachments(&self, app: &AppContext, collection: &str) -> Result<Vec<Attachment>>
    async fn detach(&self, app: &AppContext, attachment_id: &ModelId<Attachment>) -> Result<()>
    async fn detach_keep_file(&self, app: &AppContext, attachment_id: &ModelId<Attachment>) -> Result<()>
    async fn detach_all(&self, app: &AppContext, collection: &str) -> Result<u64>
}
```

## Lifecycle: Auto-Delete Files

- Default: when an `Attachment` record is deleted, the file is auto-deleted from storage
- Opt-out: `detach_keep_file()` deletes the DB record but preserves the file
- Implemented via `AttachmentLifecycle::deleted` hook

## Files

- `src/attachments/mod.rs` — Attachment model, HasAttachments trait, aliases
- `src/attachments/upload.rs` — AttachmentUploadBuilder
- `database/migrations/202604131000_create_attachments.rs`

---

# Phase 4: Metadata

Polymorphic key-value store. Any model can have arbitrary flexible data.

## Migration

```sql
CREATE TABLE IF NOT EXISTS metadata (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    metadatable_type TEXT NOT NULL,
    metadatable_id UUID NOT NULL,
    key TEXT NOT NULL,
    value JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX idx_metadata_unique ON metadata (metadatable_type, metadatable_id, key);
```

## HasMetadata Trait

```rust
pub trait HasMetadata: Model {
    fn metadatable_type() -> &'static str;

    async fn set_meta(&self, app: &AppContext, key: &str, value: impl Serialize) -> Result<()>
    async fn get_meta<T: DeserializeOwned>(&self, app: &AppContext, key: &str) -> Result<Option<T>>
    async fn get_meta_raw(&self, app: &AppContext, key: &str) -> Result<Option<serde_json::Value>>
    async fn forget_meta(&self, app: &AppContext, key: &str) -> Result<bool>
    async fn all_meta(&self, app: &AppContext) -> Result<Vec<ModelMeta>>
    async fn has_meta(&self, app: &AppContext, key: &str) -> Result<bool>
}
```

## DX

```rust
user.set_meta(&app, "theme", json!("dark")).await?;
let theme: String = user.get_meta(&app, "theme").await?.unwrap();
user.has_meta(&app, "theme").await?;  // true
user.forget_meta(&app, "theme").await?;
```

## Files

- `src/metadata/mod.rs` — ModelMeta model, HasMetadata trait
- `database/migrations/202604131001_create_metadata.rs`

---

# Phase 5: Translations (Locales)

Polymorphic translation table. Any model can have translated field values per locale.

## Migration

```sql
CREATE TABLE IF NOT EXISTS model_translations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    translatable_type TEXT NOT NULL,
    translatable_id UUID NOT NULL,
    locale TEXT NOT NULL,
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX idx_translations_unique ON model_translations (translatable_type, translatable_id, locale, field);
CREATE INDEX idx_translations_lookup ON model_translations (translatable_type, translatable_id, locale);
```

## TranslatedFields Struct

When translations are eager-loaded, each translatable field resolves to:

```rust
pub struct TranslatedFields {
    /// All locale values: {"en": "Red Shirt", "zh": "红色衬衫"}
    pub values: HashMap<String, String>,
    /// The translation for the current request locale (with fallback)
    pub translated: String,
}
```

`translated` is resolved using:
1. Current request locale (via `tokio::task_local! CURRENT_LOCALE`)
2. `I18nManager::default_locale()` fallback
3. First available translation as last resort

## Request Locale Propagation

```rust
// In src/i18n/mod.rs:
tokio::task_local! {
    pub static CURRENT_LOCALE: String;
}
```

Set automatically in `request_context_middleware`:
```rust
let locale = resolve_locale(&request, &app);
CURRENT_LOCALE.scope(locale, async { next.run(request).await }).await
```

- Inside HTTP request → uses the request's locale
- Inside jobs/CLI → falls back to `I18nManager::default_locale()`
- Fully automatic — no manual passing needed

## HasTranslations Trait

```rust
pub trait HasTranslations: Model {
    fn translatable_type() -> &'static str;
    fn translatable_fields() -> &'static [&'static str];

    async fn set_translation(&self, app: &AppContext, locale: &str, field: &str, value: &str) -> Result<()>
    async fn set_translations(&self, app: &AppContext, locale: &str, values: &[(&str, &str)]) -> Result<()>
    async fn translation(&self, app: &AppContext, locale: &str, field: &str) -> Result<Option<String>>
    async fn translations_for(&self, app: &AppContext, locale: &str) -> Result<HashMap<String, String>>
    async fn all_translations(&self, app: &AppContext) -> Result<Vec<ModelTranslation>>
    async fn delete_translations(&self, app: &AppContext, locale: &str) -> Result<u64>
}
```

## Auto-Eager-Loading

Translations use `always_with` by default:

```rust
#[derive(Model)]
#[foundry(model = "products", always_with = "translations")]
struct Product {
    id: ModelId<Self>,
    name: String,
    translations: Loaded<ProductTranslations>,
}

// Auto-loaded — no explicit .with():
let product = Product::query().first(&app).await?.unwrap();
product.translations.loaded().name.translated  // current locale
product.translations.loaded().name.values["zh"] // specific locale

// Opt-out:
let product = Product::query().without_defaults().first(&app).await?;
```

## Locale Validation

`set_translation` validates against `I18nManager::locale_list()` — only locales with filesystem translation files are valid. This ensures the translations table stays in sync with the i18n system.

## Files

- `src/translations/mod.rs` — ModelTranslation, TranslatedFields, HasTranslations
- `src/i18n/mod.rs` — `CURRENT_LOCALE` task_local
- `src/logging/middleware.rs` — set CURRENT_LOCALE in request middleware
- `database/migrations/202604131002_create_model_translations.rs`

---

# Implementation Order

| Phase | Item | Depends On | Complexity | Status |
|-------|------|-----------|-----------|--------|
| 1 | Image module | — | M | ✅ Done — `src/imaging/mod.rs`, 13 tests |
| 2 | always_with derive prep | — | S | Deferred — inert `without_defaults()` removed by completeness audit |
| 3 | Attachments | Phase 1 | L | ✅ Done — `src/attachments/mod.rs`, HasAttachments, upload pipeline |
| 4 | Metadata | — | S | ✅ Done — `src/metadata/mod.rs`, HasMetadata |
| 5 | Translations | — | M | ✅ Done — `src/translations/mod.rs`, HasTranslations, CURRENT_LOCALE task_local |
| 6 | HasToken trait | — | S | ✅ Done — `src/auth/token.rs`, create_token/revoke on Authenticatable models |
| 7 | migrate:publish CLI | — | S | ✅ Done — publishes framework Rust migrations |
| 8 | Countries table + model + seeder | — | M | ✅ Done — 250 countries, iso2 PK, seed:countries CLI |

---

# Additional Features (not in original blueprint)

## HasToken Trait

Laravel-style `HasApiTokens` for Authenticatable models:
```rust
impl HasToken for User {
    fn token_actor_id(&self) -> String { self.id.to_string() }
}

user.create_token(&app).await?;
user.create_token_named(&app, "My iPhone").await?;
user.create_token_with_abilities(&app, "ci", vec!["deploy:read".into()]).await?;
user.revoke_all_tokens(&app).await?;
```

**File:** `src/auth/token.rs`

## migrate:publish CLI Command

Publishes framework Rust migrations to the consumer project:
```bash
foundry migrate:publish              # framework migrations
foundry migrate:publish --force      # overwrite existing
foundry migrate:publish --path=db/   # custom directory
```

Tables: personal_access_tokens, password_reset_tokens, notifications, job_history, attachments, metadata, model_translations.

**File:** `src/config/publish.rs`

## Countries Table + Model + Seeder

Framework-provided countries table with 250 countries from ISO 3166-1. Primary key is `iso2` (CHAR(2)), not UUID.

```rust
// Query:
let my = Country::find(&app, "MY").await?;     // by ISO2
let all = Country::all(&app).await?;            // all 250
let enabled = Country::enabled(&app).await?;    // status = "enabled"
let exists = Country::exists(&app, "US").await?; // check existence
```

CLI commands:
```bash
foundry migrate:publish   # includes countries migrations
foundry seed:publish      # publishes 000000000001_countries_seeder.rs
foundry seed:countries    # seeds 250 countries (upsert, safe to re-run)
```

**Files:** `src/countries/mod.rs`, `src/countries/seed.json` (250 countries), `src/config/publish.rs`

---

# Assumptions

- Polymorphic `_type` column stores the table name (e.g., `"users"`, `"products"`)
- Polymorphic `_id` column stores the model's primary key as UUID
- `HasAttachments`, `HasMetadata`, `HasTranslations` are async traits the consumer implements
- Attachment file cleanup on `detach()` is default — use `detach_keep_file()` to opt out
- `tokio::task_local!` propagates request locale to translation eager-loading automatically
- `always_with` derive attribute remains deferred; its unused `without_defaults()` preparatory switch was removed until the feature has a complete typed contract
- Image module uses `image` crate only (no `imageproc`) — watermark text deferred
