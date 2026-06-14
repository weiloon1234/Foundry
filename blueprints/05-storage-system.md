# Rust Storage System Blueprint (Framework-Level)

## Overview

This document defines the design of a **framework-level storage system** for Foundry.

Goal:

> Provide Laravel-style multi-disk storage with Rust-native explicitness, a configurable default disk, built-in local and S3 support, and an easy upload workflow that feels first-class in app code.

Storage is a framework subsystem, not just a bag of file helpers.

---

# Objective

Build a storage system that:

- supports **multiple named disks**
- uses a **configurable default disk**
- provides one public API regardless of backend
- ships with **local** and **S3** as first-class v1 disks
- supports **custom adapters/drivers**
- includes an **easy upload helper** for HTTP multipart files
- fits naturally into Foundry’s existing app/runtime shape:

```rust
let storage = app.storage()?;
let disk = storage.default_disk()?;
```

---

# Core Philosophy

1. **One storage API, many disks**
2. **Disk selection is config-driven**
3. **Default disk is automatic; explicit disk selection stays available**
4. **Uploads should be easy in handlers, but still go through the same storage manager**
5. **Adapters own storage behavior; app code should not branch on local vs S3**
6. **Safe defaults first, escape hatches explicit**
7. **Storage paths are app-defined keys, not OS-specific absolute paths**

---

# Module Shape

Introduce a new framework module:

```text
src/storage/
```

Primary public types:

- `StorageManager`
- `StorageDisk`
- `StorageAdapter`
- `StorageConfig`
- `StorageDiskConfig`
- `StorageVisibility`
- `UploadedFile`
- `StoredFile`

Primary app entrypoint:

```rust
AppContext::storage() -> Result<Arc<StorageManager>>
```

This should be a first-class app service, like `app.database()?` and `app.redis()?`.

---

# Config Model

Add a new top-level typed config section:

```toml
[storage]
default = "local"

[storage.disks.local]
driver = "local"
root = "storage/app"
visibility = "private"

[storage.disks.public]
driver = "local"
root = "storage/app/public"
url = "http://localhost:3000/storage"
visibility = "public"

[storage.disks.s3]
driver = "s3"
bucket = "foundry-prod"
region = "ap-southeast-1"
key = "${AWS_ACCESS_KEY_ID}"
secret = "${AWS_SECRET_ACCESS_KEY}"
endpoint = ""
url = ""
use_path_style = false
visibility = "private"
```

## Typed Config Shape

```rust
pub struct StorageConfig {
    pub default: String,
    pub disks: BTreeMap<String, StorageDiskConfig>,
}
```

```rust
pub enum StorageDiskConfig {
    Local(LocalDiskConfig),
    S3(S3DiskConfig),
    Custom(CustomDiskConfig),
}
```

```rust
pub struct LocalDiskConfig {
    pub root: String,
    pub url: Option<String>,
    pub visibility: StorageVisibility,
}
```

```rust
pub struct S3DiskConfig {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub key: String,
    pub secret: String,
    pub url: Option<String>,
    pub use_path_style: bool,
    pub visibility: StorageVisibility,
}
```

```rust
pub struct CustomDiskConfig {
    pub driver: String,
    pub options: toml::Table,
}
```

## Rules

- `storage.default` must point to a configured disk name
- every disk must have a `driver`
- v1 built-in drivers:
  - `local`
  - `s3`
- `visibility` supports:
  - `public`
  - `private`
- missing disk, missing driver, or unknown driver is startup/config error

## Defaults

- `storage.default = "local"`
- local disk default visibility = `private`
- s3 disk default visibility = `private`
- `endpoint` and `url` are optional
- `use_path_style = false` by default for S3

---

# Public API

## AppContext

```rust
pub fn storage(&self) -> Result<Arc<StorageManager>>
```

## StorageManager

```rust
pub struct StorageManager { ... }
```

Methods:

- `default_disk(&self) -> Result<StorageDisk>`
- `disk(&self, name: &str) -> Result<StorageDisk>`
- `default_disk_name(&self) -> &str`
- `configured_disks(&self) -> Vec<String>`

Behavior:

- `default_disk()` resolves the disk named by config
- `disk(name)` resolves an explicit named disk
- resolution errors are framework `Error`

## StorageDisk

`StorageDisk` is a cheap cloneable handle around a resolved disk adapter and its disk metadata.

Methods:

- `name(&self) -> &str`
- `visibility(&self) -> StorageVisibility`
- `put(&self, path: &str, contents: impl AsRef<[u8]>) -> Result<StoredFile>`
- `put_bytes(&self, path: &str, bytes: impl AsRef<[u8]>) -> Result<StoredFile>`
- `put_file(&self, path: &str, temp_path: &Path, content_type: Option<&str>) -> Result<StoredFile>`
- `get(&self, path: &str) -> Result<Vec<u8>>`
- `delete(&self, path: &str) -> Result<()>`
- `exists(&self, path: &str) -> Result<bool>`
- `copy(&self, from: &str, to: &str) -> Result<()>`
- `move_to(&self, from: &str, to: &str) -> Result<()>`
- `url(&self, path: &str) -> Result<String>`
- `temporary_url(&self, path: &str, expires_at: DateTime) -> Result<String>`

## StoredFile

Returned by storage put/upload operations.

```rust
pub struct StoredFile {
    pub disk: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub content_type: Option<String>,
    pub url: Option<String>,
}
```

Rules:

- `path` is the storage-relative key
- `name` is the final file name portion
- `url` is populated when the disk can generate one immediately
- `url` may be `None` for private disks or disks without stable public URLs

---

# Adapter / Driver Model

## Public Trait

```rust
#[async_trait]
pub trait StorageAdapter: Send + Sync + 'static {
    async fn put_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile>;

    async fn put_file(
        &self,
        path: &str,
        temp_path: &Path,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile>;

    async fn get(&self, path: &str) -> Result<Vec<u8>>;
    async fn delete(&self, path: &str) -> Result<()>;
    async fn exists(&self, path: &str) -> Result<bool>;
    async fn copy(&self, from: &str, to: &str) -> Result<()>;
    async fn move_to(&self, from: &str, to: &str) -> Result<()>;
    async fn url(&self, path: &str) -> Result<String>;
    async fn temporary_url(&self, path: &str, expires_at: DateTime) -> Result<String>;
}
```

## Built-in Adapters

v1 first-class adapters:

- `LocalStorageAdapter`
- `S3StorageAdapter`

## Registration Model

Built-in drivers are registered by the framework automatically.

Custom drivers are registered through the normal provider flow, not through globals:

```rust
registrar.register_storage_driver("cloudflare_r2", driver_factory)?;
```

Suggested factory shape:

```rust
type StorageDriverFactory =
    Arc<dyn Fn(&ConfigRepository, &toml::Table) -> Result<Arc<dyn StorageAdapter>> + Send + Sync>;
```

Rules:

- driver lookup is by `driver = "..."`
- local and s3 are always available
- custom drivers can consume raw adapter-specific config from the `options` table
- app code never resolves adapters directly; it resolves disks through `StorageManager`

---

# Upload Helper Behavior

Upload DX is first-class in **both** surfaces:

- manager-first:
  - `app.storage()?.default_disk()?.put_file(...)`
- file-helper-first:
  - `upload.store(&app, "avatars").await?`

## UploadedFile

`UploadedFile` is the framework upload type extracted from `multipart/form-data`.

Shape:

```rust
pub struct UploadedFile {
    pub field_name: String,
    pub original_name: Option<String>,
    pub content_type: Option<String>,
    pub size: u64,
    pub temp_path: PathBuf,
}
```

Rules:

- upload contents are stored in a temp file during request handling
- framework should not require keeping full uploads in memory
- upload metadata is preserved for later validation or storage

## UploadedFile Helpers

Methods:

- `store(&self, app: &AppContext, dir: &str) -> Result<StoredFile>`
- `store_on(&self, app: &AppContext, disk: &str, dir: &str) -> Result<StoredFile>`
- `store_as(&self, app: &AppContext, dir: &str, name: &str) -> Result<StoredFile>`
- `store_as_on(&self, app: &AppContext, disk: &str, dir: &str, name: &str) -> Result<StoredFile>`

Behavior:

- `store*` without an explicit disk uses the configured default disk
- default file names are **opaque UUIDv7-based**
- original extension is preserved only after safe normalization
- provided `name` in `store_as*` is treated as app intent and must still be normalized
- helpers create the final storage path by combining `dir` + generated/normalized name
- directory creation is handled by the underlying disk adapter
- upload helpers are generic and not tied to models or database records

## Filename Rules

Default generated names:

- use `ModelId`-style UUIDv7 semantics, but for files
- serialize as string
- preserve a safe normalized extension when one exists

Example:

```text
avatars/0195f6de-0c78-7e1c-91f3-08cb998e54a1.png
```

Normalization rules:

- lowercase extension
- strip path separators and dangerous characters
- no trust in client-supplied full path/name

---

# Visibility and URL Rules

## Visibility

Supported values:

- `public`
- `private`

Meaning:

- `public` means the disk is allowed to expose stable public URLs when configured
- `private` means reads still work through storage APIs, but public URLs are not assumed

## url()

`url(path)`:

- returns a stable public URL when the disk supports it
- on local disks, requires a configured `url`
- on S3 disks, can use explicit `url` or construct from adapter config
- returns an error when URL generation is unsupported for that disk/path

## temporary_url()

`temporary_url(path, expires_at)`:

- supported for S3 in v1
- not required for local in v1
- returns a clear unsupported error for disks that cannot produce signed URLs

## delete()

Storage `delete()` is always a **physical delete**.

There is no soft-delete concept in the storage layer.

---

# HTTP Integration

Foundry should add a multipart extractor story under `http/` that matches existing extractor patterns like `Validated<T>`.

## Primary DX

Single file:

```rust
async fn upload_avatar(
    State(app): State<AppContext>,
    upload: UploadedFile,
) -> Result<Json<StoredFile>> {
    let stored = upload.store(&app, "avatars").await?;
    Ok(Json(stored))
}
```

Multipart forms:

```rust
async fn upload_document(
    State(app): State<AppContext>,
    mut form: MultipartForm,
) -> Result<Json<StoredFile>> {
    let upload = form.file("document")?;
    let stored = upload.store_on(&app, "s3", "documents").await?;
    Ok(Json(stored))
}
```

## Request Types

v1 intended public types:

- `UploadedFile` for simple single-file extraction
- `MultipartForm` for explicit multi-field multipart handling

Suggested `MultipartForm` helpers:

- `file(name) -> Result<&UploadedFile>`
- `files(name) -> Vec<&UploadedFile>`
- `text(name) -> Option<&str>`

Rules:

- multipart parsing belongs to HTTP layer
- storage belongs to storage layer
- upload helper methods bridge the two cleanly

---

# Usage Examples

## Manager-First

```rust
let storage = app.storage()?;
storage.default_disk()?.put_bytes("avatars/a.txt", bytes).await?;
storage.disk("s3")?.put_bytes("reports/x.csv", bytes).await?;
```

## Upload Helper

```rust
let stored = upload.store(&app, "avatars").await?;
let stored = upload.store_on(&app, "s3", "avatars").await?;
```

## Explicit File Name

```rust
let stored = upload.store_as(&app, "reports", "monthly.csv").await?;
let stored = upload
    .store_as_on(&app, "s3", "reports", "monthly.csv")
    .await?;
```

## Cross-Disk App Code

```rust
let storage = app.storage()?;

let local = storage.disk("public")?;
let private_s3 = storage.disk("s3")?;

local.put_bytes("avatars/a.txt", b"hello").await?;
private_s3.copy("reports/a.csv", "reports/archive/a.csv").await?;
```

---

# Error Semantics

The storage API should use Foundry `Error` consistently.

Required failure cases:

- default disk is missing
- named disk is missing
- driver is unknown
- adapter init fails from invalid config
- `url()` is unsupported on a disk
- `temporary_url()` is unsupported on a disk
- requested upload field is missing
- uploaded temp file no longer exists
- copy/move source path is missing

Errors should be explicit, not silent fallbacks.

---

# Testing and Acceptance Coverage

Implementation must cover:

- default disk resolution from config
- named disk lookup
- local disk read/write/delete
- S3 disk read/write/delete
- public URL generation
- temporary URL generation for S3
- multipart upload to default disk
- multipart upload to explicit disk
- UUIDv7 filename generation with extension preservation
- missing disk / unknown driver failures
- custom adapter registration
- temp-file cleanup after upload handling
- clear behavior when URL helpers are unsupported on a disk

## Minimum Acceptance Scenarios

### Config

- loads `[storage]` section correctly
- fails fast when `storage.default` is undefined
- fails fast when disk config is malformed

### Local Disk

- writes bytes to `root/path`
- reads them back
- deletes them physically
- generates URL only when `url` is configured

### S3 Disk

- uploads/downloads bytes
- deletes objects
- generates stable public URL when possible
- generates temporary signed URL in v1

### Uploads

- multipart upload streams to temp file
- temp file metadata is available on `UploadedFile`
- `store()` uses configured default disk
- `store_on()` uses explicit disk
- temp file is cleaned up after request/upload lifecycle

### Extensibility

- custom driver registration resolves through `driver = "..."`
- custom adapter can be used as a named disk without modifying app code

---

# Non-Goals for v1

Not part of this blueprint’s v1 runtime scope:

- in-memory/fake runtime disk as a first-class production disk
- image processing
- CDN invalidation
- file versioning
- resumable/chunked uploads
- storage-side soft delete
- automatic model attachment behavior

Testing fakes may be added later, but they are not a first-class v1 runtime disk.

---

# Assumptions and Defaults

- v1 first-class disks: `local` and `s3`
- default upload DX: both `app.storage()` and `UploadedFile::store(...)`
- default disk is config-driven
- default upload filenames are opaque UUIDv7-based names
- local and s3 are runtime disks; in-memory/fake storage is not a first-class v1 runtime disk
- testing fakes are future work
- this blueprint introduces a future `storage/` module, but does **not** claim it already exists in Foundry
