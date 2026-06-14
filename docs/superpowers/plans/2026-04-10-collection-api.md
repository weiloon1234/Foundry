# Collection API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a `Collection<T>` type that replaces `Vec<T>` in query results and provides ergonomic backend helpers (pluck, key_by, group_by, partition_by, chunk) plus ORM extension traits for eager loading.

**Architecture:** Two-file split — generic `Collection<T>` in `src/support/collection.rs` (no database dependency), and `ModelCollectionExt` trait in `src/database/collection_ext.rs` for ORM operations. Query return types change from `Vec<T>` to `Collection<T>`.

**Tech Stack:** Rust std lib (HashMap, BTreeSet for unique_by). No new crate dependencies.

---

## File Structure

| File | Responsibility |
|------|---------------|
| `src/support/collection.rs` | **New** — `Collection<T>` struct with all tier methods + unit tests |
| `src/support/mod.rs` | Add `pub mod collection` + re-export |
| `src/database/collection_ext.rs` | **New** — `ModelCollectionExt` trait for ORM load operations |
| `src/database/mod.rs` | Add `pub mod collection_ext` + re-export |
| `src/database/query.rs` | Change return types: `Vec<T>` → `Collection<T>` |
| `src/lib.rs` | Add `Collection` and `ModelCollectionExt` re-exports |

---

### Task 1: Collection struct + interop + basic methods

**Files:**
- Create: `src/support/collection.rs`
- Modify: `src/support/mod.rs`

- [ ] **Step 1: Create `src/support/collection.rs` with struct, interop traits, and basic methods**

```rust
use std::collections::HashMap;
use std::hash::Hash;

pub struct Collection<T> {
    items: Vec<T>,
}

// ── Interoperability ──────────────────────────────────────

impl<T> From<Vec<T>> for Collection<T> {
    fn from(items: Vec<T>) -> Self {
        Self { items }
    }
}

impl<T> From<Collection<T>> for Vec<T> {
    fn from(col: Collection<T>) -> Self {
        col.items
    }
}

impl<T> FromIterator<T> for Collection<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

impl<T> IntoIterator for Collection<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<T> AsRef<[T]> for Collection<T> {
    fn as_ref(&self) -> &[T] {
        &self.items
    }
}

// ── Basic ─────────────────────────────────────────────────

impl<T> Collection<T> {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn from_vec(items: Vec<T>) -> Self {
        Self { items }
    }

    pub fn into_vec(self) -> Vec<T> {
        self.items
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }
}

impl<T: Clone> Collection<T> {
    pub fn to_vec(&self) -> Vec<T> {
        self.items.clone()
    }
}

// ── Access ────────────────────────────────────────────────

impl<T> Collection<T> {
    pub fn first(&self) -> Option<&T> {
        self.items.first()
    }

    pub fn last(&self) -> Option<&T> {
        self.items.last()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }
}

impl<T> Default for Collection<T> {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Write unit tests for basic methods**

Append to `src/support/collection.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_empty_collection() {
        let col: Collection<i32> = Collection::new();
        assert!(col.is_empty());
        assert_eq!(col.len(), 0);
    }

    #[test]
    fn from_vec_and_into_vec_roundtrip() {
        let vec = vec![1, 2, 3];
        let col = Collection::from_vec(vec.clone());
        assert_eq!(col.as_slice(), &[1, 2, 3]);
        assert_eq!(col.into_vec(), vec);
    }

    #[test]
    fn from_iterator() {
        let col: Collection<i32> = vec![10, 20, 30].into_iter().collect();
        assert_eq!(col.len(), 3);
    }

    #[test]
    fn into_iterator() {
        let col = Collection::from_vec(vec![1, 2, 3]);
        let sum: i32 = col.into_iter().sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn as_ref_gives_slice() {
        let col = Collection::from_vec(vec![4, 5, 6]);
        let slice: &[i32] = col.as_ref();
        assert_eq!(slice, &[4, 5, 6]);
    }

    #[test]
    fn first_last_get() {
        let col = Collection::from_vec(vec![10, 20, 30]);
        assert_eq!(col.first(), Some(&10));
        assert_eq!(col.last(), Some(&30));
        assert_eq!(col.get(1), Some(&20));
        assert_eq!(col.get(5), None);
    }

    #[test]
    fn empty_collection_accessors() {
        let col: Collection<i32> = Collection::new();
        assert_eq!(col.first(), None);
        assert_eq!(col.last(), None);
        assert_eq!(col.get(0), None);
    }
}
```

- [ ] **Step 3: Update `src/support/mod.rs` to register the module**

Add after `mod identifiers;`:

```rust
mod collection;
```

Add to the `pub use identifiers::{...};` block, or add a new line:

```rust
pub use collection::Collection;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p foundry support::collection`
Expected: All 7 tests PASS.

---

### Task 2: Transform methods (map, map_into, filter, reject, flat_map)

**Files:**
- Modify: `src/support/collection.rs`

- [ ] **Step 1: Write failing tests for transform methods**

Append inside `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn map_transforms_elements() {
        let col = Collection::from_vec(vec![1, 2, 3]);
        let doubled = col.map(|x| x * 2);
        assert_eq!(doubled.into_vec(), vec![2, 4, 6]);
    }

    #[test]
    fn map_into_consumes_elements() {
        let col = Collection::from_vec(vec!["hello".to_string(), "world".to_string()]);
        let lengths = col.map_into(|s| s.len());
        assert_eq!(lengths.into_vec(), vec![5, 5]);
    }

    #[test]
    fn filter_keeps_matching() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let evens = col.filter(|x| x % 2 == 0);
        assert_eq!(evens.into_vec(), vec![2, 4]);
    }

    #[test]
    fn reject_removes_matching() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let odds = col.reject(|x| x % 2 == 0);
        assert_eq!(odds.into_vec(), vec![1, 3, 5]);
    }

    #[test]
    fn flat_map_flattens() {
        let col = Collection::from_vec(vec![1, 2, 3]);
        let expanded = col.flat_map(|x| vec![x, x * 10]);
        assert_eq!(expanded.into_vec(), vec![1, 10, 2, 20, 3, 30]);
    }

    #[test]
    fn filter_empty_result() {
        let col = Collection::from_vec(vec![1, 3, 5]);
        let evens = col.filter(|x| x % 2 == 0);
        assert!(evens.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p foundry support::collection -- map_ filter_ reject_ flat_`
Expected: FAIL — methods don't exist yet.

- [ ] **Step 3: Implement transform methods**

Add to the `impl<T> Collection<T>` block (after the Access section):

```rust
    // ── Transform ────────────────────────────────────────────

    pub fn map<U>(self, f: impl Fn(&T) -> U) -> Collection<U> {
        Collection::from_vec(self.items.iter().map(f).collect())
    }

    pub fn map_into<U>(self, f: impl Fn(T) -> U) -> Collection<U> {
        Collection::from_vec(self.items.into_iter().map(f).collect())
    }

    pub fn filter(self, f: impl Fn(&T) -> bool) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().filter(f).collect())
    }

    pub fn reject(self, f: impl Fn(&T) -> bool) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().filter(|x| !f(x)).collect())
    }

    pub fn flat_map<U>(self, f: impl Fn(T) -> Vec<U>) -> Collection<U> {
        Collection::from_vec(
            self.items
                .into_iter()
                .flat_map(|x| f(x).into_iter())
                .collect(),
        )
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p foundry support::collection`
Expected: All tests PASS (13 total).

---

### Task 3: Query methods (find, first_where, any, all, count_where)

**Files:**
- Modify: `src/support/collection.rs`

- [ ] **Step 1: Write failing tests for query methods**

```rust
    #[test]
    fn find_returns_reference() {
        let col = Collection::from_vec(vec![10, 20, 30]);
        let found = col.find(|x| **x == 20);
        assert_eq!(found, Some(&20));
    }

    #[test]
    fn find_returns_none() {
        let col = Collection::from_vec(vec![10, 20, 30]);
        assert_eq!(col.find(|x| **x == 99), None);
    }

    #[test]
    fn first_where_returns_owned() {
        let col = Collection::from_vec(vec![10, 20, 30]);
        assert_eq!(col.first_where(|x| *x > 15), Some(20));
    }

    #[test]
    fn any_checks_existence() {
        let col = Collection::from_vec(vec![1, 2, 3]);
        assert!(col.any(|x| *x == 2));
        assert!(!col.any(|x| *x == 5));
    }

    #[test]
    fn all_checks_every() {
        let col = Collection::from_vec(vec![2, 4, 6]);
        assert!(col.all(|x| x % 2 == 0));
        assert!(!col.all(|x| *x > 4));
    }

    #[test]
    fn count_where_counts_matching() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        assert_eq!(col.count_where(|x| x % 2 == 0), 2);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p foundry support::collection -- find_ first_where any all count_where`
Expected: FAIL.

- [ ] **Step 3: Implement query methods**

Add to the `impl<T> Collection<T>` block:

```rust
    // ── Query ────────────────────────────────────────────────

    pub fn find(&self, f: impl Fn(&T) -> bool) -> Option<&T> {
        self.items.iter().find(f)
    }

    pub fn first_where(self, f: impl Fn(&T) -> bool) -> Option<T> {
        self.items.into_iter().find(f)
    }

    pub fn any(&self, f: impl Fn(&T) -> bool) -> bool {
        self.items.iter().any(f)
    }

    pub fn all(&self, f: impl Fn(&T) -> bool) -> bool {
        self.items.iter().all(f)
    }

    pub fn count_where(&self, f: impl Fn(&T) -> bool) -> usize {
        self.items.iter().filter(|x| f(x)).count()
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p foundry support::collection`
Expected: All tests PASS (19 total).

---

### Task 4: High-value backend helpers (pluck, key_by, group_by, unique_by, partition_by, chunk)

**Files:**
- Modify: `src/support/collection.rs`

- [ ] **Step 1: Write failing tests for backend helpers**

```rust
    #[test]
    fn pluck_extracts_field() {
        #[derive(Clone, Debug)]
        struct User {
            id: i64,
            name: String,
        }
        let col = Collection::from_vec(vec![
            User { id: 1, name: "Alice".into() },
            User { id: 2, name: "Bob".into() },
        ]);
        let ids = col.pluck(|u| u.id);
        assert_eq!(ids.into_vec(), vec![1, 2]);
    }

    #[test]
    fn key_by_builds_hashmap() {
        let col = Collection::from_vec(vec![("a", 1), ("b", 2), ("c", 3)]);
        let map = col.key_by(|(k, _)| k.to_string());
        assert_eq!(map.get("a"), Some(&("a", 1)));
        assert_eq!(map.get("b"), Some(&("b", 2)));
    }

    #[test]
    fn group_by_buckets() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5, 6]);
        let grouped = col.group_by(|x| *x % 2);
        assert_eq!(grouped.get(&0).map(|c| c.len()), Some(3)); // 2,4,6
        assert_eq!(grouped.get(&1).map(|c| c.len()), Some(3)); // 1,3,5
    }

    #[test]
    fn unique_by_deduplicates() {
        let col = Collection::from_vec(vec![(1, "a"), (2, "b"), (1, "c"), (3, "d"), (2, "e")]);
        let unique = col.unique_by(|(k, _)| *k);
        assert_eq!(unique.len(), 3);
        assert_eq!(unique.into_vec(), vec![(1, "a"), (2, "b"), (3, "d")]);
    }

    #[test]
    fn partition_by_splits() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let (evens, odds) = col.partition_by(|x| x % 2 == 0);
        assert_eq!(evens.into_vec(), vec![2, 4]);
        assert_eq!(odds.into_vec(), vec![1, 3, 5]);
    }

    #[test]
    fn chunk_splits_into_batches() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let chunks = col.chunk(2);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.get(0).unwrap().as_slice(), &[1, 2]);
        assert_eq!(chunks.get(1).unwrap().as_slice(), &[3, 4]);
        assert_eq!(chunks.get(2).unwrap().as_slice(), &[5]);
    }

    #[test]
    fn chunk_exactly_divisible() {
        let col = Collection::from_vec(vec![1, 2, 3, 4]);
        let chunks = col.chunk(2);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks.get(0).unwrap().as_slice(), &[1, 2]);
        assert_eq!(chunks.get(1).unwrap().as_slice(), &[3, 4]);
    }

    #[test]
    fn chunk_empty_collection() {
        let col: Collection<i32> = Collection::new();
        let chunks = col.chunk(3);
        assert!(chunks.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p foundry support::collection -- pluck_ key_by group_by unique_by partition_ chunk`
Expected: FAIL.

- [ ] **Step 3: Implement backend helpers**

Add to the `impl<T> Collection<T>` block:

```rust
    // ── High-Value Backend Helpers ───────────────────────────

    pub fn pluck<U>(self, f: impl Fn(&T) -> U) -> Collection<U> {
        Collection::from_vec(self.items.iter().map(f).collect())
    }

    pub fn key_by<K>(self, f: impl Fn(&T) -> K) -> HashMap<K, T>
    where
        K: Eq + Hash,
    {
        self.items
            .into_iter()
            .map(|item| {
                let key = f(&item);
                (key, item)
            })
            .collect()
    }

    pub fn group_by<K>(self, f: impl Fn(&T) -> K) -> HashMap<K, Collection<T>>
    where
        K: Eq + Hash,
    {
        let mut map: HashMap<K, Vec<T>> = HashMap::new();
        for item in self.items {
            let key = f(&item);
            map.entry(key).or_default().push(item);
        }
        map.into_iter()
            .map(|(key, items)| (key, Collection::from_vec(items)))
            .collect()
    }

    pub fn unique_by<K>(self, f: impl Fn(&T) -> K) -> Collection<T>
    where
        K: Eq + Hash,
    {
        let mut seen = std::collections::HashSet::new();
        let mut unique = Vec::new();
        for item in self.items {
            let key = f(&item);
            if seen.insert(key) {
                unique.push(item);
            }
        }
        Collection::from_vec(unique)
    }

    pub fn partition_by(self, f: impl Fn(&T) -> bool) -> (Collection<T>, Collection<T>) {
        let mut matching = Vec::new();
        let mut rest = Vec::new();
        for item in self.items {
            if f(&item) {
                matching.push(item);
            } else {
                rest.push(item);
            }
        }
        (Collection::from_vec(matching), Collection::from_vec(rest))
    }

    pub fn chunk(&self, size: usize) -> Collection<Collection<T>>
    where
        T: Clone,
    {
        if size == 0 || self.items.is_empty() {
            return Collection::new();
        }
        Collection::from_vec(
            self.items
                .chunks(size)
                .map(|chunk| Collection::from_vec(chunk.to_vec()))
                .collect(),
        )
    }
```

Also add this import at the top of the file if not already present:

```rust
use std::collections::HashMap;
use std::hash::Hash;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p foundry support::collection`
Expected: All tests PASS (27 total).

---

### Task 5: Ordering, aggregation, and utility methods (Tiers 2-3)

**Files:**
- Modify: `src/support/collection.rs`

- [ ] **Step 1: Write failing tests**

```rust
    // ── Tier 2: Ordering ──────────────────────────────────

    #[test]
    fn sort_by_orders_elements() {
        let mut col = Collection::from_vec(vec![3, 1, 2]);
        col.sort_by(|a, b| a.cmp(b));
        assert_eq!(col.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn sort_by_key_orders_by_derived_key() {
        let mut col = Collection::from_vec(vec!["banana", "apple", "cherry"]);
        col.sort_by_key(|s| s.len());
        assert_eq!(col.into_vec(), vec!["apple", "banana", "cherry"]);
    }

    #[test]
    fn reverse_flips_order() {
        let mut col = Collection::from_vec(vec![1, 2, 3]);
        col.reverse();
        assert_eq!(col.into_vec(), vec![3, 2, 1]);
    }

    // ── Tier 2: Aggregation ───────────────────────────────

    #[test]
    fn sum_by_adds_values() {
        let col = Collection::from_vec(vec![1, 2, 3, 4]);
        let total: i32 = col.sum_by(|x| *x);
        assert_eq!(total, 10);
    }

    #[test]
    fn min_by_finds_minimum() {
        let col = Collection::from_vec(vec!["banana", "apple", "cherry"]);
        assert_eq!(col.min_by(|s| s.len()), Some(5)); // "apple"
    }

    #[test]
    fn max_by_finds_maximum() {
        let col = Collection::from_vec(vec!["banana", "apple", "cherry"]);
        assert_eq!(col.max_by(|s| s.len()), Some(6)); // "banana" or "cherry"
    }

    #[test]
    fn min_by_empty_returns_none() {
        let col: Collection<i32> = Collection::new();
        assert_eq!(col.min_by(|x| *x), None);
    }

    // ── Tier 3: Utilities ─────────────────────────────────

    #[test]
    fn take_first_n() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        assert_eq!(col.take(3).into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn skip_first_n() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        assert_eq!(col.skip(2).into_vec(), vec![3, 4, 5]);
    }

    #[test]
    fn for_each_consumes() {
        let col = Collection::from_vec(vec![1, 2, 3]);
        let mut sum = 0;
        col.for_each(|x| sum += x);
        assert_eq!(sum, 6);
    }

    #[test]
    fn tap_inspects_without_consuming() {
        let col = Collection::from_vec(vec![1, 2, 3]);
        let mut seen = 0;
        let same = col.tap(|c| seen = c.len());
        assert_eq!(seen, 3);
        assert_eq!(same.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn pipe_chains_transform() {
        let col = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let result = col.pipe(|c| c.filter(|x| *x > 2));
        assert_eq!(result.into_vec(), vec![3, 4, 5]);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p foundry support::collection -- sort_ reverse sum_ min_ max_ take skip for_each tap pipe`
Expected: FAIL.

- [ ] **Step 3: Implement Tier 2 and 3 methods**

Add to `impl<T> Collection<T>`:

```rust
    // ── Tier 2: Ordering ─────────────────────────────────────

    pub fn sort_by(&mut self, f: impl Fn(&T, &T) -> std::cmp::Ordering) {
        self.items.sort_by(f);
    }

    pub fn sort_by_key<K>(&mut self, f: impl Fn(&T) -> K)
    where
        K: Ord,
    {
        self.items.sort_by_key(f);
    }

    pub fn reverse(&mut self) {
        self.items.reverse();
    }

    // ── Tier 2: Aggregation ──────────────────────────────────

    pub fn sum_by<U>(self, f: impl Fn(&T) -> U) -> U
    where
        U: std::iter::Sum,
    {
        self.items.iter().map(f).sum()
    }

    pub fn min_by<U>(self, f: impl Fn(&T) -> U) -> Option<U>
    where
        U: Ord,
    {
        self.items.iter().map(f).min()
    }

    pub fn max_by<U>(self, f: impl Fn(&T) -> U) -> Option<U>
    where
        U: Ord,
    {
        self.items.iter().map(f).max()
    }

    // ── Tier 3: Utilities ────────────────────────────────────

    pub fn take(self, n: usize) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().take(n).collect())
    }

    pub fn skip(self, n: usize) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().skip(n).collect())
    }

    pub fn for_each(self, f: impl Fn(T)) {
        self.items.into_iter().for_each(f);
    }

    pub fn tap(self, f: impl Fn(&Collection<T>)) -> Collection<T> {
        f(&self);
        self
    }

    pub fn pipe(self, f: impl Fn(Collection<T>) -> Collection<T>) -> Collection<T> {
        f(self)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p foundry support::collection`
Expected: All tests PASS (39 total).

---

### Task 6: Register Collection in lib.rs

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Add Collection to the support module**

No change needed in `src/support/mod.rs` — already done in Task 1.

- [ ] **Step 2: Add re-export in `src/lib.rs`**

Add `Collection` to the existing `pub use support::{...}` line, or add a new line:

```rust
pub use support::Collection;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p foundry`
Expected: PASS — no errors.

---

### Task 7: ORM extension trait (ModelCollectionExt)

**Files:**
- Create: `src/database/collection_ext.rs`
- Modify: `src/database/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/database/collection_ext.rs`**

```rust
use crate::foundation::Result;
use crate::support::Collection;

use super::model::{Model, PersistedModel};
use super::relation::{AnyRelation, RelationDef};
use super::runtime::{DbValue, QueryExecutor};

pub trait ModelCollectionExt<T>: Sized {
    fn model_keys(&self) -> Collection<DbValue>;
    async fn load<E>(
        self,
        relation: impl IntoLoadableRelation<T>,
        executor: &E,
    ) -> Result<Collection<T>>
    where
        E: QueryExecutor;

    async fn load_missing<E>(
        self,
        relation: impl IntoLoadableRelation<T>,
        executor: &E,
    ) -> Result<Collection<T>>
    where
        E: QueryExecutor;
}

/// Trait to abstract over `RelationDef` and `ManyToManyDef` so `load()` accepts both.
pub trait IntoLoadableRelation<M: Model>: Send + Sync {
    fn into_relation(self) -> AnyRelation<M>;
}

impl<M, To> IntoLoadableRelation<M> for RelationDef<M, To>
where
    M: Model,
    To: Model,
{
    fn into_relation(self) -> AnyRelation<M> {
        std::sync::Arc::new(self)
    }
}
```

Note: `load` and `load_missing` implementations need to call the existing `hydrate_model_batch` pattern. The full implementation will convert the Collection to `&mut [M]`, call the relation loader, and return the mutated Collection. This requires the exact relation API — implement the body by following the pattern in `hydrate_model_batch` in `src/database/query.rs:3316-3351`.

- [ ] **Step 2: Register module in `src/database/mod.rs`**

Add after existing module declarations:

```rust
mod collection_ext;
```

Add to public exports:

```rust
pub use collection_ext::{IntoLoadableRelation, ModelCollectionExt};
```

- [ ] **Step 3: Add re-export in `src/lib.rs`**

```rust
pub use database::ModelCollectionExt;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p foundry`
Expected: PASS — no errors.

---

### Task 8: Change query return types from Vec<T> to Collection<T>

**Files:**
- Modify: `src/database/query.rs`

This is the integration step. The following methods change return type:

- [ ] **Step 1: Add Collection import in `src/database/query.rs`**

At the top of the file, add:

```rust
use crate::support::Collection;
```

- [ ] **Step 2: Change `ModelQuery::get()` return type**

In `src/database/query.rs`, find the `get` method on `ModelQuery` (~line 1671):

Change from:
```rust
    pub async fn get<E>(&self, executor: &E) -> Result<Vec<M>>
```

To:
```rust
    pub async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
```

Change the body from:
```rust
        let mut entries = self.fetch_entries_dyn(executor).await?;
        Ok(entries.drain(..).map(|(_, model)| model).collect())
```

To:
```rust
        let mut entries = self.fetch_entries_dyn(executor).await?;
        Ok(entries.drain(..).map(|(_, model)| model).collect())
```

(This already works because `Collection` implements `FromIterator`.)

- [ ] **Step 3: Change `ProjectionQuery::get()` return type**

Find `ProjectionQuery::get()` (~line 1291):

Change from:
```rust
    pub async fn get<E>(&self, executor: &E) -> Result<Vec<P>>
```

To:
```rust
    pub async fn get<E>(&self, executor: &E) -> Result<Collection<P>>
```

Body stays the same — `.collect()` works via `FromIterator`.

- [ ] **Step 4: Change `Paginated.data` field type**

In `src/database/query.rs`, find `Paginated` struct (~line 50):

Change from:
```rust
pub struct Paginated<T> {
    pub data: Vec<T>,
```

To:
```rust
pub struct Paginated<T> {
    pub data: Collection<T>,
```

- [ ] **Step 5: Update `ModelQuery::paginate()` body**

The paginate method constructs `Paginated { data, ... }` where `data` is already the result of `.get()` which now returns `Collection<M>`. The same applies to `ProjectionQuery::paginate()`. Both should compile without body changes.

- [ ] **Step 6: Update `ModelQuery::stream()` internals**

The stream method returns `BoxStream<'a, Result<M>>` — this stays as-is since streams yield individual items, not collections.

- [ ] **Step 7: Verify it compiles**

Run: `cargo check -p foundry`
Expected: PASS — any remaining compile errors are in test files that assert `Vec<T>`. Those get fixed in Task 9.

---

### Task 9: Fix existing tests for new Collection return types

**Files:**
- Modify: `tests/database_acceptance.rs` and any other test files that call `.get()`

- [ ] **Step 1: Find all test code using `.get()` on query types**

Run: `grep -rn "\.get(.*executor\|\.get(.*database" tests/`

For each match where the result is used as `Vec<T>`:
- If the test iterates or calls `.len()`, no change needed (Collection supports those).
- If the test calls `.into_iter().next()` for `first()`, no change needed.
- If the test explicitly types as `Vec<M>` or calls Vec-specific methods, add `.into_vec()`.

- [ ] **Step 2: Run full test suite**

Run: `cargo test -p foundry`
Expected: All tests PASS. Some tests requiring Postgres may be skipped if no `FOUNDRY_TEST_POSTGRES_URL` is set — that's expected.

---

### Task 10: Final lib.rs re-exports cleanup

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Verify all re-exports are in place**

In `src/lib.rs`, ensure these are present:

```rust
pub use support::Collection;
pub use database::ModelCollectionExt;
```

If `IntoLoadableRelation` is useful at the app level, also add:

```rust
pub use database::IntoLoadableRelation;
```

- [ ] **Step 2: Verify prelude**

Check `src/prelude/mod.rs` and add `Collection` if it exports other support types.

- [ ] **Step 3: Final compile check**

Run: `cargo check -p foundry`
Expected: PASS — clean build.

---

## Verification

After all tasks are complete, verify end-to-end:

1. **Unit tests:** `cargo test -p foundry support::collection` — all ~39 tests pass
2. **Full build:** `cargo check -p foundry` — clean
3. **Clippy:** `cargo clippy -p foundry` — no warnings
4. **Existing tests:** `cargo test -p foundry` — no regressions
5. **Usage example compiles:**

```rust
use foundry::Collection;

let users = Collection::from_vec(vec![
    (1, "Alice".to_string()),
    (2, "Bob".to_string()),
    (3, "Charlie".to_string()),
]);

let ids = users.pluck(|(id, _)| *id);
let grouped = users.group_by(|(_, name)| name.len());
```
