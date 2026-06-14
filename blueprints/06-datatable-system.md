# Rust Datatable System Blueprint (Framework-Level)

## Overview

This document defines the design of a **framework-level datatable system** for Foundry.

Goal:

> Provide a model-first, scoped, filterable, exportable datatable subsystem that feels close to Laravel datatables in day-to-day DX, while staying explicit, typed, and framework-native in Rust.

This is a **design blueprint only**. It does not mean the subsystem is already implemented.

---

# Objective

Build a datatable system that:

- is **model-first** by default
- supports explicit scoping through **`AppContext` + `Option<Actor>`**
- exposes frontend-ready JSON rows and frontend-ready filter metadata
- supports both **legacy/pattern filter input** and a cleaner **structured filter shape**
- provides backend-defined filter controls for:
  - text
  - select
  - checkbox
  - date
  - datetime
- supports row mappings for:
  - overriding output columns
  - adding computed/output-only columns
- supports full filtered export as **real XLSX**
- supports queued "generate and send by email" export through an **abstract delivery contract**
- reuses Foundry model/query/runtime primitives instead of creating a parallel query system

---

# Core Philosophy

1. **Model-first is the default path**
2. **Scoping is explicit, never magical**
3. **`Collection` is for post-query shaping, not primary query authoring**
4. **Columns define DB-backed behavior; mappings define output-only behavior**
5. **Automatic filtering only works on explicitly declared metadata**
6. **Legacy-friendly input is supported, but normalized into a typed internal shape**
7. **Frontend filter rendering should come from backend-declared metadata, not duplicated UI logic**
8. **Download/export should reuse the same scoped and filtered query, not invent another code path**

---

# Module Shape

Introduce a new framework module:

```text
src/datatable/
```

Primary public concepts:

- `ModelDatatable`
- `ProjectionDatatable`
- `DatatableContext`
- `DatatableRequest`
- `DatatableJsonResponse`
- `DatatableFilterInput`
- `DatatableColumn`
- `DatatableMapping`
- `DatatableSort`
- `DatatableSortInput`
- `DatatableFilterRow`
- `DatatableFilterField`
- `DatatableFilterOption`
- `DatatableRegistry`
- `DatatableExportDelivery`

Primary app entrypoints:

```rust
AppContext::datatables() -> Result<Arc<DatatableRegistry>>
registrar.register_datatable::<UsersTable>()?
```

Primary handler DX:

```rust
UsersTable::json(&app, Some(&actor), request).await?;
UsersTable::download(&app, Some(&actor), request).await?;
UsersTable::queue_email(&app, Some(&actor), request, "admin@example.com").await?;
```

---

# Core Datatable Shape

## Main Trait

The main path is a model-backed trait:

```rust
#[async_trait]
pub trait ModelDatatable: Send + Sync + 'static {
    type Model: Model;

    const ID: &'static str;

    fn query(ctx: &DatatableContext) -> ModelQuery<Self::Model>;

    fn columns() -> Vec<DatatableColumn<Self::Model>>;

    fn mappings() -> Vec<DatatableMapping<Self::Model>> {
        Vec::new()
    }

    async fn filters(
        ctx: &DatatableContext,
        query: ModelQuery<Self::Model>,
    ) -> Result<ModelQuery<Self::Model>> {
        Ok(query)
    }

    async fn available_filters(
        ctx: &DatatableContext,
    ) -> Result<Vec<DatatableFilterRow>> {
        Ok(Vec::new())
    }

    fn default_sort() -> Vec<DatatableSort<Self::Model>> {
        Vec::new()
    }
}
```

## Secondary Escape Hatch

Some datatables cannot be represented cleanly as plain model rows, especially:

- grouped reports
- aggregate-heavy summaries
- projection-only admin reports
- derived rows not backed by a single model

For those, add a secondary projection-backed trait, for example:

```rust
#[async_trait]
pub trait ProjectionDatatable: Send + Sync + 'static {
    type Row: Send + Sync + 'static;

    const ID: &'static str;

    fn query(ctx: &DatatableContext) -> ProjectionQuery<Self::Row>;

    fn columns() -> Vec<ProjectionDatatableColumn<Self::Row>>;

    fn mappings() -> Vec<ProjectionDatatableMapping<Self::Row>> {
        Vec::new()
    }
}
```

This is a secondary escape hatch, not the headline path.

---

# Datatable Context

Datatable execution should be explicitly scoped through a context object:

```rust
pub struct DatatableContext<'a> {
    pub app: &'a AppContext,
    pub actor: Option<&'a Actor>,
    pub request: &'a DatatableRequest,
    pub locale: Option<&'a str>,
    pub timezone: Timezone,
}
```

## Rules

- datatables do **not** rely on `app.current_actor()`
- authorization/scope decisions read from `ctx.actor`
- locale-aware filter metadata and export formatting read from `ctx.locale` and `ctx.timezone`
- this keeps datatable behavior deterministic across:
  - HTTP JSON requests
  - direct export endpoints
  - queued export jobs

---

# Request Shape

`DatatableRequest` is the normalized internal request shape.

Both:

- legacy/pattern query params
- structured frontend filter payloads

must normalize into this type.

```rust
pub struct DatatableRequest {
    pub page: u64,
    pub per_page: u64,
    pub sort: Vec<DatatableSortInput>,
    pub filters: Vec<DatatableFilterInput>,
    pub search: Option<String>,
}
```

Suggested helper methods:

- `text(&self, name: &str) -> Option<&str>`
- `bool(&self, name: &str) -> Option<bool>`
- `date(&self, name: &str) -> Option<Date>`
- `datetime(&self, name: &str) -> Option<DateTime>`
- `values(&self, name: &str) -> Collection<String>`

---

# Goal DX

The intended single-file datatable shape should look like:

```rust
pub struct UsersTable;

#[async_trait]
impl ModelDatatable for UsersTable {
    type Model = User;

    const ID: &'static str = "users";

    fn query(ctx: &DatatableContext) -> ModelQuery<User> {
        let admin_id = ctx.actor.map(|actor| actor.id).unwrap();

        User::query()
            .with(User::country())
            .where_(User::ADMIN_ID.eq(admin_id))
    }

    fn columns() -> Vec<DatatableColumn<User>> {
        vec![
            DatatableColumn::field(User::ID).sortable().exportable(),
            DatatableColumn::field(User::USERNAME).sortable().filterable(),
            DatatableColumn::field(User::EMAIL).sortable().filterable(),
            DatatableColumn::field(User::COUNTRY_ID).filterable(),
            DatatableColumn::field(User::CREATED_AT).sortable().filterable(),
        ]
    }

    fn mappings() -> Vec<DatatableMapping<User>> {
        vec![
            DatatableMapping::new("country_name", |row, _ctx| {
                row.country
                    .as_ref()
                    .and_then(|country| country.as_ref())
                    .map(|country| DatatableValue::string(country.name.clone()))
                    .unwrap_or_else(|| DatatableValue::string("-"))
            }),
        ]
    }

    async fn filters(
        ctx: &DatatableContext,
        query: ModelQuery<User>,
    ) -> Result<ModelQuery<User>> {
        if let Some(value) = ctx.request.text("f-like-username") {
            return Ok(query.where_(User::USERNAME.like(format!("%{value}%"))));
        }

        Ok(query)
    }

    async fn available_filters(
        ctx: &DatatableContext,
    ) -> Result<Vec<DatatableFilterRow>> {
        let countries = Country::query()
            .order_by(Country::NAME.asc())
            .get(ctx.app.database()?.as_ref())
            .await?;

        Ok(vec![
            DatatableFilterRow::pair(
                DatatableFilterField::select("f-country_id", ctx.t("Country"))
                    .options(
                        countries.map(|country| {
                            DatatableFilterOption::new(
                                country.id.to_string(),
                                country.name,
                            )
                        }),
                    ),
                DatatableFilterField::text("f-like-username", ctx.t("Username"))
                    .placeholder(ctx.t("All")),
            ),
            DatatableFilterRow::pair(
                DatatableFilterField::checkbox("f-active", ctx.t("Active")),
                DatatableFilterField::date("f-date-created_at", ctx.t("Created Date")),
            ),
            DatatableFilterRow::single(
                DatatableFilterField::datetime(
                    "f-datetime-from-last_login_at",
                    ctx.t("Last Login From"),
                ),
            ),
        ])
    }
}
```

Typical handler DX:

```rust
async fn index(
    State(app): State<AppContext>,
    actor: CurrentActor,
    Query(request): Query<DatatableRequest>,
) -> Result<Json<DatatableJsonResponse>> {
    UsersTable::json(&app, Some(&actor), request).await.map(Json)
}
```

```rust
async fn download(
    State(app): State<AppContext>,
    actor: CurrentActor,
    Query(request): Query<DatatableRequest>,
) -> Result<Response> {
    UsersTable::download(&app, Some(&actor), request).await
}
```

```rust
async fn email_export(
    State(app): State<AppContext>,
    actor: CurrentActor,
    Query(request): Query<DatatableRequest>,
) -> Result<Json<DatatableExportAccepted>> {
    UsersTable::queue_email(&app, Some(&actor), request, "admin@example.com")
        .await
        .map(Json)
}
```

---

# Columns and Mappings

## Columns

`columns()` defines the DB-backed datatable surface.

Columns should declare metadata such as:

- field/column source
- output key
- label
- sortable
- filterable
- exportable
- relation path when relevant
- visibility in JSON and export

Example shape:

```rust
pub struct DatatableColumn<M: Model> {
    pub name: String,
    pub label: String,
    pub sortable: bool,
    pub filterable: bool,
    pub exportable: bool,
    pub relation: Option<String>,
    pub source: DatatableColumnSource<M>,
}
```

## Mappings

`mappings()` exists for output shaping after row hydration.

Mappings can:

- override an existing output field
- add a computed/output-only field
- format model-backed values differently for JSON/export

Mappings should not mutate the underlying query.

Example value contract:

```rust
pub enum DatatableValue {
    Null,
    String(String),
    Number(serde_json::Number),
    Bool(bool),
    Date(Date),
    DateTime(DateTime),
}
```

## Rules

- computed mapping fields are **not automatically sortable**
- computed mapping fields are **not automatically filterable**
- only DB-backed declared columns participate in automatic filter/sort behavior
- custom filters and explicit sort hooks may still target derived behavior if needed

---

# Filter Field Types and Frontend Metadata

## Filter Kind

The datatable module should define explicit frontend filter kinds:

```rust
pub enum DatatableFilterKind {
    Text,
    Select,
    Checkbox,
    Date,
    DateTime,
}
```

## Field Shape

```rust
pub struct DatatableFilterField {
    pub name: String,
    pub kind: DatatableFilterKind,
    pub label: String,
    pub placeholder: Option<String>,
    pub help: Option<String>,
    pub nullable: bool,
    pub options: Collection<DatatableFilterOption>,
}
```

Constructors:

```rust
DatatableFilterField::text(name, label)
DatatableFilterField::select(name, label)
DatatableFilterField::checkbox(name, label)
DatatableFilterField::date(name, label)
DatatableFilterField::datetime(name, label)
```

Builder helpers:

- `.placeholder(...)`
- `.options(...)`
- `.help(...)`
- `.nullable(...)`

`label` is the second constructor argument.

## Select Options

Select options should be simple:

```rust
pub struct DatatableFilterOption {
    pub value: String,
    pub label: String,
}
```

They should support both Foundry `Collection` and `Vec` naturally:

```rust
pub fn options<I>(self, options: I) -> Self
where
    I: Into<Collection<DatatableFilterOption>>;
```

So both should be valid:

```rust
field.options(vec![
    DatatableFilterOption::new("1", "Malaysia"),
    DatatableFilterOption::new("2", "Singapore"),
])
```

```rust
field.options(Collection::from_vec(vec![
    DatatableFilterOption::new("1", "Malaysia"),
    DatatableFilterOption::new("2", "Singapore"),
]))
```

## Filter Layout Rows

Frontend layout should be explicit:

```rust
pub struct DatatableFilterRow {
    pub fields: Vec<DatatableFilterField>,
}
```

Recommended helpers:

- `DatatableFilterRow::single(field)`
- `DatatableFilterRow::pair(left, right)`

Rules:

- one field in the row means full-width
- two fields in the row means split layout
- do not rely on PHP-style array-vs-object inference

## Localization Rule

Labels, placeholders, help text, and select option labels should be **backend-resolved localized strings**.

That means:

- `available_filters(ctx)` uses the current `DatatableContext`
- the backend returns frontend-ready text
- the frontend renders that text directly
- the frontend does not translate datatable keys by itself

Suggested convenience:

```rust
ctx.t("Country")
ctx.t("All")
ctx.t("Created Date")
```

---

# Filter System

The datatable system should support **dual-mode filtering**:

1. legacy/pattern query parameters for fast DX
2. structured filter input for long-term frontend cleanliness

## Legacy/Pattern Input

Supported legacy-style input syntax should include:

- `f-like-email`
- `f-country_id`
- `f-active`
- `f-date-created_at`
- `f-date-from-created_at`
- `f-date-to-created_at`
- `f-datetime-last_login_at`
- `f-datetime-from-last_login_at`
- `f-datetime-to-last_login_at`
- `f-gte-score`
- `f-lte-score`
- `f-like-any-name|email`
- `f-has-country-id`
- `f-has-like-country-name`

These are supported as input syntax, not as the core internal contract.

## Structured Input

Preferred long-term frontend payload shape should normalize to something like:

```json
{
  "page": 1,
  "per_page": 20,
  "sort": [
    { "field": "created_at", "direction": "desc" }
  ],
  "filters": [
    { "field": "email", "op": "like", "value": "foundry" },
    { "field": "country.name", "op": "like", "value": "malaysia" },
    { "field": "active", "op": "eq", "value": true },
    { "field": "created_at", "op": "date", "value": "2026-04-11" }
  ]
}
```

## Internal Rule

All filter inputs must normalize into `DatatableFilterInput`.

Example:

```rust
pub struct DatatableFilterInput {
    pub field: String,
    pub op: DatatableFilterOp,
    pub value: DatatableFilterValue,
}
```

## Rules

- auto filters only work on explicitly declared filterable columns/relations
- relation filters use datatable metadata, not blind string guessing
- checkbox filters normalize into boolean values
- date filters normalize into Foundry `Date`
- datetime filters normalize into Foundry `DateTime`
- locale-aware filters are opt-in datatable metadata, not framework-wide magic
- custom `filters()` logic can further refine or replace automatic behavior

---

# Output Modes

The datatable subsystem should support three output modes:

- `json`
- `download`
- `email`

## JSON

JSON mode is for frontend rendering.

Suggested response shape:

```rust
pub struct DatatableJsonResponse {
    pub rows: Collection<serde_json::Map<String, serde_json::Value>>,
    pub columns: Vec<DatatableColumnMeta>,
    pub filters: Vec<DatatableFilterRow>,
    pub pagination: DatatablePaginationMeta,
    pub applied_filters: Vec<DatatableFilterInput>,
    pub sorts: Vec<DatatableSortInput>,
}
```

Rules:

- respects pagination
- respects datatable scope
- respects normalized filters and sorts
- returns frontend-ready filter metadata

## Download

Download mode is for direct export response.

Rules:

- ignores pagination
- reruns the full scoped + filtered query
- generates **real XLSX**, not CSV
- uses query streaming / chunked processing
- should not materialize the entire result set in memory
- works for both model-backed and projection-backed datatables

## Email

Email mode is for asynchronous export delivery.

Rules:

- queues async XLSX generation
- captures a normalized export payload
- uses an abstract delivery contract

Suggested export payload:

```rust
pub struct DatatableExportJobPayload {
    pub datatable_id: String,
    pub request: DatatableRequest,
    pub actor: Option<DatatableActorSnapshot>,
    pub locale: Option<String>,
    pub timezone: Timezone,
    pub recipient: String,
}
```

The datatable blueprint should define the public DX:

```rust
UsersTable::queue_email(&app, Some(&actor), request, "admin@example.com").await?;
```

But it should also explicitly state:

- actual email delivery depends on a future mail/notification subsystem
- the datatable module defines the contract now, not the full mail implementation

---

# Registry and Resolution

Queued exports and generic datatable dispatch need registry-based lookup.

Add:

```rust
registrar.register_datatable::<UsersTable>()?
```

```rust
AppContext::datatables() -> Result<Arc<DatatableRegistry>>
```

Primary reasons for the registry:

- queued export rehydration by datatable ID
- generic admin endpoints
- centralized datatable registration

---

# HTTP Integration

Datatables should integrate naturally with Foundry HTTP extractors.

Target DX:

```rust
async fn index(
    State(app): State<AppContext>,
    actor: CurrentActor,
    Query(request): Query<DatatableRequest>,
) -> Result<Json<DatatableJsonResponse>> {
    UsersTable::json(&app, Some(&actor), request).await.map(Json)
}
```

Rules:

- `DatatableRequest` should work as a standard extractor target
- legacy query params and structured filter input should be normalized at the boundary
- HTTP integration should not require manual request parsing inside every table

---

# Collection Usage

Foundry `Collection` should be used where it improves datatable ergonomics:

- option lists for filter controls
- JSON row collections
- mapped output rows
- export row shaping

But `Collection` should **not** replace the underlying query source for the main path.

That means:

- query authoring remains `ModelQuery<M>` or projection query
- result shaping and frontend metadata can use `Collection`

This keeps the query layer and datatable layer aligned with current Foundry design.

---

# Export Contract

Because real async export delivery depends on systems beyond the datatable module, define a contract:

```rust
#[async_trait]
pub trait DatatableExportDelivery: Send + Sync + 'static {
    async fn deliver(
        &self,
        export: GeneratedDatatableExport,
        recipient: &str,
    ) -> Result<()>;
}
```

This contract should be framework-level, but delivery implementation can come later through:

- future mail subsystem
- notification subsystem
- custom provider registration

---

# Test Plan

Implementation should include coverage for:

- model-backed datatable query scoping with `AppContext` + `Actor`
- projection-backed escape hatch for grouped/aggregate tables
- mappings that add new fields
- mappings that override existing output fields
- custom filter hook behavior
- normalization from legacy pattern filters
- normalization from structured filter input
- checkbox filter normalization
- date filter normalization
- datetime filter normalization
- relation filters and relation-like-any filters
- localized available filter metadata for frontend rendering
- `select` filter options working from both `Vec` and `Collection`
- JSON mode pagination + sorting
- download mode full-query XLSX streaming without page limit
- async export payload containing actor snapshot and normalized filters
- email/export handoff using abstract delivery contract
- computed fields not being auto-sortable/filterable unless explicitly declared
- clear failures for unknown filter fields
- clear failures for unknown sort fields
- clear failures for unsupported operations

---

# Assumptions and Defaults

- root file name: `rust_datatable_system_blueprint_framework_level.md`
- this is a **blueprint only**, not an implementation/status update
- main path is `ModelDatatable`
- projection-backed support is a secondary escape hatch
- datatable scoping uses `AppContext` + `Option<Actor>` explicitly
- `Collection` is used after query execution for transform/output work, not as the main query source
- filter input is dual-mode:
  - improved legacy/pattern support
  - structured normalized filter shape
- output modes are:
  - `json`
  - `download`
  - `email`
- export format is **real XLSX**
- emailed export is part of intended DX, but delivery depends on an abstract contract because no mail subsystem exists yet
- filter field constructors are explicit by type:
  - text
  - select
  - checkbox
  - date
  - datetime
- select options are accepted from both `Vec` and Foundry `Collection`
- labels, placeholders, help text, and select option labels are backend-localized in `available_filters(ctx)`

---

# One-Line Goal

> A Foundry datatable should let one file define query scope, output shaping, filter metadata, and export behavior in a way that is frontend-friendly, role-aware, and still grounded in the existing model/query system.
