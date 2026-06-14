# Rust App Enum System Blueprint (Framework-Level)

> **Status:** ✅ Implemented
> **Last updated:** 2026-04-12

## Overview

This document defines the design of a **framework-level Rust app enum system** for Foundry.

Goal:

> Provide a minimal, first-class enum DX for Rust application code that works naturally with models and validation, stores stable keys in the database, and exposes metadata that is ready for future Rust-to-TypeScript export.

This is a **design blueprint only**. It does not mean the subsystem is already implemented.

---

# Objective

Build an app enum system that:

- stays **Rust-level only**
- does **not** depend on database enum-column types
- works directly in model fields
- works directly in validation rules
- removes repetitive handwritten enum impl boilerplate
- standardizes the shape of:
  - stored DB keys
  - validation allowed values
  - export-ready enum metadata
- is ready for future **Rust-to-TypeScript export**, without implementing export in this v1 blueprint

---

# Core Philosophy

1. **One enum, many uses**
2. **Stored key and label key are separate concerns**
3. **Database storage stays simple and portable**
4. **Validation should reuse enum truth, not repeat string lists**
5. **Frontend/export metadata should be stable and deterministic**
6. **DX should be minimal; normal Rust enum syntax stays the center**
7. **Enum storage mode must be explicit by structure, not ambiguous by mixed annotations**

---

# Module Shape

Introduce a new framework module:

```text
src/app_enum/
```

Primary public types:

- `FoundryAppEnum`
- `EnumKey`
- `EnumKeyKind`
- `EnumOption`
- `EnumMeta`

Primary derive:

```rust
#[derive(foundry::AppEnum)]
```

This should be re-exported through `foundry::prelude::*`.

---

# Public DX

## Default String-Backed Enum

```rust
#[derive(foundry::AppEnum)]
enum OrderStatus {
    Pending,
    Reviewing,
    Completed,
}
```

Meaning:

- stored DB keys:
  - `Pending` -> `"pending"`
  - `Reviewing` -> `"reviewing"`
  - `Completed` -> `"completed"`
- default label keys:
  - `Pending` -> `"Pending"`
  - `Reviewing` -> `"Reviewing"`
  - `Completed` -> `"Completed"`

## String-Backed Enum with Overrides

```rust
#[derive(foundry::AppEnum)]
enum OrderStatus {
    Pending,
    #[foundry(key = "in_review")]
    Reviewing,
    #[foundry(label_key = "Order completed")]
    Completed,
}
```

Meaning:

- DB stores:
  - `"pending"`
  - `"in_review"`
  - `"completed"`
- label keys:
  - `"Pending"`
  - `"Reviewing"`
  - `"Order completed"`

## Int-Backed Enum

```rust
#[derive(foundry::AppEnum)]
enum UserStatus {
    Pending = 0,
    Verified = 1,
}
```

Meaning:

- DB stores `0` and `1`
- default label keys:
  - `"Pending"`
  - `"Verified"`

## Optional Enum ID

`#[foundry(id = "...")]` is optional.

If omitted, the enum id is inferred from the Rust type name in `snake_case`:

- `UserStatus` -> `user_status`
- `OrderStatus` -> `order_status`

The enum id exists for metadata/export grouping only. It is **not** part of the default label-key convention.

---

# Storage Modes

Lock the design to exactly two enum storage modes.

## 1. String-Backed Mode

Triggered when variants have **no integer discriminants**.

Rules:

- stored key defaults from the variant name in `snake_case`
- optional per-variant override:
  - `#[foundry(key = "...")]`
- default label key is human-readable title text from the Rust variant name
- optional per-variant override:
  - `#[foundry(label_key = "...")]`

Examples:

- `Pending` -> stored key `"pending"`, label key `"Pending"`
- `ActiveIncome` -> stored key `"active_income"`, label key `"Active Income"`

## 2. Int-Backed Mode

Triggered when **every variant** has an explicit integer literal discriminant.

Rules:

- Foundry treats int-backed enums as `i32`-backed in v1
- `#[repr(i32)]` is not required for DX in v1
- if any discriminant is outside `i32`, derive fails
- `#[foundry(key = "...")]` is not allowed in this mode
- default label key is human-readable title text from the Rust variant name
- `#[foundry(label_key = "...")]` remains allowed

Examples:

- `Pending = 0` -> stored key `0`, label key `"Pending"`
- `ActiveIncome = 2` -> stored key `2`, label key `"Active Income"`

---

# Mixed Key Styles Are Forbidden

These must be compile-time errors:

- some variants with integer discriminants and some without
- `#[foundry(key = "...")]` on an int-backed enum
- mixed string and integer key styles in one enum
- non-unit enum variants
- non-literal integer discriminants
- integer discriminants outside the `i32` range

This is important because the enum must have one clear storage kind. Foundry should reject ambiguous enum storage design instead of guessing.

---

# Generated API

`#[derive(foundry::AppEnum)]` should generate:

- `ToDbValue`
- `FromDbValue`
- `Serialize`
- `Deserialize`
- `Clone`, `Copy`, `Debug`, `PartialEq`, `Eq` should remain explicit and user-controlled unless already derived by the enum itself

Generated helpers:

- `id() -> &'static str`
- `key(self) -> EnumKey`
- `keys() -> Collection<EnumKey>`
- `parse_key(&str) -> Option<Self>`
- `label_key(self) -> &'static str`
- `options() -> Collection<EnumOption>`
- `meta() -> EnumMeta`

## Metadata Types

```rust
pub enum EnumKey {
    String(String),
    Int(i32),
}
```

```rust
pub enum EnumKeyKind {
    String,
    Int,
}
```

```rust
pub struct EnumOption {
    pub value: EnumKey,
    pub label_key: String,
}
```

```rust
pub struct EnumMeta {
    pub id: String,
    pub key_kind: EnumKeyKind,
    pub options: Collection<EnumOption>,
}
```

Important rule:

- metadata carries only `value + label_key`
- no localized labels are generated here
- this metadata shape is the future bridge for Rust-to-TypeScript export

---

# Model Integration

Foundry models should treat `AppEnum` types as first-class model field types.

## Intended DX

```rust
#[derive(foundry::Model)]
#[foundry(model = "users")]
struct User {
    id: ModelId<User>,
    status: UserStatus,
}
```

Rules:

- no manual `#[foundry(db_type = "text")]` should be needed for string-backed app enums
- no extra `#[foundry(enum)]` field marker should be needed
- string-backed enums bind as text
- int-backed enums bind as `i32`
- query comparisons should work naturally:

```rust
User::query().where_(User::STATUS.eq(UserStatus::Pending))
```

---

# Validation Integration

Validation should reuse enum truth instead of repeating `in_list(...)`.

## Fluent API

```rust
validator
    .field("status", &payload.status)
    .app_enum::<UserStatus>()
    .apply()
    .await?;
```

## Derive Validation

```rust
#[validate(app_enum(UserStatus))]
pub status: String,
```

Rules:

- string-backed enums validate incoming string keys
- int-backed enums validate numeric input against declared integer keys
- validation errors reuse normal Foundry validation messaging
- enum metadata should be the SSOT behind both fluent and derive validation paths

---

# Label-Key Convention

Default `label_key` is **human-readable title text from the Rust variant name**.

Examples:

- `Pending` -> `"Pending"`
- `ActiveIncome` -> `"Active Income"`

This matches the Foundry i18n style where the translation key itself can be human-readable text.

Optional per-variant override:

```rust
#[foundry(label_key = "Status pending")]
```

Important rules:

- default label key does **not** use `enum_id`
- label key is not localized output
- localized output still comes from i18n lookup later

---

# Future Rust-to-TypeScript Readiness

Rust-to-TypeScript export is **not in this blueprint's scope** — it should be a separate module covering DTOs, models, and enums.

But the Rust side is shaped so export can be added later without redesign.

That means:

- `meta()` must be deterministic
- `options()` must be deterministic
- metadata must use stable primitive shapes
- the metadata must carry:
  - enum id
  - key kind
  - option values
  - option label keys

Future export can then serialize `EnumMeta` directly into TS constants or JSON artifacts.

---

# V2 Direction

V2 should stay additive and build on the v1 metadata shape instead of changing the core enum model.

The intended v2 work is:

- **Rust-to-TypeScript export** — removed from this blueprint. This should be a separate module covering DTOs, models, and enums — not just enum conversion. Not in any blueprint yet. Not doing anytime soon.

- **Localized option helpers** ✅ Done (layered on v1 via i18n)
  - add optional runtime helpers that resolve `label_key` through Foundry i18n
  - for example, return `{ value, label }` lists for server-driven UI or datatable filter metadata
  - keep this layered on top of v1; v1 remains metadata-only and never bakes translated labels into enum definitions

- **Backward-compat parsing aliases** ✅ Done (`#[foundry(aliases(...))]`)
  - allow optional extra accepted input keys for migrations or legacy API compatibility
  - serialization and DB persistence should still use one canonical key
  - alias support should be explicit and remain out of the default v1 surface

- **Enum discovery for tooling** ❌ Deferred
  - provide a registry or build-time collection mechanism so future export tooling can discover all `AppEnum` types in an app cleanly
  - this is especially useful for frontend export and documentation generation

Important rule:

- v2 should not introduce database enum-column support
- v2 should not change the v1 meaning of `key`, `label_key`, or the two storage modes

---

# Test Plan

**Status: ✅ Covered** — 35 acceptance tests in `src/app_enum/mod.rs`.

The blueprint should require coverage for:

- string-backed enum derive
- int-backed enum derive
- default enum id inference
- default snake_case stored keys for string-backed enums
- `#[foundry(key = "...")]` override for string-backed enums
- default title-text label-key generation
- `#[foundry(label_key = "...")]` override
- model persistence and hydration using `AppEnum`
- query filters using enum values
- fluent validation with `.app_enum::<T>()`
- derive validation with `#[validate(app_enum(...))]`
- serde round-trip for string-backed enums
- serde round-trip for int-backed enums
- `options()` returning stable metadata
- `meta()` returning stable export-ready metadata

Compile-fail coverage must include:

- mixed key styles
- partial discriminants
- non-unit enum variants
- `#[foundry(key = "...")]` on int-backed enums
- integer discriminants outside `i32`

---

# Assumptions and Defaults

- root file name: `rust_app_enum_system_blueprint_framework_level.md`
- this is a **blueprint** — now implemented as of 2026-04-12
- Foundry app enums are Rust-level, not Postgres enum-column based
- v1 supports **unit enums only**
- string-backed enums default to `snake_case` stored keys
- int-backed enums are inferred from explicit integer discriminants and standardized to `i32`
- `rename_all` is intentionally omitted in v1
- default `label_key` is human-readable title text from the Rust variant name
- `id` is optional and used for metadata/export grouping, not default label-key generation
- Rust-to-TypeScript export is removed from this blueprint — it should be a separate module covering DTOs too, not just enums
- v2 is expected to add localized option helpers (done), compatibility aliases (done), and enum discovery (deferred) without changing the v1 enum core
