# validation

Validation: 38+ rules, custom rules, request validation extractor

[Back to index](../index.md)

## foundry::validation

```rust
struct EachValidator
  fn required(self) -> Self
  fn required_if( self, other_field: impl Into<String>, other_value: impl Into<String>, expected_values: impl IntoIterator<Item = impl Into<String>>, ) -> Self
  fn required_unless( self, other_field: impl Into<String>, other_value: impl Into<String>, expected_values: impl IntoIterator<Item = impl Into<String>>, ) -> Self
  fn required_with<I, N, V>(self, other_fields: I) -> Self
  fn present(self) -> Self
  fn prohibited(self) -> Self
  fn email(self) -> Self
  fn min(self, length: usize) -> Self
  fn max(self, length: usize) -> Self
  fn rule<I>(self, id: I) -> Self
  fn regex(self, pattern: impl Into<String>) -> Self
  fn url(self) -> Self
  fn uuid(self) -> Self
  fn numeric(self) -> Self
  fn boolean(self) -> Self
  fn alpha(self) -> Self
  fn alpha_numeric(self) -> Self
  fn in_list( self, values: impl IntoIterator<Item = impl Into<String>>, ) -> Self
  fn not_in(self, values: impl IntoIterator<Item = impl Into<String>>) -> Self
  fn starts_with(self, prefix: impl Into<String>) -> Self
  fn ends_with(self, suffix: impl Into<String>) -> Self
  fn ip(self) -> Self
  fn json(self) -> Self
  fn confirmed( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn digits(self) -> Self
  fn timezone(self) -> Self
  fn date(self) -> Self
  fn time(self) -> Self
  fn datetime(self) -> Self
  fn local_datetime(self) -> Self
  fn before( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn before_or_equal( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn after( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn after_or_equal( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn min_numeric(self, min: f64) -> Self
  fn max_numeric(self, max: f64) -> Self
  fn integer(self) -> Self
  fn between(self, min: f64, max: f64) -> Self
  fn ipv4(self) -> Self
  fn ipv6(self) -> Self
  fn same( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn different( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn unique(self, table: impl Into<String>, column: impl Into<String>) -> Self
  fn exists(self, table: impl Into<String>, column: impl Into<String>) -> Self
  fn app_enum<E: FoundryAppEnum>(self) -> Self
  fn distinct(self) -> Self
  fn nullable(self) -> Self
  fn sometimes(self) -> Self
  fn bail(self) -> Self
  fn with_message(self, message: impl Into<String>) -> Self
  async fn apply(self) -> Result<()>
struct FieldError
struct FieldValidator
  fn required(self) -> Self
  fn required_if( self, other_field: impl Into<String>, other_value: impl Into<String>, expected_values: impl IntoIterator<Item = impl Into<String>>, ) -> Self
  fn required_unless( self, other_field: impl Into<String>, other_value: impl Into<String>, expected_values: impl IntoIterator<Item = impl Into<String>>, ) -> Self
  fn required_with<I, N, V>(self, other_fields: I) -> Self
  fn present(self) -> Self
  fn prohibited(self) -> Self
  fn email(self) -> Self
  fn min(self, length: usize) -> Self
  fn max(self, length: usize) -> Self
  fn rule<I>(self, id: I) -> Self
  fn regex(self, pattern: impl Into<String>) -> Self
  fn url(self) -> Self
  fn uuid(self) -> Self
  fn numeric(self) -> Self
  fn boolean(self) -> Self
  fn alpha(self) -> Self
  fn alpha_numeric(self) -> Self
  fn in_list( self, values: impl IntoIterator<Item = impl Into<String>>, ) -> Self
  fn not_in(self, values: impl IntoIterator<Item = impl Into<String>>) -> Self
  fn starts_with(self, prefix: impl Into<String>) -> Self
  fn ends_with(self, suffix: impl Into<String>) -> Self
  fn ip(self) -> Self
  fn json(self) -> Self
  fn confirmed( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn digits(self) -> Self
  fn timezone(self) -> Self
  fn date(self) -> Self
  fn time(self) -> Self
  fn datetime(self) -> Self
  fn local_datetime(self) -> Self
  fn before( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn before_or_equal( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn after( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn after_or_equal( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn min_numeric(self, min: f64) -> Self
  fn max_numeric(self, max: f64) -> Self
  fn integer(self) -> Self
  fn between(self, min: f64, max: f64) -> Self
  fn ipv4(self) -> Self
  fn ipv6(self) -> Self
  fn same( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn different( self, other_field: impl Into<String>, other_value: impl Into<String>, ) -> Self
  fn unique(self, table: impl Into<String>, column: impl Into<String>) -> Self
  fn exists(self, table: impl Into<String>, column: impl Into<String>) -> Self
  fn app_enum<E: FoundryAppEnum>(self) -> Self
  fn distinct(self) -> Self
  fn nullable(self) -> Self
  fn sometimes(self) -> Self
  fn bail(self) -> Self
  fn with_message(self, message: impl Into<String>) -> Self
  async fn apply(self) -> Result<()>
struct JsonValidated
struct Multipart
  async fn next_field(&mut self) -> Result<Option<Field<'_>>, MultipartError>
struct RuleContext
  fn new(app: AppContext, field: impl Into<String>) -> Self
  fn app(&self) -> &AppContext
  fn field(&self) -> &str
struct RuleRegistry
  fn new() -> Self
  fn register<I>(&self, id: I, rule: impl ValidationRule) -> Result<()>
  fn register_arc<I>( &self, id: I, rule: Arc<dyn ValidationRule>, ) -> Result<()>
  fn get( &self, id: &ValidationRuleId, ) -> Result<Option<Arc<dyn ValidationRule>>>
struct Validated
struct ValidationError
  fn new(code: impl Into<String>, message: impl Into<String>) -> Self
struct ValidationErrors
  fn new(errors: Vec<FieldError>) -> Self
  fn is_empty(&self) -> bool
struct Validator
  fn new(app: AppContext) -> Self
  fn app(&self) -> &AppContext
  fn field<'a>( &'a mut self, name: impl Into<String>, value: impl Into<String>, ) -> FieldValidator<'a>
  fn field_with_presence<'a>( &'a mut self, name: impl Into<String>, value: impl Into<String>, present: bool, ) -> FieldValidator<'a>
  fn optional_field<'a, T>( &'a mut self, name: impl Into<String>, value: Option<T>, ) -> FieldValidator<'a>
  fn each<'a, T>( &'a mut self, field: impl Into<String>, items: &'a [T], ) -> EachValidator<'a, T>
  fn finish(self) -> Result<(), ValidationErrors>
  fn add_error(&mut self, field: &str, code: &str, params: &[(&str, &str)])
  fn locale(self, locale: impl Into<String>) -> Self
  fn set_locale(&mut self, locale: impl Into<String>)
  fn custom_message( &mut self, field: impl Into<String>, code: impl Into<String>, message: impl Into<String>, )
  fn custom_attribute( &mut self, field: impl Into<String>, name: impl Into<String>, )
trait FromMultipart
  fn from_multipart<'life0, 'async_trait>(
  fn from_multipart_with_presence<'life0, 'async_trait>(
  fn cleanup_multipart_files<'life0, 'async_trait>(
trait RequestValidator
  fn validate<'life0, 'life1, 'async_trait>(
  fn messages(&self) -> Vec<(String, String, String)>
  fn attributes(&self) -> Vec<(String, String)>
  fn request_messages() -> Vec<(String, String, String)>
  fn request_attributes() -> Vec<(String, String)>
trait ValidationRule
  fn validate<'life0, 'life1, 'life2, 'async_trait>(
```

## foundry::validation::file_rules

```rust
fn check_allowed_extensions(file: &UploadedFile, allowed: &[String]) -> bool
async fn check_allowed_mimes( file: &UploadedFile, allowed: &[String], ) -> Result<bool>
fn check_max_size(file: &UploadedFile, max_kb: u64) -> bool
async fn get_image_dimensions(file: &UploadedFile) -> Result<(u32, u32)>
async fn is_image(file: &UploadedFile) -> Result<bool>
```
