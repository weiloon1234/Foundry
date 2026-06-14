# Collection API Design Spec

## Context

Foundry's query system should return `Collection<T>` on its public retrieval surfaces instead of leaking `Vec<T>` through the ORM. Users should be able to chain `.iter().map()`, `pluck`, `group_by`, relation loading, and other collection helpers without manually wrapping vectors first.

The Collection API provides a thin, ergonomic wrapper over `Vec<T>` focused on real backend value: plucking fields, grouping, keying, partitioning, and ORM-level eager loading on collections of models.

## Architecture

### Two-file structure

1. **`src/support/collection.rs`** — Generic `Collection<T>`, no database dependency
2. **`src/database/collection_ext.rs`** — `ModelCollectionExt<T: Model>` trait for ORM operations

### Core type

```rust
pub struct Collection<T> {
    items: Vec<T>,
}
```

## Interoperability

All of these are implemented so Collection plugs into existing Rust patterns:

- `From<Vec<T>>` and `From<Collection<T>> for Vec<T>`
- `FromIterator<T>` — enables `.collect::<Collection<_>>()`
- `IntoIterator` (owned)
- `AsRef<[T]>`
- Iterator via `.iter()` and `.into_iter()`

## Tier 1 — Core & High Value

### Basic

| Method | Signature | Notes |
|--------|-----------|-------|
| `new` | `() -> Collection<T>` | Empty collection |
| `from_vec` | `(Vec<T>) -> Collection<T>` | From raw vec |
| `into_vec` | `(self) -> Vec<T>` | Unwrap |
| `as_slice` | `(&self) -> &[T]` | Borrow |
| `len` | `(&self) -> usize` | |
| `is_empty` | `(&self) -> bool` | |

### Access

| Method | Signature |
|--------|-----------|
| `first` | `(&self) -> Option<&T>` |
| `last` | `(&self) -> Option<&T>` |
| `get` | `(&self, usize) -> Option<&T>` |

### Transform (returns new Collection)

| Method | Signature | Notes |
|--------|-----------|-------|
| `map` | `(self, f: impl Fn(&T) -> U) -> Collection<U>` | Reference access |
| `map_into` | `(self, f: impl Fn(T) -> U) -> Collection<U>` | Consuming |
| `filter` | `(self, f: impl Fn(&T) -> bool) -> Collection<T>` | Keep matches |
| `reject` | `(self, f: impl Fn(&T) -> bool) -> Collection<T>` | Remove matches |
| `flat_map` | `(self, f: impl Fn(T) -> Vec<U>) -> Collection<U>` | Flatten |

### Query

| Method | Signature |
|--------|-----------|
| `find` | `(&self, f: impl Fn(&T) -> bool) -> Option<&T>` |
| `first_where` | `(self, f: impl Fn(&T) -> bool) -> Option<T>` |
| `any` | `(&self, f: impl Fn(&T) -> bool) -> bool` |
| `all` | `(&self, f: impl Fn(&T) -> bool) -> bool` |
| `count_where` | `(&self, f: impl Fn(&T) -> bool) -> usize` |

### High-Value Backend Helpers

| Method | Signature | Notes |
|--------|-----------|-------|
| `pluck` | `(self, f: impl Fn(&T) -> U) -> Collection<U>` | Extract field |
| `key_by` | `(self, f: impl Fn(&T) -> K) -> HashMap<K, T>` where K: Eq + Hash | Index by key |
| `group_by` | `(self, f: impl Fn(&T) -> K) -> HashMap<K, Collection<T>>` where K: Eq + Hash | Bucket |
| `unique_by` | `(self, f: impl Fn(&T) -> K) -> Collection<T>` where K: Eq + Hash | Deduplicate |
| `partition_by` | `(self, f: impl Fn(&T) -> bool) -> (Collection<T>, Collection<T>)` | Split in two |
| `chunk` | `(&self, size: usize) -> Collection<Collection<T>>` | Batch |

## Tier 2 — Ordering & Aggregation

### Ordering

| Method | Signature |
|--------|-----------|
| `sort_by` | `(&mut self, f: impl Fn(&T, &T) -> Ordering)` |
| `sort_by_key` | `(&mut self, f: impl Fn(&T) -> K) where K: Ord` |
| `reverse` | `(&mut self)` |

### Aggregation

| Method | Signature | Notes |
|--------|-----------|-------|
| `sum_by` | `(self, f: impl Fn(&T) -> U) -> U where U: Sum` | Numeric sum |
| `min_by` | `(self, f: impl Fn(&T) -> U) -> Option<U> where U: Ord` | |
| `max_by` | `(self, f: impl Fn(&T) -> U) -> Option<U> where U: Ord` | |

Note: `avg_by` is omitted — it requires dividing by count which gets tricky with integer types. Users can do `col.sum_by(f) / col.len()` or use a specific numeric type.

## Tier 3 — Utilities

| Method | Signature | Notes |
|--------|-----------|-------|
| `take` | `(self, n: usize) -> Collection<T>` | First n items |
| `skip` | `(self, n: usize) -> Collection<T>` | Skip first n |
| `for_each` | `(self, f: impl Fn(T))` | Consume each |
| `tap` | `(self, f: impl Fn(&Collection<T>)) -> Collection<T>` | Inspect without consuming |
| `pipe` | `(self, f: impl Fn(Collection<T>) -> Collection<T>) -> Collection<T>` | Chain transforms |

## ORM Extension — `ModelCollectionExt`

File: `src/database/collection_ext.rs`

```rust
pub trait ModelCollectionExt<T: Model> {
    async fn load<E>(self, relation: ..., executor: &E) -> Result<Collection<T>>;
    async fn load_missing<E>(self, relation: ..., executor: &E) -> Result<Collection<T>>;
    fn model_keys(&self) -> Collection<DbValue>;
}
```

- `load` — eagerly loads a relation onto all models in the collection (batch query like existing `hydrate_model_batch`)
- `load_missing` — same but skips models where `Loaded<T>` is already `Loaded`
- `model_keys` — extracts primary key values from all models

Implemented on `Collection<T>` where `T: Model + PersistedModel`.

## Query System Integration

**Return type changes:**

| Method | Current | New |
|--------|---------|-----|
| `Query::get()` | `Vec<DbRecord>` | `Collection<DbRecord>` |
| `ModelQuery::get()` | `Vec<M>` | `Collection<M>` |
| `ProjectionQuery::get()` | `Vec<P>` | `Collection<P>` |
| `CreateModel::get()` | `Vec<M>` | `Collection<M>` |
| `CreateManyModel::get()` | `Vec<M>` | `Collection<M>` |
| `UpdateModel::get()` | `Vec<M>` | `Collection<M>` |
| `Paginated.data` | `Vec<T>` | `Collection<T>` |
| `ModelQuery::paginate()` | `Result<Paginated<M>>` | `Result<Paginated<M>>` (Paginated internally uses Collection) |

`first()`, `count()`, `sum()`, etc. remain unchanged — they return `Option<T>` or scalars.

Users who need `Vec<T>` can call `.into_vec()` or `Vec::from(collection)`.

## Re-exports

In `src/support/mod.rs`: `pub mod collection; pub use collection::Collection;`
In `src/database/mod.rs`: `pub mod collection_ext; pub use collection_ext::ModelCollectionExt;`
In `src/lib.rs`: `pub use support::Collection; pub use database::ModelCollectionExt;`

## Testing

- Unit tests in `src/support/collection.rs` via `#[cfg(test)] mod tests`
- Integration tests in `tests/` for ORM extension (requires database)
- Existing tests that assert `Vec<M>` from queries update to use `Collection<M>`

## File Changes Summary

| File | Change |
|------|--------|
| `src/support/collection.rs` | **New** — Collection<T> with all tiers |
| `src/support/mod.rs` | Add `pub mod collection` + re-export |
| `src/database/collection_ext.rs` | **New** — ModelCollectionExt trait |
| `src/database/mod.rs` | Add `pub mod collection_ext` + re-export |
| `src/database/query.rs` | Change return types from Vec<T> to Collection<T> |
| `src/lib.rs` | Add Collection and ModelCollectionExt re-exports |
