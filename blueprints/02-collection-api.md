# Rust Collection API Blueprint (Framework-Level)

## Overview

This document defines a **Collection<T> design** for a Rust backend framework.

Goal:

> Provide real, practical value beyond Vec<T>, especially for backend and ORM use-cases.

---

# Core Principle

## DO NOT replace Vec<T>

Collection is:

- A wrapper over Vec<T>
- An ergonomic utility layer
- Focused on real backend needs

---

# Design Philosophy

1. Must provide real value beyond iterators
2. Must remain strongly typed
3. Must interoperate easily with Vec<T>
4. Must NOT duplicate std unnecessarily
5. Must support both generic and ORM use-cases

---

# Type Definition

```rust
pub struct Collection<T> {
    items: Vec<T>,
}
```

---

# Interoperability (MANDATORY)

- From<Vec<T>>
- Into<Vec<T>>
- IntoIterator
- AsRef<[T]>

---

# Tier 1 — Core & High Value Methods

## Basic

- new() → create empty collection
- from_vec(vec)
- into_vec()
- as_slice()
- len()
- is_empty()

---

## Access

- first() → Option<&T>
- last() → Option<&T>
- get(index)

---

## Transform

- map(f)
- map_into(f)
- filter(f)
- reject(f)
- flat_map(f)

---

## Query

- find(f)
- first_where(f)
- any(f)
- all(f)
- count_where(f)

---

## High-Value Backend Helpers (CORE VALUE)

### pluck
Extract field/value

Example:
```rust
users.pluck(|u| u.id)
```

---

### key_by
Convert to HashMap

```rust
users.key_by(|u| u.id)
```

---

### group_by
Group into buckets

```rust
orders.group_by(|o| o.user_id)
```

---

### unique_by
Remove duplicates

```rust
users.unique_by(|u| u.email.clone())
```

---

### partition_by
Split into two collections

```rust
let (active, inactive) = users.partition_by(|u| u.active);
```

---

### chunk
Split into batches

```rust
users.chunk(100)
```

---

# Tier 2 — Useful Extensions

## Ordering

- sort_by(f)
- sort_by_key(f)
- reverse()

---

## Aggregation

- sum_by(f)
- avg_by(f)
- min_by(f)
- max_by(f)

---

# Tier 3 — Optional

- take(n)
- skip(n)
- for_each(f)
- tap(f)
- pipe(f)

---

# ORM / Model Extensions (IMPORTANT VALUE)

These MUST NOT be in base Collection<T>.

Use extension traits.

## Methods

### load(relation)
Eager load relation

```rust
users.load(User::merchants()).await?
```

---

### load_missing(relation)
Load only if not loaded

---

### model_keys()
Extract primary keys

---

# Strong Typing Examples

```rust
Collection<User>
Collection<String>
Collection<i64>
```

## Example

```rust
let users = User::query().get(&db).await?;

let ids = users.pluck(|u| u.id);
let grouped = users.group_by(|u| u.role.clone());
let map = users.key_by(|u| u.id);
```

---

# When Collection is Worth It

Collection is justified ONLY if you implement:

- key_by
- group_by
- pluck
- unique_by
- partition_by
- map_into
- ORM extensions (load)

---

# When NOT to Use Collection

DO NOT build Collection if:

- It only wraps Vec
- It duplicates iterators
- It adds no backend-specific value

---

# Final Recommendation

Use Collection as:

> A high-level ergonomic layer for query results and backend transformations

NOT as:

> A replacement for Vec<T>

---

# Final Statement

> Only build Collection if it removes real backend boilerplate and improves model/query workflows.
