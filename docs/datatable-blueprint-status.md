# Datatable Blueprint Status

The datatable-system blueprint in [datatable-system](../blueprints/06-datatable-system.md) is implemented in Foundry.

This status note maps the blueprint's goals to the concrete surfaces already in the repo. It exists so the blueprint can remain a stable design record while contributors can quickly see what implements it today.

## Status

- Blueprint scope: core implementation complete
- First-class target: model-backed datatables with JSON, download, and email export modes
- Deferred items: none

## Module Structure

All datatable modules live under `src/datatable/`:

| File | Purpose |
|------|---------|
| `mod.rs` | Module root + re-exports |
| `value.rs` | `DatatableValue` enum (Null, String, Number, Bool, Date, DateTime) |
| `column.rs` | `DatatableColumn<M>` builder with `::field()` constructor |
| `mapping.rs` | `DatatableMapping<M>` for computed output fields |
| `sort.rs` | `DatatableSort<M>` with `::asc()` / `::desc()` constructors |
| `request.rs` | `DatatableRequest`, `DatatableFilterInput`, `DatatableSortInput` |
| `filter_meta.rs` | `DatatableFilterField`, `DatatableFilterRow`, `DatatableFilterOption` |
| `filter_engine.rs` | Auto-filter application + legacy param normalization |
| `relation_filter.rs` | Typed relation-backed auto-filter declarations |
| `context.rs` | `DatatableContext` (scoped execution context) |
| `datatable_trait.rs` | Unified `Datatable` + sealed `DatatableQuery` traits |
| `response.rs` | `DatatableJsonResponse`, column/pagination meta |
| `json.rs` | JSON output mode (paginated) |
| `download.rs` | XLSX download mode (fully implemented with `rust_xlsxwriter`) |
| `export.rs` | `DatatableExportDelivery` contract + `NoopExportDelivery` |
| `export_job.rs` | Queued export dispatch (fully implemented) |
| `query_pipeline.rs` | Shared query build pipeline for JSON/download/export |
| `registry.rs` | `DatatableRegistry` + `DatatableRegistryBuilder` (type-erased lookup by ID) |

## Blueprint Mapping

### Core Types (Blueprint: Columns and Mappings)

- `DatatableColumn<Row>` with `::field(column_or_projection_field)` constructor
- Builder methods: `.sortable()`, `.sort_by()`, `.filterable()`, `.filter_by()`, `.filter_having()`, `.exportable()`, `.label()`, `.relation()`
- `DatatableMapping<M>` with `::new(name, |row, ctx| ...)` for computed/override fields
- `DatatableRelationFilter<Row, Query>` and `DatatableRelationColumn<Row>` for opt-in relation filters
- `DatatableValue` enum with constructors and `Into<serde_json::Value>` conversion
- `DatatableSort<Row>` with typed `::asc(field)` / `::desc(field)` constructors

### Request Shape (Blueprint: Request Shape)

- `DatatableRequest` with page, per_page, sort, filters, search
- Helper methods: `.text()`, `.bool()`, `.date()`, `.datetime()`, `.values()`
- `DatatableFilterInput` with field, op, value
- `DatatableFilterOp` enum covering Eq, Like, date ranges, In, Has, etc.
- `DatatableFilterValue` enum (Text, Bool, Number, Values)
- Frontends send the same `DatatableFilterInput` shape for direct and relation filters; relation fields such as `merchant.name`, legacy aliases such as `merchant-name`, and multi-column `LikeAny` fields such as `merchant.name|merchant.slug` resolve only when declared by the server datatable

### Filter Metadata (Blueprint: Filter Field Types)

- `DatatableFilterKind`: Text, Select, Checkbox, Date, DateTime
- `DatatableFilterField` with typed constructors: `::text()`, `::select()`, `::checkbox()`, `::date()`, `::datetime()`
- Builder helpers: `.placeholder()`, `.options()`, `.help()`, `.nullable()`
- `DatatableFilterRow::single()` / `::pair()` for layout
- `DatatableFilterOption::new(value, label)` for select options
- Options accept both `Vec` and `Collection` via `Into<Collection<>>`

### Auto-Filter Engine (Blueprint: Filter System)

- Legacy param normalization via `DatatableRequest::from_query_params()` supporting f-like-, f-date-, f-gte-, etc.
- `apply_auto_filters()` building `Condition` from declared filter expressions + `DbType`
- `DatatableRelationFilter` applies typed `where_has(...)` / `where_has_many_to_many(...)` filters for declared relation fields and legacy hyphen aliases
- Relation filters are server-side declarations only; the framework does not expose arbitrary relation paths or add a separate frontend wire schema
- `apply_sorts()` with column validation against declared sort expressions
- Supports all filter ops: Eq, Like, Gt/Gte/Lt/Lte, Date/DateFrom/DateTo, DateTime ranges, In, Has, HasLike, LikeAny

### Traits (Blueprint: Core Datatable Shape)

- Unified `Datatable` trait with associated `Row` and `Query`, plus `query()`, `columns()`, `mappings()`, `filters()`, `available_filters()`, `relation_filters()`, `default_sort()`
- Provided methods: `json()`, `download()`, `queue_email()` delegating to output modules
- `DatatableQuery<Row>` sealed adapter implemented for `ModelQuery<Row>` and `ProjectionQuery<Row>`
- `DatatableContext` with `app`, `actor`, `request`, `locale`, `timezone` + `t()` helper

### Output Modes (Blueprint: Output Modes)

- **JSON**: `build_json_response()` in `json.rs` — scoped query, auto-filter, custom filter hook, sorting, pagination, row building with column extraction + mapping overrides
- **Download**: `build_download_response()` in `download.rs` — fully implemented with `rust_xlsxwriter` (bold headers, type-aware cells, exportable column filtering, mapping support)
- **Email**: `dispatch_export()` in `export_job.rs` — fully implemented with `DatatableExportJob` (`Job` trait, max 3 retries), reuses download pipeline, pluggable `DatatableExportDelivery`

### Export Contract (Blueprint: Export Contract)

- `DatatableExportDelivery` trait with `deliver()` method
- `GeneratedDatatableExport` payload with datatable_id, filename, data bytes, columns
- `NoopExportDelivery` as default/log implementation
- `DatatableActorSnapshot` for serializing actor state into export jobs

### Registry (Blueprint: Registry and Resolution)

- `DatatableRegistry` with `get(id)` and `ids()` for type-erased lookup
- `DatatableRegistryBuilder` with shared-handle pattern (Arc<Mutex<>>)
- `DynDatatable` trait as type-erased interface
- `DatatableAdapter<D>` bridging `Datatable` to `DynDatatable`
- `ServiceRegistrar::register_datatable::<D>()` for provider registration
- `AppContext::datatables()` for runtime resolution

### AST Gap Filled

- `ComparisonOp::Like` and `ComparisonOp::NotLike` added to database AST
- `Column<M, T>::like()` and `.not_like()` methods for typed LIKE queries
- Compiler support for LIKE/NOT LIKE SQL generation

### Framework Integration

- `pub mod datatable` in `src/lib.rs`
- All primary types re-exported from `src/lib.rs` and `src/prelude.rs`
- Datatable registry frozen during bootstrap and registered as singleton
- Accessible via `app.datatables()?` on `AppContext`

## Remaining Work

- No open items from the datatable blueprint are currently tracked here.

## Verification Coverage

- JSON, model/projection registry resolution, WHERE/HAVING filters, relation filters, sorting, decimal filters, XLSX download, and queued export delivery are covered by `tests/datatable_acceptance.rs`.
- Filter engine edge cases and filter metadata bindings are covered by unit tests under `src/datatable/`.
- Legacy query-param normalization is covered by unit tests in `src/datatable/request.rs`.
