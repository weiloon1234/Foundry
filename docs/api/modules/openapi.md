# openapi

OpenAPI 3.1.0 spec generation (ApiSchema, RouteDoc)

[Back to index](../index.md)

## foundry::openapi

```rust
struct ApiSchemaDefinition
struct RouteDoc
  fn new() -> Self
  fn method(self, m: &str) -> Self
  fn action_name(self, action_name: impl Into<String>) -> Self
  fn get(self) -> Self
  fn post(self) -> Self
  fn put(self) -> Self
  fn patch(self) -> Self
  fn delete(self) -> Self
  fn summary(self, s: &str) -> Self
  fn description(self, d: &str) -> Self
  fn tag(self, t: &str) -> Self
  fn request<T: ApiSchema>(self) -> Self
  fn request_content_type(self, content_type: impl Into<String>) -> Self
  fn parameter<T: ApiSchema>( self, name: impl Into<String>, location: ContractParameterLocation, required: bool, ) -> Self
  fn path_parameter<T: ApiSchema>(self, name: impl Into<String>) -> Self
  fn query_parameter<T: ApiSchema>( self, name: impl Into<String>, required: bool, ) -> Self
  fn header_parameter<T: ApiSchema>( self, name: impl Into<String>, required: bool, ) -> Self
  fn cookie_parameter<T: ApiSchema>( self, name: impl Into<String>, required: bool, ) -> Self
  fn response<T: ApiSchema>(self, status: u16) -> Self
  fn error<T: ApiSchema>(self, status: u16, code: impl Into<String>) -> Self
  fn error_without_schema(self, status: u16, code: impl Into<String>) -> Self
  fn deprecated(self) -> Self
  fn merge_defaults(self, defaults: &Self) -> Self
struct SchemaRef
  fn of<T: ApiSchema>() -> Self
trait ApiSchema
  fn schema() -> Value
  fn schema_name() -> &'static str
fn nullable_schema(schema: Value) -> Value
```

## foundry::openapi::spec

```rust
struct DocumentedRoute
fn generate_openapi_spec( title: &str, version: &str, routes: &[DocumentedRoute], ) -> Value
fn generate_openapi_spec_from_contract( title: &str, version: &str, manifest: &ContractManifest, ) -> Value
```
