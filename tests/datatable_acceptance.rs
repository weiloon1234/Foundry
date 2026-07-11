use std::fs;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use foundry::prelude::*;
use foundry::{DatatableFilterOp, DatatableFilterValue, WorkerKernel};
use serde::Serialize;
use tempfile::TempDir;
use tokio::sync::Mutex as AsyncMutex;

const ORDERS_TABLE: &str = "foundry_datatable_orders";
const PAYMENTS_TABLE: &str = "foundry_datatable_payments";
const MERCHANTS_TABLE: &str = "foundry_datatable_merchants";
const ORDER_ITEMS_TABLE: &str = "foundry_datatable_order_items";
const TAGS_TABLE: &str = "foundry_datatable_tags";
const ORDER_TAGS_TABLE: &str = "foundry_datatable_order_tags";

fn database_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

struct DatatableRuntime {
    _dir: TempDir,
    kernel: WorkerKernel,
    deliveries: Arc<Mutex<Vec<CapturedExport>>>,
}

#[derive(Clone, Debug)]
struct CapturedExport {
    datatable_id: String,
    recipient: String,
    filename: String,
    data: Vec<u8>,
    file_backed: bool,
}

#[derive(Clone)]
struct CaptureDelivery {
    deliveries: Arc<Mutex<Vec<CapturedExport>>>,
}

#[async_trait]
impl DatatableExportDelivery for CaptureDelivery {
    async fn deliver_file(
        &self,
        export: GeneratedDatatableExportFile,
        recipient: &str,
    ) -> Result<()> {
        let data = export.read_bounded(1024 * 1024).await?;
        self.deliveries.lock().unwrap().push(CapturedExport {
            datatable_id: export.datatable_id().to_string(),
            recipient: recipient.to_string(),
            filename: export.filename().to_string(),
            data,
            file_backed: true,
        });
        Ok(())
    }
}

#[derive(Clone)]
struct DatatableProvider {
    deliveries: Arc<Mutex<Vec<CapturedExport>>>,
}

#[async_trait]
impl ServiceProvider for DatatableProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_datatable::<OrdersDatatable>()?;
        registrar.register_datatable::<MerchantSalesDatatable>()?;
        registrar.register_datatable::<PaymentsDatatable>()?;
        registrar.singleton(Box::new(CaptureDelivery {
            deliveries: self.deliveries.clone(),
        }) as Box<dyn DatatableExportDelivery>)?;
        Ok(())
    }
}

async fn datatable_runtime() -> Option<DatatableRuntime> {
    let url = postgres_url()?;
    let dir = tempfile::tempdir().ok()?;
    fs::write(
        dir.path().join("00-runtime.toml"),
        format!(
            r#"
            [database]
            url = "{url}"
            "#
        ),
    )
    .ok()?;

    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let kernel = App::builder()
        .load_config_dir(dir.path())
        .register_provider(DatatableProvider {
            deliveries: deliveries.clone(),
        })
        .build_worker_kernel()
        .await
        .ok()?;

    Some(DatatableRuntime {
        _dir: dir,
        kernel,
        deliveries,
    })
}

async fn execute_batch(database: &DatabaseManager, statements: &[&str]) {
    for statement in statements {
        database.raw_execute(statement, &[]).await.unwrap();
    }
}

async fn reset_schema(database: &DatabaseManager) {
    execute_batch(
        database,
        &[
            &format!("DROP TABLE IF EXISTS {ORDER_TAGS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {ORDER_ITEMS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {ORDERS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {PAYMENTS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {MERCHANTS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {TAGS_TABLE}"),
            &format!(
                "CREATE TABLE {ORDERS_TABLE} (id BIGINT PRIMARY KEY, merchant_id BIGINT NOT NULL, total BIGINT NOT NULL)"
            ),
            &format!("CREATE TABLE {PAYMENTS_TABLE} (id BIGINT PRIMARY KEY, amount NUMERIC NOT NULL)"),
            &format!(
                "CREATE TABLE {MERCHANTS_TABLE} (id BIGINT PRIMARY KEY, name TEXT NOT NULL, slug TEXT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {ORDER_ITEMS_TABLE} (id BIGINT PRIMARY KEY, order_id BIGINT NOT NULL, sku TEXT NOT NULL)"
            ),
            &format!("CREATE TABLE {TAGS_TABLE} (id BIGINT PRIMARY KEY, name TEXT NOT NULL)"),
            &format!("CREATE TABLE {ORDER_TAGS_TABLE} (order_id BIGINT NOT NULL, tag_id BIGINT NOT NULL)"),
        ],
    )
    .await;
}

async fn seed_orders(database: &DatabaseManager) {
    execute_batch(
        database,
        &[
            &format!("INSERT INTO {ORDERS_TABLE} (id, merchant_id, total) VALUES (1, 1, 100)"),
            &format!("INSERT INTO {ORDERS_TABLE} (id, merchant_id, total) VALUES (2, 1, 150)"),
            &format!("INSERT INTO {ORDERS_TABLE} (id, merchant_id, total) VALUES (3, 2, 120)"),
            &format!("INSERT INTO {ORDERS_TABLE} (id, merchant_id, total) VALUES (4, 3, 300)"),
        ],
    )
    .await;
}

async fn seed_order_relations(database: &DatabaseManager) {
    seed_orders(database).await;
    execute_batch(
        database,
        &[
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, name, slug) VALUES (1, 'Acme Retail', 'acme-retail')"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, name, slug) VALUES (2, 'Beta Goods', 'beta-goods')"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, name, slug) VALUES (3, 'Gamma Wholesale', 'gamma-wholesale')"
            ),
            &format!(
                "INSERT INTO {ORDER_ITEMS_TABLE} (id, order_id, sku) VALUES (1, 1, 'vip-box')"
            ),
            &format!(
                "INSERT INTO {ORDER_ITEMS_TABLE} (id, order_id, sku) VALUES (2, 2, 'standard-box')"
            ),
            &format!(
                "INSERT INTO {ORDER_ITEMS_TABLE} (id, order_id, sku) VALUES (3, 4, 'vip-crate')"
            ),
            &format!("INSERT INTO {TAGS_TABLE} (id, name) VALUES (10, 'urgent')"),
            &format!("INSERT INTO {TAGS_TABLE} (id, name) VALUES (11, 'wholesale')"),
            &format!("INSERT INTO {ORDER_TAGS_TABLE} (order_id, tag_id) VALUES (1, 10)"),
            &format!("INSERT INTO {ORDER_TAGS_TABLE} (order_id, tag_id) VALUES (2, 10)"),
            &format!("INSERT INTO {ORDER_TAGS_TABLE} (order_id, tag_id) VALUES (4, 11)"),
        ],
    )
    .await;
}

async fn seed_payments(database: &DatabaseManager) {
    execute_batch(
        database,
        &[
            &format!("INSERT INTO {PAYMENTS_TABLE} (id, amount) VALUES (1, 10.25)"),
            &format!("INSERT INTO {PAYMENTS_TABLE} (id, amount) VALUES (2, 12.50)"),
            &format!("INSERT INTO {PAYMENTS_TABLE} (id, amount) VALUES (3, 19.99)"),
        ],
    )
    .await;
}

#[derive(Debug, PartialEq, Serialize, foundry::Model)]
#[foundry(table = ORDERS_TABLE, primary_key_strategy = "manual")]
struct Order {
    id: i64,
    merchant_id: i64,
    total: i64,
}

#[derive(Debug, PartialEq, Serialize, foundry::Model)]
#[foundry(table = MERCHANTS_TABLE, primary_key_strategy = "manual")]
struct Merchant {
    id: i64,
    name: String,
    slug: String,
}

#[derive(Debug, PartialEq, Serialize, foundry::Model)]
#[foundry(table = ORDER_ITEMS_TABLE, primary_key_strategy = "manual")]
struct OrderItem {
    id: i64,
    order_id: i64,
    sku: String,
}

#[derive(Debug, PartialEq, Serialize, foundry::Model)]
#[foundry(table = TAGS_TABLE, primary_key_strategy = "manual")]
struct Tag {
    id: i64,
    name: String,
}

fn order_merchant_relation() -> RelationDef<Order, Merchant> {
    belongs_to(
        Order::MERCHANT_ID,
        Merchant::ID,
        |order| Some(order.merchant_id),
        |_order, _merchant| {},
    )
}

fn order_items_relation() -> RelationDef<Order, OrderItem> {
    has_many(
        Order::ID,
        OrderItem::ORDER_ID,
        |order| order.id,
        |_order, _items| {},
    )
}

fn order_tags_relation() -> ManyToManyDef<Order, Tag> {
    many_to_many(
        Order::ID,
        ORDER_TAGS_TABLE,
        "order_id",
        "tag_id",
        Tag::ID,
        |order| order.id,
        |_order, _tags| {},
    )
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
            DatatableColumn::field(Order::ID)
                .label("Order")
                .sortable()
                .exportable(),
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
        vec![DatatableSort::asc(Order::ID)]
    }

    fn relation_filters() -> Vec<DatatableRelationFilter<Self::Row, Self::Query>> {
        vec![
            DatatableRelationFilter::relation(
                "merchant.name",
                order_merchant_relation(),
                Merchant::NAME,
            ),
            DatatableRelationFilter::relation("items.sku", order_items_relation(), OrderItem::SKU),
            DatatableRelationFilter::many_to_many("tags.name", order_tags_relation(), Tag::NAME),
            DatatableRelationFilter::relation_columns(
                "merchant.name|merchant.slug",
                order_merchant_relation(),
                [
                    DatatableRelationColumn::field(Merchant::NAME),
                    DatatableRelationColumn::field(Merchant::SLUG),
                ],
            ),
        ]
    }
}

#[derive(Debug, PartialEq, Serialize, foundry::Model)]
#[foundry(table = PAYMENTS_TABLE, primary_key_strategy = "manual")]
struct Payment {
    id: i64,
    amount: Numeric,
}

struct PaymentsDatatable;

#[async_trait]
impl Datatable for PaymentsDatatable {
    type Row = Payment;
    type Query = ModelQuery<Payment>;

    const ID: &'static str = "payments";

    fn query(_ctx: &DatatableContext) -> Self::Query {
        Payment::query()
    }

    fn columns() -> Vec<DatatableColumn<Self::Row>> {
        vec![
            DatatableColumn::field(Payment::ID)
                .label("Payment")
                .sortable()
                .exportable(),
            DatatableColumn::field(Payment::AMOUNT)
                .label("Amount")
                .sortable()
                .filterable()
                .exportable(),
        ]
    }

    fn default_sort() -> Vec<DatatableSort<Self::Row>> {
        vec![DatatableSort::asc(Payment::ID)]
    }

    async fn available_filters(_ctx: &DatatableContext) -> Result<Vec<DatatableFilterRow>> {
        Ok(vec![DatatableFilterRow::pair(
            DatatableFilterField::decimal_min("minimum_amount", "Minimum Amount").bind(
                Payment::AMOUNT.name(),
                DatatableFilterOp::Gte,
                DatatableFilterValueKind::Decimal,
            ),
            DatatableFilterField::decimal_max("maximum_amount", "Maximum Amount").bind(
                Payment::AMOUNT.name(),
                DatatableFilterOp::Lte,
                DatatableFilterValueKind::Decimal,
            ),
        )])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, foundry::Projection)]
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
        MerchantSalesRow::source(ORDERS_TABLE)
            .select_source(MerchantSalesRow::MERCHANT_ID, ORDERS_TABLE)
            .select_aggregate(AggregateProjection::<i64>::count_all(
                MerchantSalesRow::ORDER_COUNT.alias(),
            ))
            .select_aggregate(AggregateProjection::<Option<i64>>::sum(
                Order::TOTAL.column_ref(),
                MerchantSalesRow::TOTAL.alias(),
            ))
            .group_by(MerchantSalesRow::MERCHANT_ID.column_ref_from(ORDERS_TABLE))
    }

    fn columns() -> Vec<DatatableColumn<Self::Row>> {
        vec![
            DatatableColumn::field(MerchantSalesRow::MERCHANT_ID)
                .label("Merchant")
                .sortable()
                .filter_by(MerchantSalesRow::MERCHANT_ID.column_ref_from(ORDERS_TABLE))
                .exportable(),
            DatatableColumn::field(MerchantSalesRow::ORDER_COUNT)
                .label("Orders")
                .sortable()
                .exportable(),
            DatatableColumn::field(MerchantSalesRow::TOTAL)
                .label("Total")
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

fn request_with(
    filters: Vec<DatatableFilterInput>,
    sorts: Vec<DatatableSortInput>,
) -> DatatableRequest {
    DatatableRequest {
        page: 1,
        per_page: 20,
        sort: sorts,
        filters,
        search: None,
    }
}

#[tokio::test]
async fn registry_serves_model_and_projection_datatables() {
    let Some(runtime) = datatable_runtime().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    let app = runtime.kernel.app();
    let database = app.database().unwrap();

    reset_schema(database.as_ref()).await;
    seed_orders(database.as_ref()).await;

    let registry = app.datatables().unwrap();

    let orders = registry
        .get("orders")
        .expect("orders datatable should exist");
    let orders_response = orders
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "total".to_string(),
                    op: DatatableFilterOp::Gte,
                    value: DatatableFilterValue::Number(150),
                }],
                vec![DatatableSortInput {
                    field: "total".to_string(),
                    direction: OrderDirection::Desc,
                }],
            ),
        )
        .await
        .unwrap();

    assert_eq!(orders_response.rows.len(), 2);
    assert_eq!(
        orders_response.rows[0]
            .get("total")
            .and_then(|value| value.as_i64()),
        Some(300)
    );

    let sales = registry
        .get("merchant-sales")
        .expect("projection datatable should exist");
    let sales_response = sales
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![
                    DatatableFilterInput {
                        field: "merchant_id".to_string(),
                        op: DatatableFilterOp::Eq,
                        value: DatatableFilterValue::Number(1),
                    },
                    DatatableFilterInput {
                        field: "total".to_string(),
                        op: DatatableFilterOp::Gte,
                        value: DatatableFilterValue::Number(200),
                    },
                ],
                vec![DatatableSortInput {
                    field: "total".to_string(),
                    direction: OrderDirection::Desc,
                }],
            ),
        )
        .await
        .unwrap();

    assert_eq!(sales_response.rows.len(), 1);
    assert_eq!(
        sales_response.rows[0]
            .get("merchant_id")
            .and_then(|value| value.as_i64()),
        Some(1)
    );
    assert_eq!(
        sales_response.rows[0]
            .get("total")
            .and_then(|value| value.as_i64()),
        Some(250)
    );
}

#[tokio::test]
async fn model_datatable_relation_filters_use_declared_relation_metadata() {
    let Some(runtime) = datatable_runtime().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    let app = runtime.kernel.app();
    let database = app.database().unwrap();

    reset_schema(database.as_ref()).await;
    seed_order_relations(database.as_ref()).await;

    let registry = app.datatables().unwrap();
    let orders = registry
        .get("orders")
        .expect("orders datatable should exist");

    let merchant_response = orders
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "merchant.name".to_string(),
                    op: DatatableFilterOp::Like,
                    value: DatatableFilterValue::Text("Acme".to_string()),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(row_ids(&merchant_response), vec![1, 2]);

    let has_many_response = orders
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "items.sku".to_string(),
                    op: DatatableFilterOp::Like,
                    value: DatatableFilterValue::Text("vip".to_string()),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(row_ids(&has_many_response), vec![1, 4]);

    let many_to_many_response = orders
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "tags.name".to_string(),
                    op: DatatableFilterOp::Eq,
                    value: DatatableFilterValue::Text("urgent".to_string()),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(row_ids(&many_to_many_response), vec![1, 2]);

    let legacy_alias_response = orders
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "merchant-name".to_string(),
                    op: DatatableFilterOp::HasLike,
                    value: DatatableFilterValue::Text("Gamma".to_string()),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(row_ids(&legacy_alias_response), vec![4]);

    let relation_like_any_response = orders
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "merchant.name|merchant.slug".to_string(),
                    op: DatatableFilterOp::LikeAny,
                    value: DatatableFilterValue::Text("beta".to_string()),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(row_ids(&relation_like_any_response), vec![3]);
}

#[tokio::test]
async fn projection_datatable_downloads_and_queues_exports_through_the_registry() {
    let Some(runtime) = datatable_runtime().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    let app = runtime.kernel.app().clone();
    let database = app.database().unwrap();

    reset_schema(database.as_ref()).await;
    seed_orders(database.as_ref()).await;

    let registry = app.datatables().unwrap();
    let sales = registry
        .get("merchant-sales")
        .expect("projection datatable should exist");

    let response = sales
        .download(
            &app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "total".to_string(),
                    op: DatatableFilterOp::Gte,
                    value: DatatableFilterValue::Number(200),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap();

    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        response.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"merchant-sales.xlsx\"; filename*=UTF-8''merchant-sales.xlsx"
    );
    let (_, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    assert!(bytes.starts_with(b"PK"));

    sales
        .queue_email(
            &app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "merchant_id".to_string(),
                    op: DatatableFilterOp::Eq,
                    value: DatatableFilterValue::Number(3),
                }],
                Vec::new(),
            ),
            "reports@example.com",
        )
        .await
        .unwrap();

    assert!(runtime.kernel.run_once().await.unwrap());

    let deliveries = runtime.deliveries.lock().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].datatable_id, "merchant-sales");
    assert_eq!(deliveries[0].recipient, "reports@example.com");
    assert_eq!(deliveries[0].filename, "merchant-sales.xlsx");
    assert!(deliveries[0].data.starts_with(b"PK"));
    assert!(deliveries[0].file_backed);
}

#[tokio::test]
async fn decimal_filters_and_binding_metadata_work_through_registry_json() {
    let Some(runtime) = datatable_runtime().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    let app = runtime.kernel.app();
    let database = app.database().unwrap();

    reset_schema(database.as_ref()).await;
    seed_payments(database.as_ref()).await;

    let registry = app.datatables().unwrap();
    let payments = registry
        .get("payments")
        .expect("payments datatable should exist");

    let response = payments
        .json(
            app,
            Option::<&Actor>::None,
            request_with(
                vec![DatatableFilterInput {
                    field: "amount".to_string(),
                    op: DatatableFilterOp::Gte,
                    value: DatatableFilterValue::Text("12.50".to_string()),
                }],
                Vec::new(),
            ),
        )
        .await
        .unwrap();

    assert_eq!(response.rows.len(), 2);
    assert_eq!(
        response.rows[0]
            .get("amount")
            .and_then(|value| value.as_str()),
        Some("12.50")
    );
    assert_eq!(
        response.rows[1]
            .get("amount")
            .and_then(|value| value.as_str()),
        Some("19.99")
    );

    let minimum_amount = &response.filters[0].fields[0];
    assert_eq!(minimum_amount.name, "minimum_amount");
    assert_eq!(minimum_amount.kind, DatatableFilterKind::Number);
    assert_eq!(minimum_amount.binding.field, "amount");
    assert_eq!(minimum_amount.binding.op, DatatableFilterOp::Gte);
    assert_eq!(
        minimum_amount.binding.value_kind,
        DatatableFilterValueKind::Decimal
    );

    let maximum_amount = &response.filters[0].fields[1];
    assert_eq!(maximum_amount.name, "maximum_amount");
    assert_eq!(maximum_amount.binding.field, "amount");
    assert_eq!(maximum_amount.binding.op, DatatableFilterOp::Lte);
}

fn row_ids(response: &DatatableJsonResponse) -> Vec<i64> {
    response
        .rows
        .iter()
        .filter_map(|row| row.get("id").and_then(|value| value.as_i64()))
        .collect()
}
