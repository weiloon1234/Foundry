# Query Blueprint Status

The query-system blueprint in [query-system-v1-v3](../blueprints/01-query-system-v1-v3.md) is complete in Foundry.

This status note maps the blueprint's v1-v3 goals to the concrete surfaces, examples, and acceptance coverage already in the repo. It exists so the blueprint can remain a stable design record while contributors can quickly see what implements it today.

## Status

- Blueprint scope: complete
- First-class target: Postgres query system
- Post-blueprint DB work: Rust migration/seeder lifecycle, runtime hardening, safe-by-default `ModelId<M>` UUIDv7 primary keys, built-in timestamps/soft deletes, model lifecycle hooks, and field mutators/accessors are implemented separately and are not part of the v1-v3 query blueprint scope

## Blueprint Mapping

### AST-First Foundation

Foundry implements the AST-first architecture described in the blueprint through the public database AST and compiler surface:

- `database::ast` with `QueryAst`, `QueryBody`, `Expr`, `Condition`, `JoinNode`, `RelationNode`, and aggregate/window/set-operation nodes
- `PostgresCompiler` and `to_compiled_sql()` on query surfaces
- `Query`, `ModelQuery`, and `ProjectionQuery` as builder layers over AST, not SQL-string builders

References:

- Generic builder example: [examples/phase3_database_generic.rs](../examples/phase3_database_generic.rs)
- Projection/advanced AST example: [examples/phase3_database_projection.rs](../examples/phase3_database_projection.rs)
- Acceptance coverage: [tests/database_acceptance.rs](../tests/database_acceptance.rs) `advanced_projection_queries_support_cte_case_json_union_and_numeric_aggregates`

### Phase 1 — Foundation (v1)

The blueprint's v1 scope is covered by:

- raw SQL execution through `DatabaseManager` and `QueryExecutor`
- parameter binding and transactions
- generic `Query::table(...)` builder
- pagination helpers and compiled-SQL inspection

References:

- Example: [examples/phase3_database_generic.rs](../examples/phase3_database_generic.rs)
- Acceptance coverage: [tests/database_acceptance.rs](../tests/database_acceptance.rs) `typed_runtime_supports_production_postgres_values_and_custom_adapters`

### Phase 2 — Typed Query Builder (v2)

The blueprint's typed model/codegen layer is covered by:

- `Model`, `Column`, `TableMeta`, `ModelQuery`
- `CreateModel`, `CreateManyModel`, `UpdateModel`
- safe-by-default `ModelId<M>` UUIDv7 primary keys serialized as strings on the model surface
- built-in timestamp and soft-delete conventions on the model write/query layer
- field-level write mutators and explicit generated read accessor methods on derived models
- `Projection`, `ProjectionField`, `ProjectionQuery`
- derive-assisted metadata via `foundry::Model` and `foundry::Projection`
- explicit handwritten relations layered on top of generated metadata

References:

- Typed model example: [examples/phase3_database_model.rs](../examples/phase3_database_model.rs)
- Projection example: [examples/phase3_database_projection.rs](../examples/phase3_database_projection.rs)
- Derive and compile coverage: [tests/derive_ui.rs](../tests/derive_ui.rs)

### Phase 3 — Advanced ORM Layer (v3)

The blueprint's final target behavior is covered by:

- recursive relation-tree eager loading
- `where_has`
- relation aggregates
- `Loaded<T>` hydration
- explicit many-to-many definitions with optional pivot projection hydration

Canonical final-target example:

- [examples/phase3_database_relations.rs](../examples/phase3_database_relations.rs)

Supporting examples:

- Many-to-many and pivot data: [examples/phase3_database_many_to_many.rs](../examples/phase3_database_many_to_many.rs)

Acceptance coverage:

- Relation tree and unlimited-depth eager loading: [tests/database_acceptance.rs](../tests/database_acceptance.rs) `relation_tree_eager_loads_without_hardcoded_depth`
- Many-to-many and relation aggregates: [tests/database_acceptance.rs](../tests/database_acceptance.rs) `many_to_many_relations_load_pivot_data_and_aggregates`
- Typed projections with CTE/UNION/CASE/JSON/aggregates: [tests/database_acceptance.rs](../tests/database_acceptance.rs) `advanced_projection_queries_support_cte_case_json_union_and_numeric_aggregates`

### Codegen vs Handwritten Boundary

Foundry matches the blueprint's intended boundary:

- codegen assists metadata, columns, and projections
- codegen also binds field mutator/accessor metadata into the model layer
- writes use model-first fluent builders instead of dedicated payload structs
- relationships remain handwritten and explicit in application code
- business-specific relation semantics are not generated

References:

- Manual relation definitions: [examples/phase3_database_relations.rs](../examples/phase3_database_relations.rs)
- Many-to-many relation definitions: [examples/phase3_database_many_to_many.rs](../examples/phase3_database_many_to_many.rs)
- Proc-macro implementation: [foundry-macros](../foundry-macros)

### Raw SQL Escape Hatch

Foundry keeps raw SQL permanently available through:

- `DatabaseManager::raw_query(...)`
- `DatabaseManager::raw_execute(...)`
- `QueryExecutor` raw execution helpers

References:

- Runtime/value acceptance coverage: [tests/database_acceptance.rs](../tests/database_acceptance.rs) `typed_runtime_supports_production_postgres_values_and_custom_adapters`

## Post-Blueprint Features

The blueprint is intentionally narrower than the full current database platform.

The following are implemented in Foundry but are post-blueprint work and should not be treated as unfinished blueprint scope:

### Cursor Pagination

Fully implemented alongside offset pagination:

- `CursorPagination`, `CursorPaginated<T>`, `CursorMeta`, `CursorInfo`
- `ModelQuery::cursor_paginate()` with opaque versioned `(sort value, primary key)` cursor tokens and deterministic forward/backward ordering

### Streaming

Memory-efficient record iteration with eager relation support:

- `DbRecordStream<'a>` type alias
- `.stream()` on `Query`, `ModelQuery`, and `ProjectionQuery`
- `stream_records()` on the `QueryExecutor` trait
- `.with_stream_batch_size()` option on `ModelQuery`

### JSON Path Expressions

Full JSON querying support:

- `JsonPathExpr`, `JsonPathSegment`, `JsonPathMode` in AST
- `JsonExprBuilder` with `.as_json()`, `.contains()`, `.contained_by()`

### Window Functions

Full window function support:

- `WindowExpr`, `WindowSpec`, `WindowFrame`, `WindowFrameUnits`, `WindowFrameBound` in AST
- `WindowBuilder` with `.partition_by()`, `.order_by()`, `.rows_between()`, `.range_between()`

### Row Locking

Pessimistic locking for concurrent access:

- `LockClause`, `LockStrength` (`Update`, `NoKeyUpdate`, `Share`, `KeyShare`), `LockBehavior` (`Wait`, `NoWait`, `SkipLocked`)
- `.for_update()`, `.for_no_key_update()`, `.for_share()`, `.for_key_share()`, `.skip_locked()`, `.nowait()` on `Query`

### UPSERT / ON CONFLICT

Full upsert support on insert builders:

- `OnConflictNode`, `OnConflictTarget` (`Columns`, `Constraint`), `OnConflictAction` (`DoNothing`, `DoUpdate`)
- `.on_conflict_columns()`, `.on_conflict_constraint()`, `.do_nothing()`, `.do_update()`, `.set_conflict()`, `.set_conflict_expr()` on `CreateModel`, `CreateManyModel`, and `Query` insert builders

### Model Conventions & Lifecycle

- Rust migration and seeding lifecycle
- Runtime hardening, native streaming, and statement timeouts
- Safe-by-default `ModelId<M>` UUIDv7 primary keys, timestamps, soft deletes
- Model lifecycle hooks (creating, created, updating, updated, deleting, deleted)
- Field-level write mutators and read accessors
