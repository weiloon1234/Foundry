# Storage & Imaging Guide

File storage with local + S3 backends, multipart uploads, and a chainable image processing pipeline.

---

## Quick Start

```rust
// Upload a file from a handler
async fn upload(State(app): State<AppContext>, file: UploadedFile) -> Result<impl IntoResponse> {
    let stored = file.store_and_cleanup(&app, "avatars").await?;
    Ok(Json(json!({ "url": stored.url })))
}
```

---

## Config

```toml
# config/storage.toml
[storage]
default = "local"
max_upload_size_bytes = 0            # total file bytes per multipart request; 0 = no storage-level cap
max_upload_file_size_bytes = 0       # per-file cap; 0 = no storage-level cap
max_upload_files = 0                 # max files per multipart request; 0 = no storage-level cap
upload_temp_retention_seconds = 3600 # worker cleanup age for foundry-upload-* temp files; 0 = keep forever
upload_temp_prune_interval_ms = 3600000
upload_temp_prune_batch_size = 1000
image_max_input_bytes = 52428800    # max image bytes decoded by attachments; 0 = disabled
image_max_pixels = 50000000         # max decoded image pixels; 0 = disabled
image_max_width = 12000             # max decoded image width; 0 = disabled
image_max_height = 12000            # max decoded image height; 0 = disabled
attachment_orphan_audit_enabled = true
attachment_orphan_delete_enabled = false
attachment_orphan_retention_seconds = 604800
attachment_orphan_prune_interval_ms = 3600000
attachment_orphan_prune_batch_size = 100
attachment_orphan_prefix = "attachments/"

[storage.disks.local]
driver = "local"
root = "storage/app"
url = "/storage"                    # public URL prefix
visibility = "private"              # "public" or "private"

[storage.disks.s3]
driver = "s3"
bucket = "my-bucket"
region = "ap-southeast-1"
key = "AKIA..."
secret = "..."
# endpoint = "https://..."         # custom endpoint for MinIO, R2, etc.
# url = "https://cdn.example.com"  # public URL prefix
# use_path_style = false
visibility = "public"
```

For S3 disks, `visibility` describes the disk's application-level access intent; Foundry does not translate it into an `x-amz-acl` object ACL. New AWS buckets disable ACLs by default, and S3-compatible providers such as Cloudflare R2 do not support that header. Configure public access with the bucket policy, public bucket/custom-domain settings, or CDN represented by `url`. This keeps uploads compatible with [AWS bucket-owner-enforced object ownership](https://docs.aws.amazon.com/AmazonS3/latest/userguide/about-object-ownership.html) and [Cloudflare R2's S3 API](https://developers.cloudflare.com/r2/api/s3/api/). When `put_file` supplies a content type, the S3 adapter stores it as the object's `Content-Type` metadata.

Upload caps are storage-level guardrails for `UploadedFile`, `MultipartForm`, and derive-generated multipart DTOs. Route-level validators and file rules still own product-specific limits such as avatar size, gallery count, and allowed MIME types.

Foundry streams multipart files to OS temp files named `foundry-upload-*`. Failed multipart extraction removes files that Foundry already created. For a final store, the consuming `store*_and_cleanup` methods remove the Foundry-owned temp file immediately on both success and failure. The borrowed `store*` methods intentionally retain it for workflows that read, transform, attach, or store the upload more than once; call `remove_uploaded_temp_file` when that work finishes. Worker maintenance prunes any stale remainder using the retention settings above, so consumers do not need to add a scheduler job. Stored attachments/files are not pruned by this temp cleanup.

Uploaded filenames are metadata, not trusted paths. Foundry strips Unix/Windows path components, removes control characters, trims unsafe wrapper whitespace/quotes, caps display filename length, and falls back to `upload` when no safe name remains. Generated storage names stay UUID-based and only preserve a sanitized extension.

File validation helpers prefer magic-byte MIME detection and do not trust arbitrary client `Content-Type` headers for binary formats like images or PDFs. Text-like uploads keep a compatibility fallback for safe MIME types such as `text/plain`, after checking that sampled bytes look like text.

Storage paths are logical relative keys, not filesystem paths. Foundry rejects absolute paths, `..` or `.` segments, empty path segments, backslashes, drive prefixes, and control characters before calling a disk adapter. Local disks also reject symlinked path components so app storage cannot escape the configured disk root.

Attachment image processing has generous decode guardrails by default. `image_max_input_bytes` rejects large image uploads before decode, and the width/height/pixel caps reject suspicious image dimensions before transforms. Set an individual value to `0` only when the app has its own stricter validation or explicitly needs to disable that one safety check.

Foundry can audit attachment storage objects under `attachment_orphan_prefix` on list-capable disks (`local` and `s3`). The worker compares listed objects against `attachments.disk/path` and logs candidates older than `attachment_orphan_retention_seconds`. Deletion is off by default; enable `attachment_orphan_delete_enabled` only when the app owns that prefix in the bucket/disk. Consumers do not need to add a scheduler job.

---

## Handling Uploads

### MultipartForm

Extract files and text fields from multipart requests:

```rust
async fn create_post(
    State(app): State<AppContext>,
    form: MultipartForm,
) -> Result<impl IntoResponse> {
    let title = form.text("title").unwrap_or("Untitled");
    // Clone the metadata handle because MultipartForm exposes borrowed files.
    // This endpoint does not need the temporary bytes after this store.
    let cover = form.file("cover")?.clone();

    let stored = cover
        .store_and_cleanup(&app, "posts/covers")
        .await?;

    Ok(Json(json!({
        "title": title,
        "cover_url": stored.url,
        "cover_size": stored.size,
    })))
}
```

### UploadedFile Methods

```rust
let file: UploadedFile = uploaded_file;

// Final use: consume the handle and clean up Foundry's temp file after
// either a successful or failed storage attempt.
let stored = file.store_and_cleanup(&app, "avatars").await?;
// → avatars/01912a4b-7c8d-7000-abcd-ef1234567890.jpg
```

The consuming variants mirror every storage target:

- `store_and_cleanup(app, dir)`
- `store_as_and_cleanup(app, dir, name)`
- `store_on_and_cleanup(app, disk, dir)`
- `store_as_on_and_cleanup(app, disk, dir, name)`

Use the borrowed variants when the same temporary file is still needed:

```rust
let file: &UploadedFile = form.file("avatar")?;
let original = file.store(&app, "avatars").await?;
let archive = file.store_on(&app, "s3", "archive").await?;
remove_uploaded_temp_file(file).await;

// File metadata
file.original_name;    // Option<String> — "photo.jpg"
file.content_type;     // Option<String> — "image/jpeg"
file.size;             // u64 — bytes
file.original_extension(); // Option<String> — "jpg"
```

Consuming cleanup only deletes paths created inside Foundry's private upload temp directory; manually constructed `UploadedFile` values that point elsewhere are never deleted. If storage and cleanup both fail, the storage error remains the returned error. If storage succeeds but cleanup fails, the method returns the cleanup error and the stored object may already exist.

`content_type` is client-supplied metadata. Validation helpers such as `is_image` and `allowed_mimes` inspect file bytes first when possible.

### Multiple File Uploads

```rust
let files = form.files("documents");  // &[UploadedFile]
for file in files.iter().cloned() {
    file.store_and_cleanup(&app, "documents").await?;
}
```

### StoredFile Result

Every store operation returns a `StoredFile`:

```rust
pub struct StoredFile {
    pub disk: String,              // "local" or "s3"
    pub path: String,              // "avatars/uuid.jpg"
    pub name: String,              // "uuid.jpg"
    pub size: u64,                 // bytes
    pub content_type: Option<String>, // "image/jpeg"
    pub url: Option<String>,       // public URL if available
}
```

---

## Storage Manager

For direct file operations (not from uploads):

```rust
let storage = app.storage()?;

// Write bytes
storage.put("data/report.json", serde_json::to_vec(&report)?).await?;

// Read bytes
let bytes = storage.get("data/report.json").await?;

// Check existence
if storage.exists("data/report.json").await? {
    // file exists
}

// Delete
storage.delete("data/old-report.json").await?;

// Copy / Move
storage.copy("data/report.json", "backups/report.json").await?;
storage.move_to("temp/upload.csv", "data/import.csv").await?;

// Get URL
let url = storage.url("avatars/profile.jpg")?;

// Temporary URL (signed, for private S3 files)
let url = storage.temporary_url("documents/contract.pdf", DateTime::now().add_days(1)).await?;
```

Attachment detach deletes database rows before best-effort file deletion. If storage deletion fails, Foundry avoids leaving a visible attachment record pointing at a missing file; operators can clean any leftover storage object separately.

Run an operator audit manually with:

```bash
cargo run -- attachment:orphans
cargo run -- attachment:orphans --json --disk s3 --limit 100
cargo run -- attachment:orphans --delete
```

`--delete` requires `storage.attachment_orphan_delete_enabled = true`. Custom storage drivers compile without listing support; they can opt in by implementing `StorageAdapter::list_prefix`.

### Working with Specific Disks

```rust
let storage = app.storage()?;

// Default disk (configured in [storage] default = "local")
let local = storage.default_disk()?;

// Named disk
let s3 = storage.disk("s3")?;
s3.put("exports/data.csv", csv_bytes).await?;
let url = s3.url("exports/data.csv")?;

// List configured disks
let disks = storage.configured_disks();  // ["local", "s3"]

let objects = storage.disk("s3")?.list_prefix("attachments/", 100).await?;
```

---

## Image Processing

Chainable pipeline for transforming images. Works with files from disk or raw bytes.

### Opening Images

```rust
use foundry::imaging::{ImageProcessor, ImageFormat, Rotation};

// From file path
let img = ImageProcessor::open("uploads/photo.jpg")?;

// From bytes (e.g., from storage)
let bytes = app.storage()?.get("avatars/profile.jpg").await?;
let img = ImageProcessor::from_bytes(&bytes)?;

// Check dimensions
println!("{}x{}", img.width(), img.height());
```

### Transformations

All methods return `Self` for chaining:

```rust
let result = ImageProcessor::open("photo.jpg")?
    .resize(800, 600)              // exact dimensions (stretches)
    .resize_to_fit(800, 600)       // fit within bounds (preserves aspect ratio)
    .resize_to_fill(800, 600)      // fill bounds (crops excess)
    .crop(10, 10, 200, 200)        // crop region (x, y, width, height)
    .rotate(Rotation::Deg90)       // rotate 90°, 180°, or 270°
    .flip_horizontal()             // mirror horizontally
    .flip_vertical()               // mirror vertically
    .grayscale()                   // convert to grayscale
    .blur(2.0)                     // Gaussian blur (sigma)
    .brightness(20)                // adjust brightness (-255 to +255)
    .contrast(1.5)                 // adjust contrast
    .quality(85)                   // JPEG/WebP quality (1-100)
    .to_bytes(ImageFormat::Jpeg)?; // output as bytes
```

### Saving

```rust
// Save to file (format inferred from extension)
img.save("output.jpg")?;

// Save with explicit format
img.save_as("output.webp", ImageFormat::WebP)?;

// Get bytes (for storing in storage)
let bytes = img.to_bytes(ImageFormat::Png)?;
app.storage()?.put("thumbnails/photo.png", bytes).await?;
```

### Supported Formats

| Format | Extension | Read | Write |
|--------|-----------|------|-------|
| JPEG | `.jpg`, `.jpeg` | Yes | Yes |
| PNG | `.png` | Yes | Yes |
| WebP | `.webp` | Yes | Yes |
| GIF | `.gif` | Yes | Yes |
| BMP | `.bmp` | Yes | Yes |
| TIFF | `.tiff`, `.tif` | Yes | Yes |
| AVIF | `.avif` | Yes | Yes |
| ICO | `.ico` | Yes | Yes |

---

## Upload → Process → Store

Common pattern: receive upload, process image, store result:

```rust
async fn upload_avatar(
    State(app): State<AppContext>,
    Auth(user): Auth<User>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse> {
    let form = MultipartForm::from_multipart(&mut multipart).await?;
    let file = form.file("avatar")?;

    // Process: resize to 256x256, optimize quality
    let processed = ImageProcessor::open(&file.temp_path)?
        .resize_to_fill(256, 256)
        .quality(80)
        .to_bytes(ImageFormat::WebP)?;

    // Store processed image
    let storage = app.storage()?;
    let path = format!("avatars/{}.webp", user.id);
    let stored = storage.put(&path, processed).await?;

    Ok(Json(json!({ "avatar_url": stored.url })))
}
```

### Generate Multiple Sizes

```rust
async fn upload_photo(app: &AppContext, file: &UploadedFile) -> Result<PhotoUrls> {
    let storage = app.storage()?;
    let img = ImageProcessor::open(&file.temp_path)?;
    let name = file.generate_storage_name();
    let stem = name.trim_end_matches(&format!(".{}", file.original_extension().unwrap_or_default()));

    // Original
    let original = file.store(app, "photos").await?;

    // Thumbnail (150x150)
    let thumb_bytes = ImageProcessor::open(&file.temp_path)?
        .resize_to_fill(150, 150)
        .quality(75)
        .to_bytes(ImageFormat::WebP)?;
    storage.put(&format!("photos/thumbs/{stem}.webp"), thumb_bytes).await?;

    // Medium (800px wide)
    let medium_bytes = ImageProcessor::open(&file.temp_path)?
        .resize_to_fit(800, 800)
        .quality(85)
        .to_bytes(ImageFormat::WebP)?;
    storage.put(&format!("photos/medium/{stem}.webp"), medium_bytes).await?;

    Ok(PhotoUrls {
        original: original.url.unwrap_or_default(),
        thumb: storage.url(&format!("photos/thumbs/{stem}.webp"))?,
        medium: storage.url(&format!("photos/medium/{stem}.webp"))?,
    })
}
```

---

## Custom Storage Drivers

Register via ServiceProvider or Plugin:

```rust
registrar.register_storage_driver("gcs", Arc::new(|config, table| {
    Box::pin(async move {
        let bucket = table.get("bucket").and_then(|v| v.as_str()).unwrap_or_default();
        Ok(Arc::new(GcsAdapter::new(bucket)) as Arc<dyn StorageAdapter>)
    })
}));
```

Then configure:

```toml
[storage.disks.gcs]
driver = "gcs"
bucket = "my-bucket"
```

Use identically to built-in drivers:

```rust
let gcs = app.storage()?.disk("gcs")?;
gcs.put("file.txt", b"hello").await?;
```
