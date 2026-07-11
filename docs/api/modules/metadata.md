# metadata

Key-value metadata for models (HasMetadata)

[Back to index](../index.md)

## foundry::metadata

```rust
struct MetadataOwner
  fn new( metadatable_type: impl Into<String>, table: impl Into<String>, primary_key: impl Into<String>, ) -> Result<Self>
  fn for_model<M>() -> Result<Self>
  fn metadatable_type(&self) -> &str
  fn table(&self) -> &str
  fn primary_key(&self) -> &str
struct ModelMeta
trait HasMetadata
  fn metadatable_type() -> &'static str
  fn metadatable_id(&self) -> String
  fn set_meta<'life0, 'life1, 'life2, 'async_trait>(
  fn get_meta<'life0, 'life1, 'life2, 'async_trait, T>(
  fn get_meta_raw<'life0, 'life1, 'life2, 'async_trait>(
  fn forget_meta<'life0, 'life1, 'life2, 'async_trait>(
  fn has_meta<'life0, 'life1, 'life2, 'async_trait>(
  fn all_meta<'life0, 'life1, 'async_trait>(
  fn delete_all_meta<'life0, 'life1, 'async_trait>(
  fn delete_all_meta_with<'life0, 'life1, 'async_trait, E>(
async fn audit_metadata_orphans<E>( executor: &E, owner: &MetadataOwner, ) -> Result<u64>
async fn prune_metadata_orphans<E>( executor: &E, owner: &MetadataOwner, ) -> Result<u64>
```
