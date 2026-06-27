# typescript

[Back to index](../index.md)

## foundry::typescript

```rust
struct TsAppEnum
struct TsType
struct TsValidation
struct TsValidationAttribute
struct TsValidationField
struct TsValidationMessage
struct TsValidationRule
  fn new(code: impl Into<String>) -> Self
struct TsValidationSchema
fn builtin_cli_registrar(routes: Vec<RouteRegistrar>) -> CommandRegistrar
fn export_all(dir: &Path) -> Result<()>
fn export_all_with_routes( dir: &Path, routes: &[RouteManifestEntry], ) -> Result<()>
```

