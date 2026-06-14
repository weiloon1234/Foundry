# metadata

Key-value metadata for models (HasMetadata)

[Back to index](../index.md)

## foundry::metadata

```rust
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
```

