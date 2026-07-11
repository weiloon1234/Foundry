# contract

Normalized contract manifest for generated SDKs, OpenAPI, validation, and realtime

[Back to index](../index.md)

## foundry::contract

```rust
pub const CONTRACT_MANIFEST_VERSION: u32;
enum ContractHttpBody { None, Json, Multipart, Unknown }
enum ContractParameterLocation { Path, Query, Header, Cookie }
enum ContractTransport { Http, WebSocket }
enum ContractValueKind { Show 15 variants    Unknown, Scalar, Array, Object, File, FileList, Date, DateTime, LocalDateTime, Time, Decimal, Uuid, Json, Page, Error }
struct ContractAction
  fn from_http_route(route: &RouteManifestEntry) -> Result<Self>
struct ContractAuth
struct ContractError
  fn standard_errors() -> Vec<Self>
struct ContractHttpTransport
struct ContractManifest
  fn new() -> Self
  fn from_http_routes(routes: &[RouteManifestEntry]) -> Result<Self>
  fn with_schemas(self, schemas: Vec<ContractSchema>) -> Self
  fn merge_schemas(self, schemas: Vec<ContractSchema>) -> Result<Self>
  fn with_validation_schemas( self, schemas: Vec<ContractValidationSchema>, ) -> Self
  fn with_realtime_channels( self, channels: Vec<ContractRealtimeChannel>, ) -> Self
  fn infer_transport_body_kinds(&mut self)
struct ContractParameter
struct ContractPayload
struct ContractRealtimeChannel
struct ContractRealtimeEvent
struct ContractResponse
struct ContractSchema
struct ContractValidationAttribute
struct ContractValidationField
struct ContractValidationMessage
struct ContractValidationRule
  fn is_file_rule(&self) -> bool
struct ContractValidationSchema
  fn requires_multipart(&self) -> bool
struct ContractWebSocketTransport
```
