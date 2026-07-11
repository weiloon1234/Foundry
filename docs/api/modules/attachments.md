# attachments

File attachments with lifecycle (HasAttachments)

[Back to index](../index.md)

## foundry::attachments

```rust
enum AttachmentImageResize { Exact, Fit, Fill }
enum AttachmentSpecKind { File, Image }
struct Attachment
  fn upload(file: UploadedFile) -> AttachmentUploadBuilder
  fn is_image(&self) -> bool
  fn is_video(&self) -> bool
  async fn update_custom_properties( app: &AppContext, attachment_id: &str, custom_properties: Value, ) -> Result<u64>
  async fn update_custom_properties_with<E>( executor: &E, attachment_id: &str, custom_properties: Value, ) -> Result<u64>
  fn is_audio(&self) -> bool
  fn is_document(&self) -> bool
  fn extension(&self) -> Option<&str>
  fn human_size(&self) -> String
  async fn url(&self, app: &AppContext) -> Result<String>
  async fn temporary_url( &self, app: &AppContext, expires_at: DateTime, ) -> Result<String>
  async fn image(&self, app: &AppContext) -> Result<ImageProcessor>
struct AttachmentAfterStoreContext
struct AttachmentBeforeStoreContext
struct AttachmentImagePolicy
struct AttachmentSpec
  fn file(collection: impl Into<String>) -> Self
  fn image(collection: impl Into<String>) -> Self
  fn single(self) -> Self
  fn resize_exact(self, width: u32, height: u32) -> Self
  fn resize_to_fit(self, max_width: u32, max_height: u32) -> Self
  fn resize_to_fill(self, width: u32, height: u32) -> Self
  fn format(self, format: ImageFormat) -> Self
  fn quality(self, quality: u8) -> Self
  fn upscale(self, upscale: bool) -> Self
  fn hook<H>(self, hook: H) -> Self
  fn collection(&self) -> &str
  fn kind(&self) -> AttachmentSpecKind
  fn is_single(&self) -> bool
  fn image_policy(&self) -> Option<AttachmentImagePolicy>
struct AttachmentUploadBuilder
  fn collection(self, collection: impl Into<String>) -> Self
  fn disk(self, disk: impl Into<String>) -> Self
  fn resize(self, width: u32, height: u32) -> Self
  fn resize_to_fit(self, max_width: u32, max_height: u32) -> Self
  fn resize_to_fill(self, width: u32, height: u32) -> Self
  fn quality(self, quality: u8) -> Self
  fn format(self, format: ImageFormat) -> Self
  fn upscale(self, upscale: bool) -> Self
  async fn store( self, app: &AppContext, attachable_type: &str, attachable_id: &str, ) -> Result<Attachment>
trait AttachmentSpecHook
  fn before_store<'life0, 'life1, 'async_trait>(
  fn after_store<'life0, 'life1, 'async_trait>(
trait HasAttachments
  fn attachable_type() -> &'static str
  fn attachable_id(&self) -> String
  fn attachment_specs() -> Vec<AttachmentSpec<Self>>
  fn attach<'life0, 'life1, 'life2, 'async_trait>(
  fn replace_attachment<'life0, 'life1, 'life2, 'async_trait>(
  fn attach_localized<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn replace_localized_attachment<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn localized_attachment<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn localized_attachments<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn localized_attachment_or_default<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn current_localized_attachment<'life0, 'life1, 'life2, 'async_trait>(
  fn attachment<'life0, 'life1, 'life2, 'async_trait>(
  fn attachments<'life0, 'life1, 'life2, 'async_trait>(
  fn reorder_attachments<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn detach<'life0, 'life1, 'life2, 'async_trait>(
  fn detach_keep_file<'life0, 'life1, 'life2, 'async_trait>(
  fn detach_all<'life0, 'life1, 'life2, 'async_trait>(
fn available_attachment_locales(app: &AppContext) -> Result<Vec<String>>
fn localized_attachment_collection(collection: &str, locale: &str) -> String
```
