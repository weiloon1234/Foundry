# audit

Built-in audit logging with automatic model mutation tracking and redaction

[Back to index](../index.md)

## foundry::audit

```rust
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
```

## Notes

- `#[foundry(audit_exclude)]` still removes a field entirely from audit payloads.
- `audit.redact_sensitive_fields = true` masks common credential-like field names with `[redacted]` in before/after/changes JSON.
- `audit.sensitive_fields` adds project-specific names; set `redact_sensitive_fields = false` to return to explicit model-only exclusions.

