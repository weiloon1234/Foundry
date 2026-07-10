# app_enum

Enum metadata and serialization (FoundryAppEnum)

[Back to index](../index.md)

## foundry::app_enum

```rust
enum EnumKey { String, Int }
  fn as_str(&self) -> Option<&str>
  fn as_i32(&self) -> Option<i32>
enum EnumKeyKind { String, Int }
struct EnumMeta
struct EnumOption
trait FoundryAppEnum: Clone
  fn id() -> &'static str
  fn key(self) -> EnumKey
  fn keys() -> Collection<EnumKey>
  fn parse_key(key: &str) -> Option<Self>
  fn label_key(self) -> &'static str
  fn options() -> Collection<EnumOption>
  fn meta() -> EnumMeta
  fn key_kind() -> EnumKeyKind
fn to_snake_case(name: &str) -> String
fn to_title_text(name: &str) -> String
```
