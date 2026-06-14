# Rust Query System Blueprint (v1 → v3)

> Status note (April 12, 2026): the v1-v3 query blueprint described here is complete in Foundry. See [docs/query-blueprint-status.md](docs/query-blueprint-status.md) for the implementation map, canonical examples, and acceptance coverage. Post-blueprint model DX is now stricter than this historical spec in a few places, including safe-by-default `ModelId<M>` UUIDv7 primary keys serialized as strings, built-in timestamps/soft deletes, model lifecycle hooks, and same-type field write mutators plus explicit read accessor methods.

## Overview

This document defines the design of a **strongly-typed, scalable query system** for a Rust backend framework.

Goals:

- Strong typing
- SQL generation (not replacement)
- Relationship support
- Tree-based eager loading
- No hardcoded nesting limits
- Minimal reliance on codegen
- Raw SQL escape hatch always available

---

# Core Philosophy

1. SQL is the source of truth
2. Rust provides type safety and composition
3. Relationships are explicit (not magic)
4. Eager loading is tree-based
5. Codegen assists, not dominates

---

# ⚠️ CRITICAL: AST-FIRST DEVELOPMENT STRATEGY

## Before Anything Else

This query system MUST be built **AST-first**, not API-first.

### Why this matters

The real architecture is:

```
Builder API
   ↓
AST (source of truth)
   ↓
SQL Generator
   ↓
Database
```

If AST is wrong:

- Query builder becomes messy
- Relationships become hacky
- Eager loading becomes unscalable
- Typing becomes inconsistent
- SQL generation breaks under complexity

---

## Development Order (MANDATORY)

### Step 1 — Design AST (FIRST)

Define core structures:

- Column
- Expr
- Condition (AND / OR tree)
- QueryNode (SELECT / INSERT / UPDATE / DELETE)
- JoinNode
- RelationNode (tree-based)
- AggregateNode

This is the **foundation of everything**.

---

### Step 2 — Builder API (SECOND)

Example:

```rust
User::query()
    .where_(User::STATUS.eq(...))
    .with(User::merchants())
```

Builder is only a **UI layer over AST**.

---

### Step 3 — SQL Generator (THIRD)

Convert AST → SQL + bindings.

---

### Step 4 — Eager Loading Engine (FOURTH)

Use RelationNode tree to:

- batch queries
- avoid N+1
- hydrate nested structures

---

### Step 5 — Refine API & Docs (LAST)

Only after AST is stable.

---

## What This Blueprint Originally Missed

This document previously described:

- v1 → v3 phases
- relation tree
- eager loading

But did NOT define:

- AST structure
- expression system
- condition tree
- query representation

👉 Without AST, system is not implementable.

---

## Action Plan (Next Step)

Before implementing anything:

> Design AST v1 in detail

Then:

- map builder → AST
- map AST → SQL
- map relation tree → execution

---

## One-Line Rule

> Do NOT design API first. Design AST first.

---

# FINAL TARGET (v3 EXPERIENCE)

## Example Query

```rust
User::query()
    .with(
        User::merchants().with(
            Merchant::orders().with(
                Order::items().with(
                    OrderItem::product()
                )
            )
        )
    )
    .where_has(User::merchants(), |q| {
        q.where_(Merchant::STATUS.eq(MerchantStatus::Active))
    })
    .get(&db)
    .await?;
```

---

# RELATION TREE DESIGN

Internally:

```
User
└── merchants
    └── orders
        └── items
            └── product
```

## Key Properties

- Recursive
- Unlimited nesting (logical)
- No hardcoded depth
- Supports filters, aggregates, nested children

---

# PHASE 1 — Foundation (v1)

## Goal

Basic SQL execution + lightweight builder

## Features

- Raw SQL execution
- Parameter binding
- Basic query builder
- Transactions
- Pagination helper

## Example

```rust
db.query("SELECT * FROM users WHERE id = ?", &[id]).await?;
```

## Builder Example

```rust
Query::table("users")
    .select(["id", "name"])
    .where_eq("status", "active")
    .limit(20)
    .get(&db)
    .await?;
```

## Architecture

```
database/
├── connection.rs
├── query.rs
├── builder.rs
├── executor.rs
├── transaction.rs
```

---

# PHASE 2 — Typed Query Builder (v2)

## Goal

Introduce type safety + model awareness

## Features

- Typed columns
- Generated model metadata
- Basic relationships
- Insert/update structs

## Example

```rust
User::query()
    .where_(User::STATUS.eq(UserStatus::Active))
    .get(&db)
    .await?;
```

## Codegen Scope

Codegen should provide:

- Column definitions
- Table metadata
- Basic struct types

Example:

```rust
pub struct User;

impl User {
    pub const ID: Column<i64>;
    pub const NAME: Column<String>;
}
```

## Relationships (Manual)

```rust
impl User {
    pub fn merchants() -> HasMany<Merchant> {
        has_many(Merchant::USER_ID, User::ID)
    }
}
```

## Architecture

```
database/
├── ast/
├── builder/
├── column/
├── relation/
├── model/
├── codegen/
```

---

# PHASE 3 — Advanced ORM Layer (v3)

## Goal

Full relationship + eager loading system

## Features

- Nested eager loading
- where\_has
- Aggregates (count, sum)
- Relation scopes
- Relation tree execution

---

# EAGER LOADING ENGINE

## Flow

```
Main Query
   ↓
Build Relation Tree
   ↓
Execute Root Query
   ↓
Batch Load Relations
   ↓
Hydrate Nested Results
```

---

# INTERNAL RELATION NODE

```rust
struct RelationNode {
    name: String,
    kind: RelationKind,
    target: Model,
    filters: Vec<Condition>,
    children: Vec<RelationNode>,
}
```

---

# CODEGEN vs HANDWRITTEN

## Codegen SHOULD provide:

- Typed columns
- Table metadata
- Default models

## Codegen SHOULD NOT dominate:

- Relationship semantics
- Business-specific relations

---

## Handwritten SHOULD define:

```rust
impl User {
    pub fn merchants() -> HasMany<Merchant>

    pub fn active_merchants() -> HasMany<Merchant>
}
```

---

# RAW SQL ESCAPE HATCH

Always supported:

```rust
db.raw_query("SELECT * FROM users WHERE status = ?", &[status]).await?;
```

---

# DESIGN PRINCIPLES

1. No N+1 queries
2. SQL remains debuggable
3. Relations are explicit
4. Tree-based loading
5. Strong typing where useful

---

# FINAL SUMMARY

This system evolves from:

- v1: SQL execution
- v2: Typed builder
- v3: Full relation engine

---

# FINAL STATEMENT

> Build a system that feels like Laravel/Eloquent, but behaves like Rust: explicit, typed, and predictable.
