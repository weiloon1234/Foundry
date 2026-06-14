# settings

[Back to index](../index.md)

## foundry::settings

```rust
enum SettingType { Show 16 variants    Text, Textarea, Number, Boolean, Select, ... +11 more }
  fn as_str(&self) -> &'static str
  fn parse(s: &str) -> Option<Self>
  fn all() -> &'static [(&'static str, SettingType)]
struct NewSetting
  fn new(key: impl Into<String>, label: impl Into<String>) -> Self
  fn value(self, value: Value) -> Self
  fn setting_type(self, setting_type: SettingType) -> Self
  fn parameters(self, parameters: Value) -> Self
  fn group(self, group_name: impl Into<String>) -> Self
  fn description(self, description: impl Into<String>) -> Self
  fn sort_order(self, sort_order: i32) -> Self
  fn is_public(self, is_public: bool) -> Self
struct Setting
  async fn get(app: &AppContext, key: &str) -> Result<Option<Value>>
  async fn get_as<T: DeserializeOwned>( app: &AppContext, key: &str, ) -> Result<Option<T>>
  async fn get_or( app: &AppContext, key: &str, default: Value, ) -> Result<Value>
  async fn find(app: &AppContext, key: &str) -> Result<Option<Setting>>
  async fn set(app: &AppContext, key: &str, value: Value) -> Result<()>
  async fn create(app: &AppContext, new: NewSetting) -> Result<()>
  async fn upsert(app: &AppContext, key: &str, value: Value) -> Result<()>
  async fn remove(app: &AppContext, key: &str) -> Result<bool>
  async fn exists(app: &AppContext, key: &str) -> Result<bool>
  async fn all(app: &AppContext) -> Result<Vec<Setting>>
  async fn by_group(app: &AppContext, group: &str) -> Result<Vec<Setting>>
  async fn by_prefix(app: &AppContext, prefix: &str) -> Result<Vec<Setting>>
  async fn public(app: &AppContext) -> Result<Vec<Setting>>
  async fn groups(app: &AppContext) -> Result<Vec<String>>
```

