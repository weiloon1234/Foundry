# translations

Model field translations across locales (HasTranslations)

[Back to index](../index.md)

## foundry::translations

```rust
pub const MODEL_TRANSLATIONS_TABLE: &str;
struct ModelTranslation
struct TranslatedFields
  fn from_entries( entries: Vec<(String, String)>, current_locale: &str, default_locale: &str, ) -> Self
  fn get(&self, locale: &str) -> Option<&str>
struct TranslationJoin
  fn new(alias: impl Into<String>) -> Self
  fn alias(&self) -> &str
  fn table(&self) -> TableRef
  fn column(&self, name: impl Into<String>) -> Expr
  fn value(&self) -> Expr
  fn on<M>( &self, translatable_id: impl Into<Expr>, field: impl Into<String>, locale: impl Into<String>, ) -> Condition
trait HasTranslations
  fn translatable_type() -> &'static str
  fn translatable_id(&self) -> String
  fn set_translation<'life0, 'life1, 'life2, 'life3, 'life4, 'async_trait>(
  fn set_translation_with<'life0, 'life1, 'life2, 'life3, 'life4, 'async_trait, E>(
  fn set_translations<'life0, 'life1, 'life2, 'life3, 'life4, 'life5, 'async_trait>(
  fn set_translations_with<'life0, 'life1, 'life2, 'life3, 'life4, 'life5, 'async_trait, E>(
  fn translation<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn translations_for<'life0, 'life1, 'life2, 'async_trait>(
  fn translated_field<'life0, 'life1, 'life2, 'async_trait>(
  fn all_translations<'life0, 'life1, 'async_trait>(
  fn delete_translations<'life0, 'life1, 'life2, 'async_trait>(
  fn delete_translations_with<'life0, 'life1, 'life2, 'async_trait, E>(
  fn delete_translation_field<'life0, 'life1, 'life2, 'async_trait>(
  fn delete_translation_field_with<'life0, 'life1, 'life2, 'async_trait, E>(
  fn delete_all_translations<'life0, 'life1, 'async_trait>(
  fn delete_all_translations_with<'life0, 'life1, 'async_trait, E>(
fn current_locale(app: &AppContext) -> String
fn translation_join(alias: impl Into<String>) -> TranslationJoin
```

