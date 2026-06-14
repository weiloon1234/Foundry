# countries

Built-in country data (250 countries)

[Back to index](../index.md)

## foundry::countries

```rust
enum CountryStatus { Enabled, Disabled }
  fn as_str(&self) -> &'static str
  fn parse(s: &str) -> Self
  fn is_enabled(&self) -> bool
struct Country
  async fn find(app: &AppContext, iso2: &str) -> Result<Option<Country>>
  async fn all(app: &AppContext) -> Result<Vec<Country>>
  async fn by_status( app: &AppContext, status: CountryStatus, ) -> Result<Vec<Country>>
  async fn enabled(app: &AppContext) -> Result<Vec<Country>>
  async fn disabled(app: &AppContext) -> Result<Vec<Country>>
  async fn exists(app: &AppContext, iso2: &str) -> Result<bool>
struct CountryCurrency
struct CountrySeed
fn load_seed() -> Result<Vec<CountrySeed>>
async fn seed_countries(app: &AppContext) -> Result<u64>
async fn seed_countries_with(executor: &dyn QueryExecutor) -> Result<u64>
```

