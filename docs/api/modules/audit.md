# audit

Built-in audit logging with automatic model mutation tracking and redaction

[Back to index](../index.md)

## foundry::audit

```rust
struct AuditContext
  fn new(area: impl Into<String>) -> Self
  fn try_new(area: impl Into<String>) -> Result<Self>
  fn with_actor(self, actor: Actor) -> Self
  fn with_request_id(self, request_id: RequestId) -> Self
  fn with_ip(self, ip: IpAddr) -> Self
  fn with_user_agent(self, user_agent: impl Into<String>) -> Self
  fn area(&self) -> &str
  fn actor(&self) -> Option<&Actor>
struct AuditEntry
  fn new( event_type: impl Into<String>, subject_table: impl Into<String>, subject_id: impl Into<String>, ) -> Self
  fn subject_model(self, subject_model: impl Into<String>) -> Self
  fn area(self, area: impl Into<String>) -> Self
  fn before(self, value: impl Serialize) -> Result<Self>
  fn after(self, value: impl Serialize) -> Result<Self>
  fn changes(self, value: impl Serialize) -> Result<Self>
struct AuditLog
  const ID: Column<Self, ModelId<AuditLog>>
  const EVENT_TYPE: Column<Self, String>
  const SUBJECT_MODEL: Column<Self, String>
  const SUBJECT_TABLE: Column<Self, String>
  const SUBJECT_ID: Column<Self, String>
  const AREA: Column<Self, Option<String>>
  const ACTOR_GUARD: Column<Self, Option<String>>
  const ACTOR_ID: Column<Self, Option<String>>
  const REQUEST_ID: Column<Self, Option<String>>
  const IP: Column<Self, Option<String>>
  const USER_AGENT: Column<Self, Option<String>>
  const BEFORE_DATA: Column<Self, Option<Value>>
  const AFTER_DATA: Column<Self, Option<Value>>
  const CHANGES: Column<Self, Option<Value>>
  const CREATED_AT: Column<Self, DateTime>
  fn query() -> ModelQuery<Self>
  fn create() -> CreateModel<Self>
  fn create_many() -> CreateManyModel<Self>
  fn update() -> UpdateModel<Self>
  fn delete() -> DeleteModel<Self>
  fn force_delete() -> DeleteModel<Self>
  fn restore() -> RestoreModel<Self>
struct AuditManager
  async fn record<E>(&self, executor: &E, entry: AuditEntry) -> Result<()>
  async fn prune_before<E>( &self, executor: &E, cutoff: DateTime, ) -> Result<u64>
  async fn prune_retention<E>( &self, executor: &E, now: DateTime, ) -> Result<u64>
  fn retention_days(&self) -> u32
async fn scope_audit<F>(context: AuditContext, future: F) -> F::Output
```

## Notes

- `#[foundry(audit_exclude)]` still removes a field entirely from audit payloads.
- `audit.redact_sensitive_fields = true` masks common credential-like field names with `[redacted]` in before/after/changes JSON.
- `audit.sensitive_fields` adds project-specific names; set `redact_sensitive_fields = false` to return to explicit model-only exclusions.
