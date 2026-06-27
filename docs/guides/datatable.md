# Datatable

Foundry datatables now use a single `Datatable` trait for both model-backed tables and projection/report rows.

## Registering a Datatable

```rust
use foundry::prelude::*;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_datatable::<OrdersDatatable>()?;
        registrar.register_datatable::<MerchantSalesDatatable>()?;
        Ok(())
    }
}
```

The same registration path works for app providers and plugins.

Registered datatables are also included in `types:export` output. The generated
`DatatableManifest.ts` contains backend-owned table ids, columns, computed
mappings, relation-filter aliases, and default sorts so frontend table builders
can import `DatatableManifest`, `DatatableRuntimeManifest`, `DatatableIds`,
`DatatableMaxPerPage`, `DatatableMaxExportRows`, `datatablePerPageCap()`,
`datatableExportRowsCap()`, `datatableManifestEntry()`, `datatableEntries()`,
`datatableNames()`, `datatableColumns()`, `datatableColumn()`,
`datatableColumnNames()`, `datatableSortableColumns()`,
`datatableSortableColumn()`, `datatableSortableFieldNames()`,
`datatableFilterableColumns()`, `datatableFilterableColumn()`,
`datatableFilterableColumnNames()`, `datatableMappings()`,
`datatableMappingNames()`, `datatableRelationFilters()`,
`datatableRelationFilterForField()`,
`datatableRelationFilterCanonicalField()`, `datatableRelationFilterFields()`,
`datatableRelationFilterAliases()`, `datatableStaticFilterFieldNames()`,
`datatableDefaultSortFieldNames()`, `datatableSort()`, `datatableFilter()`,
`datatableRequest()`, `datatableQueryParams()`,
`datatableRequestFromQueryParams()`, `isDatatableName()`, and
`datatableNameOrNull()` instead of maintaining
a parallel table registry or copied pagination/export caps. The generated `DatatableRequestFor<Name>`
type narrows sort fields and static filter fields from the manifest, and
`datatableRequest()` validates both before query params are generated or parsed.
Nested safe parsers such as `datatableColumnNameOrNull()`,
`datatableColumnNameForRelationOrNull()`,
`datatableRelationFilterFieldNameOrNull()`,
`datatableRelationFilterFieldNameForRelationOrNull()`,
`datatableStaticFilterFieldNameOrNull()`, `datatableMappingNameOrNull()`, and
`datatableDefaultSortFieldNameOrNull()` normalize runtime field, relation,
mapping, filter, and default-sort strings into generated datatable unions.
`datatableRequestFromQueryParams()` hydrates a typed request from `URLSearchParams`,
raw `?sort=...&f-...` strings, or plain query objects for shareable table URLs.
Dynamic
`available_filters()` controls can still use the raw generated
`DatatableRequest` shape when their fields are not part of the static manifest,
or call `datatableRequestFromQueryParams(name, query, { validate: false })`.
Directly constructed `DatatableDescriptor` values must use non-empty, trimmed
field names, unique column/mapping/default-sort names, and non-colliding static
filter fields for filterable columns and relation filter aliases.
Generated datatable manifests, runtime caps, and id trees are frozen at runtime,
so direct mutation cannot change backend-owned table metadata. Datatable
selector helpers clone entries, columns, mappings, relation-filter aliases, and
default sorts before returning them, so table builders can add local column
state, selection state, or UI-only labels to selector results. Lookup helpers
retrieve a single backend-owned column and normalize relation-filter aliases to
their canonical field names without local `.find(...)` scans.

Queued export responses are also backend-owned: `DatatableExportAccepted` and
the AppEnum-backed `DatatableExportStatus` are generated for frontend export
actions instead of requiring a local `"queued"` union.

OpenAPI schemas use the same backend-owned contracts: `DatatableJsonResponse`
documents `columns`, `pagination`, `filters`, `applied_filters`, and `sorts`
with `DatatableColumnMeta`, `DatatablePaginationMeta`, `DatatableFilterRow`,
`DatatableFilterInput`, and `DatatableSortInput` instead of opaque or duplicated
object fragments.

## The `Datatable` Trait

```rust
#[async_trait]
trait Datatable: Send + Sync + 'static {
    type Row: Serialize + Send + Sync + 'static;
    type Query: DatatableQuery<Self::Row>;

    const ID: &'static str;

    fn query(ctx: &DatatableContext) -> Self::Query;
    fn columns() -> Vec<DatatableColumn<Self::Row>>;

    fn mappings() -> Vec<DatatableMapping<Self::Row>> { vec![] }
    async fn filters(ctx: &DatatableContext, query: Self::Query) -> Result<Self::Query> { Ok(query) }
    async fn available_filters(ctx: &DatatableContext) -> Result<Vec<DatatableFilterRow>> { Ok(vec![]) }
    fn relation_filters() -> Vec<DatatableRelationFilter<Self::Row, Self::Query>> { vec![] }
    fn default_sort() -> Vec<DatatableSort<Self::Row>> { vec![] }

    async fn json(app, actor, request) -> Result<DatatableJsonResponse>;
    async fn download(app, actor, request) -> Result<Response>;
    async fn queue_email(app, actor, request, recipient) -> Result<DatatableExportAccepted>;
}
```

`type Query` is usually one of:

- `ModelQuery<MyModel>`
- `ProjectionQuery<MyProjection>`

## Column DX

`DatatableColumn::field(...)` accepts either:

- a model `Column<M, T>`
- a projection `ProjectionField<P, T>`

Common builders:

```rust
DatatableColumn::field(Order::ID).label("Order").sortable().exportable();
DatatableColumn::field(Order::TOTAL).filterable();
DatatableColumn::field(SalesRow::TOTAL).sortable();
DatatableColumn::field(SalesRow::TOTAL).filter_having(Expr::function("SUM", [Expr::column(Order::TOTAL.column_ref())]));
DatatableColumn::field(SalesRow::MERCHANT_ID).filter_by(SalesRow::MERCHANT_ID.column_ref_from("orders"));
```

Rules:

- model fields get implicit sort/filter targets
- projection fields get implicit sort-by-alias support
- projection auto-filtering is explicit: use `filter_by(...)` for `WHERE` and `filter_having(...)` for `HAVING`
- relation auto-filtering is explicit: use `relation_filters()` for typed `where_has(...)` filters
- JSON responses include each column's `sortable`, `filterable`, `exportable`, and nullable `relation` metadata so frontend tables can render controls from backend-owned declarations

## Model Datatable Example

```rust
use foundry::prelude::*;
use serde::Serialize;

#[derive(Debug, Serialize, Model)]
#[foundry(table = "orders", primary_key_strategy = "manual")]
struct Order {
    id: i64,
    merchant_id: i64,
    total: i64,
}

struct OrdersDatatable;

#[async_trait]
impl Datatable for OrdersDatatable {
    type Row = Order;
    type Query = ModelQuery<Order>;

    const ID: &'static str = "orders";

    fn query(_ctx: &DatatableContext) -> Self::Query {
        Order::query()
    }

    fn columns() -> Vec<DatatableColumn<Self::Row>> {
        vec![
            DatatableColumn::field(Order::ID).label("Order").sortable().exportable(),
            DatatableColumn::field(Order::MERCHANT_ID)
                .label("Merchant")
                .filterable()
                .exportable(),
            DatatableColumn::field(Order::TOTAL)
                .label("Total")
                .sortable()
                .filterable()
                .exportable(),
        ]
    }

    fn default_sort() -> Vec<DatatableSort<Self::Row>> {
        vec![DatatableSort::desc(Order::ID)]
    }
}
```

## Relation Filters

Model datatables can opt in to relation-backed filters without accepting arbitrary string paths. Declare the relation and target column in `relation_filters()`:

```rust
fn order_merchant() -> RelationDef<Order, Merchant> {
    belongs_to(
        Order::MERCHANT_ID,
        Merchant::ID,
        |order| Some(order.merchant_id),
        |_order, _merchant| {},
    )
}

fn order_tags() -> ManyToManyDef<Order, Tag> {
    many_to_many(
        Order::ID,
        "order_tags",
        "order_id",
        "tag_id",
        Tag::ID,
        |order| order.id,
        |_order, _tags| {},
    )
}

fn relation_filters() -> Vec<DatatableRelationFilter<Self::Row, Self::Query>> {
    vec![
        DatatableRelationFilter::relation(
            "merchant.name",
            order_merchant(),
            Merchant::NAME,
        ),
        DatatableRelationFilter::many_to_many(
            "tags.name",
            order_tags(),
            Tag::NAME,
        ),
        DatatableRelationFilter::relation_columns(
            "merchant.name|merchant.slug",
            order_merchant(),
            [
                DatatableRelationColumn::field(Merchant::NAME),
                DatatableRelationColumn::field(Merchant::SLUG),
            ],
        ),
    ]
}
```

Supported relation filter input:

- structured fields such as `merchant.name`
- legacy hyphen aliases such as `merchant-name`
- `LikeAny` over declared relation columns, such as `merchant.name|merchant.slug`
- normal filter ops like `Eq`, `Like`, `Has`, `HasLike`, date, datetime, and numeric comparisons when the target column type supports them

Client requests still use the normal `DatatableRequest` shape; relation filters are server-side declarations, not frontend metadata:

```json
{
  "page": 1,
  "per_page": 25,
  "filters": [
    { "field": "total", "op": "gte", "value": { "number": 5000 } },
    { "field": "merchant.name", "op": "like", "value": { "text": "foundry" } },
    { "field": "tags.name", "op": "eq", "value": { "text": "priority" } },
    { "field": "merchant.name|merchant.slug", "op": "like_any", "value": { "text": "foundry" } }
  ]
}
```

Legacy query strings normalize to the same sort and filter inputs when routed
through `Query<DatatableRequest>` or `DatatableRequest::from_query_params()`:

```text
?sort=total,-created_at&direction=desc,asc&f-gte-total=5000&f-like-merchant-name=foundry&f-eq-tags-name=priority
```

Exact filters should use `f-eq-<field>` in generated or custom query builders.
The older `f-<field>` fallback is still accepted, but it is ambiguous when a
field name starts with an operator prefix such as `in-`, `date-`, or `not-eq-`.

Projection datatables should keep relation-like behavior explicit with `filter_by(...)`, `filter_having(...)`, or a custom `filters()` hook.

## Projection / Report Example

```rust
use foundry::prelude::*;
use serde::Serialize;

#[derive(Debug, Serialize, Model)]
#[foundry(table = "orders", primary_key_strategy = "manual")]
struct Order {
    id: i64,
    merchant_id: i64,
    total: i64,
}

#[derive(Clone, Debug, Serialize, Projection)]
struct MerchantSalesRow {
    merchant_id: i64,
    order_count: i64,
    total: Option<i64>,
}

struct MerchantSalesDatatable;

#[async_trait]
impl Datatable for MerchantSalesDatatable {
    type Row = MerchantSalesRow;
    type Query = ProjectionQuery<MerchantSalesRow>;

    const ID: &'static str = "merchant-sales";

    fn query(_ctx: &DatatableContext) -> Self::Query {
        MerchantSalesRow::source("orders")
            .select_source(MerchantSalesRow::MERCHANT_ID, "orders")
            .select_aggregate(AggregateProjection::<i64>::count_all(
                MerchantSalesRow::ORDER_COUNT.alias(),
            ))
            .select_aggregate(AggregateProjection::<Option<i64>>::sum(
                Order::TOTAL.column_ref(),
                MerchantSalesRow::TOTAL.alias(),
            ))
            .group_by(MerchantSalesRow::MERCHANT_ID.column_ref_from("orders"))
    }

    fn columns() -> Vec<DatatableColumn<Self::Row>> {
        vec![
            DatatableColumn::field(MerchantSalesRow::MERCHANT_ID)
                .label("Merchant")
                .sortable()
                .filter_by(MerchantSalesRow::MERCHANT_ID.column_ref_from("orders"))
                .exportable(),
            DatatableColumn::field(MerchantSalesRow::ORDER_COUNT)
                .label("Orders")
                .sortable()
                .exportable(),
            DatatableColumn::field(MerchantSalesRow::TOTAL)
                .label("Revenue")
                .sortable()
                .filter_having(Expr::function(
                    "SUM",
                    [Expr::column(Order::TOTAL.column_ref())],
                ))
                .exportable(),
        ]
    }

    fn default_sort() -> Vec<DatatableSort<Self::Row>> {
        vec![DatatableSort::desc(MerchantSalesRow::TOTAL)]
    }
}
```

## Generic Runtime Registry

Every registered datatable is available through the app registry:

```rust
async fn datatable_json(
    State(app): State<AppContext>,
    Path(datatable_id): Path<String>,
    CurrentActor(actor): CurrentActor,
    Query(request): Query<DatatableRequest>,
) -> Result<Json<DatatableJsonResponse>> {
    let registry = app.datatables()?;
    let datatable = registry
        .get(&datatable_id)
        .ok_or_else(|| Error::not_found(format!("datatable '{datatable_id}' not found")))?;

    Ok(Json(
        datatable.json(&app, Some(&actor), request).await?,
    ))
}
```

The same registry-backed object also supports:

- `datatable.download(&app, actor, request).await?`
- `datatable.queue_email(&app, actor, request, "ops@example.com").await?`

## Filter Metadata

Use `available_filters()` when the frontend needs declarative filter controls:

```rust
async fn available_filters(_ctx: &DatatableContext) -> Result<Vec<DatatableFilterRow>> {
    Ok(vec![
        DatatableFilterRow::pair(
            DatatableFilterField::text_search_fields(
                "merchant_query",
                "Merchant",
                [Order::MERCHANT_ID],
            ),
            DatatableFilterField::decimal_min("minimum_total", "Minimum Total")
                .bind("total", DatatableFilterOp::Gte, DatatableFilterValueKind::Integer),
        ),
    ])
}
```

Each filter field now declares:

- `name`: frontend control id
- `binding.field`: server-side filter field
- `binding.op`: backend-declared operator
- `binding.value_kind`: how the frontend should serialize the value

Foundry also ships semantic helpers for common cases:

```rust
DatatableFilterField::text_like("email", "Email");
DatatableFilterField::text_search_fields("query", "Search", [User::EMAIL, User::NAME]);
DatatableFilterField::text_search("query", "Search").server_field(User::EMAIL);
DatatableFilterField::date_from("created_after", "Created After").bind(
    "created_at",
    DatatableFilterOp::DateFrom,
    DatatableFilterValueKind::Date,
);
DatatableFilterField::date_to("created_before", "Created Before").bind(
    "created_at",
    DatatableFilterOp::DateTo,
    DatatableFilterValueKind::Date,
);
DatatableFilterField::decimal_min("minimum_amount", "Minimum Amount").bind(
    "amount",
    DatatableFilterOp::Gte,
    DatatableFilterValueKind::Decimal,
);
DatatableFilterField::decimal_max("maximum_amount", "Maximum Amount").bind(
    "amount",
    DatatableFilterOp::Lte,
    DatatableFilterValueKind::Decimal,
);
DatatableFilterField::select("status", "Status").options(OrderStatus::options());
DatatableFilterField::select("status", "Status").enum_options::<OrderStatus>();
DatatableFilterField::enum_select::<OrderStatus>("status", "Status");
```

`DatatableFilterField::text(...)` still represents an exact match. Use `text_search(...)` for one-field search and `text_search_fields(...)` for multi-field search.

For AppEnum-backed filters, the datatable option payload keeps the enum label metadata unchanged. With the default AppEnum conventions that means `DatatableFilterOption.label` carries the translation key such as `enum.order_status.pending`.

Foundry accepts structured `DatatableRequest` JSON and legacy `sort` /
`direction` / `f-...` query params directly through `Query<DatatableRequest>`,
but generated manifest helpers such as `datatableRequest()` and
`datatableQueryParams()` are now the preferred frontend contract. Use
`datatableRequestFromQueryParams(name, window.location.search)` to hydrate table
state from a shareable URL, and `datatableQueryParams(name, request)` to write it
back to query params.

## Output Modes

Each datatable gets three output modes with identical scoping/filtering:

- `Datatable::json(...)`
- `Datatable::download(...)`
- `Datatable::queue_email(...)`

Foundry caps expensive datatable outputs by default:

```toml
[datatable]
max_per_page = 500       # 0 = no JSON page-size cap
max_export_rows = 50000  # 0 = no XLSX export row cap
```

`max_per_page` clamps client-provided `DatatableRequest.per_page`. `max_export_rows`
is applied before XLSX downloads and queued export jobs load rows into memory; if
the filtered result is larger, Foundry returns a clear error and the operator can
narrow filters or raise the cap.
After `types:export`, frontend table builders can import
`DatatableRuntimeManifest`, `datatablePerPageCap()`, and
`datatableExportRowsCap()` instead of copying these limits. Runtime manifest
export requires `max_per_page` and `max_export_rows` to stay within JavaScript's
safe integer range; `0` remains the documented unlimited sentinel for each cap.

Queued email exports return `DatatableExportAccepted` with a
`DatatableExportStatus` status field, serialized as `"queued"` for JSON clients.

That keeps model tables and grouped report tables on the same framework path end-to-end.
