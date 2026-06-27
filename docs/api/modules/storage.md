# storage

File storage: local + S3, multipart uploads, file validation

[Back to index](../index.md)

## Notes

- `types:export` mirrors browser-safe storage config into
  `StorageRuntimeManifest`, plus `storageUploadLimits()`,
  `storageImageLimits()`, and `storageAttachmentOrphanPolicy()` helpers for
  frontend upload forms and storage admin tooling.

## foundry::storage

```rust
pub type StorageDriverFactory = Arc<dyn Fn(&ConfigRepository, &Table) -> Pin<Box<dyn Future<Output = Result<Arc<dyn StorageAdapter>>> + Send>> + Send + Sync>;
struct StorageManager
  async fn from_config( config: &ConfigRepository, custom_drivers: HashMap<String, StorageDriverFactory>, ) -> Result<Self>
  fn default_disk(&self) -> Result<StorageDisk>
  fn disk(&self, name: &str) -> Result<StorageDisk>
  fn default_disk_name(&self) -> &str
  fn configured_disks(&self) -> Vec<String>
  fn descriptors(&self) -> Vec<StorageDiskDescriptor>
  async fn put( &self, path: &str, contents: impl AsRef<[u8]>, ) -> Result<StoredFile>
  async fn put_bytes( &self, path: &str, bytes: impl AsRef<[u8]>, ) -> Result<StoredFile>
  async fn put_file( &self, path: &str, temp_path: &Path, content_type: Option<&str>, ) -> Result<StoredFile>
  async fn get(&self, path: &str) -> Result<Vec<u8>>
  async fn delete(&self, path: &str) -> Result<()>
  async fn exists(&self, path: &str) -> Result<bool>
  async fn copy(&self, from: &str, to: &str) -> Result<()>
  async fn move_to(&self, from: &str, to: &str) -> Result<()>
  async fn url(&self, path: &str) -> Result<String>
  async fn temporary_url( &self, path: &str, expires_at: DateTime, ) -> Result<String>
  async fn list_prefix( &self, prefix: &str, limit: usize, ) -> Result<Vec<StorageObject>>
struct StorageDiskDescriptor
```

## foundry::storage::adapter

```rust
enum StorageVisibility { Private, Public }
trait StorageAdapter
  fn put_bytes<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn put_file<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn get<'life0, 'life1, 'async_trait>(
  fn delete<'life0, 'life1, 'async_trait>(
  fn exists<'life0, 'life1, 'async_trait>(
  fn copy<'life0, 'life1, 'life2, 'async_trait>(
  fn move_to<'life0, 'life1, 'life2, 'async_trait>(
  fn url<'life0, 'life1, 'async_trait>(
  fn temporary_url<'life0, 'life1, 'async_trait>(
  fn list_prefix<'life0, 'life1, 'async_trait>(
```

## foundry::storage::config

```rust
pub const DEFAULT_MAX_UPLOAD_FILES: u64;
pub const DEFAULT_MAX_UPLOAD_FILE_SIZE_BYTES: u64;
pub const DEFAULT_MAX_UPLOAD_SIZE_BYTES: u64;
struct ResolvedLocalConfig
  fn from_table(table: &Table) -> Result<Self>
struct ResolvedS3Config
  fn from_table(table: &Table) -> Result<Self>
struct StorageConfig
```

## foundry::storage::disk

```rust
struct StorageDisk
  fn name(&self) -> &str
  fn driver(&self) -> &str
  fn visibility(&self) -> StorageVisibility
  async fn put( &self, path: &str, contents: impl AsRef<[u8]>, ) -> Result<StoredFile>
  async fn put_bytes( &self, path: &str, bytes: impl AsRef<[u8]>, ) -> Result<StoredFile>
  async fn put_file( &self, path: &str, temp_path: &Path, content_type: Option<&str>, ) -> Result<StoredFile>
  async fn get(&self, path: &str) -> Result<Vec<u8>>
  async fn delete(&self, path: &str) -> Result<()>
  async fn exists(&self, path: &str) -> Result<bool>
  async fn copy(&self, from: &str, to: &str) -> Result<()>
  async fn move_to(&self, from: &str, to: &str) -> Result<()>
  async fn url(&self, path: &str) -> Result<String>
  async fn temporary_url( &self, path: &str, expires_at: DateTime, ) -> Result<String>
  async fn list_prefix( &self, prefix: &str, limit: usize, ) -> Result<Vec<StorageObject>>
```

## foundry::storage::local

```rust
struct LocalStorageAdapter
  fn from_config(config: &ResolvedLocalConfig) -> Result<Self>
```

## foundry::storage::multipart

```rust
pub type UploadedFile = UploadedFile;
struct MultipartForm
  fn file(&self, name: &str) -> Result<&UploadedFile>
  fn files(&self, name: &str) -> &[UploadedFile] ⓘ
  fn text(&self, name: &str) -> Option<&str>
```

## foundry::storage::s3

```rust
struct S3StorageAdapter
  fn from_config(config: &ResolvedS3Config) -> Result<Self>
```

## foundry::storage::stored_file

```rust
struct StorageObject
struct StoredFile
```

## foundry::storage::upload

```rust
struct UploadCounters
struct UploadLimits
  fn from_config(config: &StorageConfig) -> Self
struct UploadedFile
  fn sanitize_name(name: &str) -> String
  fn generate_storage_name(&self) -> String
  async fn from_multipart_field( field_name: String, field: Field<'_>, counters: &mut UploadCounters, ) -> Result<Option<Self>>
  fn original_extension(&self) -> Option<String>
  fn normalize_name(name: &str) -> String
  async fn store(&self, app: &AppContext, dir: &str) -> Result<StoredFile>
  async fn store_on( &self, app: &AppContext, disk_name: &str, dir: &str, ) -> Result<StoredFile>
  async fn store_as( &self, app: &AppContext, dir: &str, name: &str, ) -> Result<StoredFile>
  async fn store_as_on( &self, app: &AppContext, disk_name: &str, dir: &str, name: &str, ) -> Result<StoredFile>
async fn cleanup_uploaded_files<'a, I>(files: I)
fn current_upload_limits() -> UploadLimits
async fn prune_stale_upload_temp_files( retention_seconds: u64, batch_size: u64, ) -> Result<u64>
async fn remove_uploaded_temp_file(file: &UploadedFile) -> bool
async fn scope_upload_limits<F>( limits: UploadLimits, future: F, ) -> F::Output
async fn uploaded_file_from_multipart_field( field_name: String, field: Field<'_>, limits: UploadLimits, counters: &mut UploadCounters, ) -> Result<Option<UploadedFile>>
```
