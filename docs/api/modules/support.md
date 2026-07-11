# support

Utilities: typed IDs, datetime/clock, Collection<T>, crypto, hashing, locks

[Back to index](../index.md)

## foundry::support

```rust
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
enum Timezone { Utc, Iana, FixedOffset }
  fn utc() -> Self
  fn parse(value: impl AsRef<str>) -> Result<Self>
  fn as_str(&self) -> String
struct ChannelEventId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct ChannelId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct Clock
  fn new(timezone: Timezone) -> Self
  fn now(&self) -> DateTime
  fn today(&self) -> Date
  fn timezone(&self) -> &Timezone
struct Collection
  fn new() -> Self
  fn from_vec(items: Vec<T>) -> Self
  fn into_vec(self) -> Vec<T>
  fn as_slice(&self) -> &[T]
  fn len(&self) -> usize
  fn is_empty(&self) -> bool
  fn iter(&self) -> Iter<'_, T>
  fn to_vec(&self) -> Vec<T>
  fn first(&self) -> Option<&T>
  fn last(&self) -> Option<&T>
  fn get(&self, index: usize) -> Option<&T>
  fn map<U>(self, f: impl Fn(&T) -> U) -> Collection<U>
  fn map_into<U>(self, f: impl Fn(T) -> U) -> Collection<U>
  fn filter(self, f: impl Fn(&T) -> bool) -> Collection<T>
  fn reject(self, f: impl Fn(&T) -> bool) -> Collection<T>
  fn flat_map<U>(self, f: impl Fn(T) -> Vec<U>) -> Collection<U>
  fn find(&self, f: impl Fn(&T) -> bool) -> Option<&T>
  fn first_where(self, f: impl Fn(&T) -> bool) -> Option<T>
  fn any(&self, f: impl Fn(&T) -> bool) -> bool
  fn all(&self, f: impl Fn(&T) -> bool) -> bool
  fn count_where(&self, f: impl Fn(&T) -> bool) -> usize
  fn pluck<U>(self, f: impl Fn(&T) -> U) -> Collection<U>
  fn key_by<K: Eq + Hash>(self, f: impl Fn(&T) -> K) -> HashMap<K, T>
  fn group_by<K: Eq + Hash>( self, f: impl Fn(&T) -> K, ) -> HashMap<K, Collection<T>>
  fn unique_by<K: Eq + Hash>(self, f: impl Fn(&T) -> K) -> Collection<T>
  fn partition_by( self, f: impl Fn(&T) -> bool, ) -> (Collection<T>, Collection<T>)
  fn chunk(self, size: usize) -> Collection<Collection<T>>
  fn sort_by(&mut self, f: impl Fn(&T, &T) -> Ordering)
  fn sort_by_key<K: Ord>(&mut self, f: impl Fn(&T) -> K)
  fn reverse(&mut self)
  fn sum_by<U: Sum>(self, f: impl Fn(&T) -> U) -> U
  fn min_by<U: Ord>(self, f: impl Fn(&T) -> U) -> Option<U>
  fn max_by<U: Ord>(self, f: impl Fn(&T) -> U) -> Option<U>
  fn take(self, n: usize) -> Collection<T>
  fn skip(self, n: usize) -> Collection<T>
  fn for_each(self, f: impl FnMut(T))
  fn tap(self, f: impl FnMut(&Collection<T>)) -> Collection<T>
  fn pipe(self, f: impl Fn(Collection<T>) -> Collection<T>) -> Collection<T>
struct CommandId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct CryptManager
  fn from_config(config: &CryptConfig) -> Result<Self>
  fn encrypt(&self, plaintext: &[u8]) -> Result<String>
  fn decrypt(&self, encoded: &str) -> Result<Vec<u8>>
  fn encrypt_string(&self, plaintext: &str) -> Result<String>
  fn decrypt_string(&self, encoded: &str) -> Result<String>
struct Date
  fn parse(value: impl AsRef<str>) -> Result<Self>
  fn format(&self) -> String
struct DateTime
  fn now() -> Self
  fn parse(value: impl AsRef<str>) -> Result<Self>
  fn parse_in_timezone( value: impl AsRef<str>, timezone: &Timezone, ) -> Result<Self>
  fn format(&self) -> String
  fn format_in(&self, timezone: &Timezone) -> String
  fn date_in(&self, timezone: &Timezone) -> Date
  fn local_datetime_in(&self, timezone: &Timezone) -> LocalDateTime
  fn add_seconds(self, seconds: i64) -> Self
  fn sub_seconds(self, seconds: i64) -> Self
  fn add_days(self, days: i64) -> Self
  fn sub_days(self, days: i64) -> Self
  fn timestamp_millis(&self) -> i64
  fn timestamp_micros(&self) -> i64
struct EventId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct GuardId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct HashManager
  fn from_config(config: &HashingConfig) -> Result<Self>
  fn hash(&self, password: &str) -> Result<String>
  fn check(&self, password: &str, hash: &str) -> Result<bool>
  fn needs_rehash(&self, hash: &str) -> Result<bool>
  fn random_string(length: usize) -> Result<String>
struct JobId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct LocalDateTime
  fn parse(value: impl AsRef<str>) -> Result<Self>
  fn format(&self) -> String
  fn in_timezone(&self, timezone: &Timezone) -> Result<DateTime>
  fn date(&self) -> Date
  fn time(&self) -> Time
  fn add_seconds(self, seconds: i64) -> Self
  fn sub_seconds(self, seconds: i64) -> Self
  fn add_days(self, days: i64) -> Self
  fn sub_days(self, days: i64) -> Self
struct MiddlewareGroupId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct MigrationId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct ModelId
  fn generate() -> Self
  const fn from_uuid(value: Uuid) -> Self
  fn parse_str(value: &str) -> Result<Self, Error>
  const fn as_uuid(&self) -> &Uuid
  const fn into_uuid(self) -> Uuid
struct NotificationChannelId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct PermissionId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct PluginAssetId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct PluginId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct PluginScaffoldId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct PolicyId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct ProbeId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct QueueId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct RoleId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct RouteId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct ScheduleId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct SeederId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
struct Time
  fn parse(value: impl AsRef<str>) -> Result<Self>
  fn format(&self) -> String
struct Token
  fn generate(length: usize) -> Result<String>
  fn bytes(length: usize) -> Result<Vec<u8>>
  fn hex(bytes: usize) -> Result<String>
  fn base64(bytes: usize) -> Result<String>
struct ValidationRuleId
  const fn new(value: &'static str) -> Self
  fn owned(value: impl Into<String>) -> Self
  fn as_str(&self) -> &str
fn boxed<F, T>(future: F) -> BoxFuture<T>
async fn run_blocking<T, F>(label: impl Into<String>, work: F) -> Result<T>
fn sanitize_html(input: &str, allowed_tags: &[&str]) -> String
fn sha256_hex(data: &[u8]) -> String
fn sha256_hex_str(s: &str) -> String
fn strip_tags(input: &str) -> String
```

## foundry::support::lock

```rust
struct DistributedLock
  async fn acquire( &self, key: &str, ttl: Duration, ) -> Result<Option<LockGuard>>
  async fn block( &self, key: &str, ttl: Duration, wait_timeout: Duration, ) -> Result<LockGuard>
struct LockGuard
  async fn release(self) -> Result<bool>
  async fn extend(&self, ttl: Duration) -> Result<bool>
  fn start_heartbeat( &self, ttl: Duration, interval: Duration, ) -> LockHeartbeat
struct LockHeartbeat
```

## Notes

- `run_blocking(label, work)` isolates CPU-heavy or blocking synchronous work on Tokio's blocking pool and maps task panics into Foundry errors.
- `HashManager::hash()`, `HashManager::check()`, and `HashManager::needs_rehash()` remain synchronous; wrap password hashing or checking in `run_blocking` inside async handlers or model mutators.
