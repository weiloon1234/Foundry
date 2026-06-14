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

Legacy query strings normalize to the same filter inputs when routed through `DatatableRequest::from_query_params()`:

```text
?f-gte-total=5000&f-like-merchant-name=foundry&f-tags-name=priority
```

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

Foundry still accepts structured `DatatableRequest` filters and legacy `f-...` query params through `DatatableRequest::from_query_params()`, but explicit binding metadata is now the preferred frontend contract.

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

That keeps model tables and grouped report tables on the same framework path end-to-end.
