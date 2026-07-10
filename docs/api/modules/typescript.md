# typescript

[Back to index](../index.md)

## foundry::typescript

```rust
struct I18nTypeScriptManifest
struct TsAppEnum
struct TsType
struct TsValidation
struct TsValidationAttribute
struct TsValidationField
struct TsValidationMessage
struct TsValidationRule
  fn new(code: impl Into<String>) -> Self
struct TsValidationSchema
struct TypeScriptExportContext
fn builtin_cli_registrar( route_manifest: Vec<RouteManifestEntry>, websocket_routes: Vec<WebSocketRouteRegistrar>, ) -> CommandRegistrar
fn export_all(dir: &Path) -> Result<()>
fn export_all_with_context( dir: &Path, routes: &[RouteManifestEntry], context: TypeScriptExportContext, ) -> Result<()>
fn export_all_with_routes( dir: &Path, routes: &[RouteManifestEntry], ) -> Result<()>
```
