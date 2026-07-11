# datatable

Server-side datatables: filtering, sorting, pagination, XLSX export

[Back to index](../index.md)

## foundry::datatable::column

```rust
struct DatatableColumn
  fn field(field: impl Into<DatatableFieldRef<Row>>) -> Self
  fn label(self, label: impl Into<String>) -> Self
  fn sortable(self) -> Self
  fn sort_by(self, expr: impl Into<Expr>) -> Self
  fn filterable(self) -> Self
  fn filter_by(self, expr: impl Into<Expr>) -> Self
  fn filter_having(self, expr: impl Into<Expr>) -> Self
  fn exportable(self) -> Self
  fn relation(self, relation: impl Into<String>) -> Self
  fn db_type(&self) -> DbType
struct DatatableFieldRef
```

## foundry::datatable::context

```rust
struct DatatableContext
  fn new( app: &'a AppContext, actor: Option<&'a Actor>, request: &'a DatatableRequest, ) -> Self
  fn with_locale_and_timezone( app: &'a AppContext, actor: Option<&'a Actor>, request: &'a DatatableRequest, locale: Option<String>, timezone: Timezone, ) -> Self
  fn t(&self, key: &str) -> String
```

## foundry::datatable::datatable_trait

```rust
trait Datatable
  fn query(ctx: &DatatableContext<'_>) -> Self::Query
  fn columns() -> Vec<DatatableColumn<Self::Row>>
  fn mappings() -> Vec<DatatableMapping<Self::Row>>
  fn filters<'life0, 'async_trait>(
  fn available_filters<'life0, 'async_trait>(
  fn relation_filters( ) -> Vec<DatatableRelationFilter<Self::Row, Self::Query>>
  fn default_sort() -> Vec<DatatableSort<Self::Row>>
  fn json<'life0, 'life1, 'async_trait>(
  fn download<'life0, 'life1, 'async_trait>(
  fn queue_email<'life0, 'life1, 'life2, 'async_trait>(
trait DatatableQuery: Clone
  fn apply_where(self, condition: Condition) -> Self
  fn apply_having(self, condition: Condition) -> Self
  fn apply_order(self, order: OrderBy) -> Self
  fn apply_limit(self, limit: u64) -> Self
  fn stream<'a, E>(
  fn get<'life0, 'life1, 'async_trait, E>(
  fn paginate<'life0, 'life1, 'async_trait, E>(
```

## foundry::datatable::download

```rust
async fn build_download_response<D>( app: &AppContext, actor: Option<&Actor>, request: DatatableRequest, ) -> Result<Response>
```

## foundry::datatable::export

```rust
pub const LEGACY_DATATABLE_EXPORT_MAX_BYTES: u64;
struct GeneratedDatatableExport
struct GeneratedDatatableExportFile
  fn datatable_id(&self) -> &str
  fn filename(&self) -> &str
  fn columns(&self) -> &[String]
  fn path(&self) -> &Path
  fn size(&self) -> u64
  async fn open(&self) -> Result<File>
  async fn read_bounded(&self, max_bytes: u64) -> Result<Vec<u8>>
trait DatatableExportDelivery
  fn deliver<'life0, 'life1, 'async_trait>(
  fn deliver_file<'life0, 'life1, 'async_trait>(
```

## foundry::datatable::export_job

```rust
struct DatatableExportJob
struct DatatableExportJobPayload
async fn dispatch_export<D: Datatable + ?Sized>( app: &AppContext, actor: Option<&Actor>, request: DatatableRequest, recipient: &str, ) -> Result<DatatableExportAccepted>
```

## foundry::datatable::filter_engine

```rust
fn apply_auto_filters<Row: 'static, Q>( query: Q, filters: &[DatatableFilterInput], columns: &[DatatableColumn<Row>], ) -> Result<Q>
fn apply_auto_filters_with_relation_filters<Row: 'static, Q>( query: Q, filters: &[DatatableFilterInput], columns: &[DatatableColumn<Row>], relation_filters: &[DatatableRelationFilter<Row, Q>], ) -> Result<Q>
fn apply_default_sorts<Row: 'static, Q>( query: Q, sorts: &[DatatableSort<Row>], ) -> Result<Q>
fn apply_sorts<Row: 'static, Q>( query: Q, sorts: &[DatatableSortInput], columns: &[DatatableColumn<Row>], ) -> Result<Q>
```

## foundry::datatable::filter_meta

```rust
enum DatatableFilterKind { Text, Number, Select, Checkbox, Date, DateTime }
enum DatatableFilterValueKind { Text, Boolean, Integer, Decimal, Date, DateTime, Values }
struct DatatableFilterBinding
  fn new( field: impl Into<String>, op: DatatableFilterOp, value_kind: DatatableFilterValueKind, ) -> Self
struct DatatableFilterField
  fn text(name: impl Into<String>, label: impl Into<String>) -> Self
  fn text_like(name: impl Into<String>, label: impl Into<String>) -> Self
  fn text_search(name: impl Into<String>, label: impl Into<String>) -> Self
  fn text_search_fields<Row, I, F>( name: impl Into<String>, label: impl Into<String>, fields: I, ) -> Self
  fn number(name: impl Into<String>, label: impl Into<String>) -> Self
  fn decimal_min(name: impl Into<String>, label: impl Into<String>) -> Self
  fn decimal_max(name: impl Into<String>, label: impl Into<String>) -> Self
  fn select(name: impl Into<String>, label: impl Into<String>) -> Self
  fn checkbox(name: impl Into<String>, label: impl Into<String>) -> Self
  fn date(name: impl Into<String>, label: impl Into<String>) -> Self
  fn date_from(name: impl Into<String>, label: impl Into<String>) -> Self
  fn date_to(name: impl Into<String>, label: impl Into<String>) -> Self
  fn datetime(name: impl Into<String>, label: impl Into<String>) -> Self
  fn placeholder(self, placeholder: impl Into<String>) -> Self
  fn options<I>(self, options: I) -> Self
  fn enum_options<E: FoundryAppEnum>(self) -> Self
  fn help(self, help: impl Into<String>) -> Self
  fn nullable(self) -> Self
  fn server_field<Row, F>(self, field: F) -> Self
  fn server_fields<Row, I, F>(self, fields: I) -> Self
  fn bind( self, field: impl Into<String>, op: DatatableFilterOp, value_kind: DatatableFilterValueKind, ) -> Self
  fn enum_select<E: FoundryAppEnum>( name: impl Into<String>, label: impl Into<String>, ) -> Self
struct DatatableFilterOption
  fn new(value: impl Into<String>, label: impl Into<String>) -> Self
struct DatatableFilterRow
  fn single(field: DatatableFilterField) -> Self
  fn pair(left: DatatableFilterField, right: DatatableFilterField) -> Self
```

## foundry::datatable::json

```rust
async fn build_json_response<D>( app: &AppContext, actor: Option<&Actor>, request: DatatableRequest, ) -> Result<DatatableJsonResponse>
```

## foundry::datatable::mapping

```rust
struct DatatableMapping
  fn new<F>(name: impl Into<String>, callback: F) -> Self
  fn compute(&self, model: &M, ctx: &DatatableContext<'_>) -> DatatableValue
```

## foundry::datatable::registry

```rust
struct DatatableAdapter
  fn new() -> Self
struct DatatableRegistry
  fn get(&self, id: &str) -> Option<Arc<dyn DynDatatable>>
  fn ids(&self) -> Vec<&str>
trait DynDatatable
  fn id(&self) -> &str
  fn json<'life0, 'life1, 'life2, 'async_trait>(
  fn download<'life0, 'life1, 'life2, 'async_trait>(
  fn queue_email<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn export_file<'life0, 'life1, 'life2, 'async_trait>(
```

## foundry::datatable::relation_filter

```rust
struct DatatableRelationColumn
  fn field<T>(column: Column<Row, T>) -> Self
struct DatatableRelationFilter
  fn alias(self, alias: impl Into<String>) -> Self
  fn relation<To, T>( field: impl Into<String>, relation: RelationDef<From, To>, column: Column<To, T>, ) -> Self
  fn relation_columns<To>( field: impl Into<String>, relation: RelationDef<From, To>, columns: impl IntoIterator<Item = DatatableRelationColumn<To>>, ) -> Self
  fn many_to_many<To, Pivot, T>( field: impl Into<String>, relation: ManyToManyDef<From, To, Pivot>, column: Column<To, T>, ) -> Self
  fn many_to_many_columns<To, Pivot>( field: impl Into<String>, relation: ManyToManyDef<From, To, Pivot>, columns: impl IntoIterator<Item = DatatableRelationColumn<To>>, ) -> Self
```

## foundry::datatable::request

```rust
enum DatatableFilterOp { Show 17 variants    Eq, NotEq, Like, Gt, Gte, ... +12 more }
enum DatatableFilterValue { Text, Bool, Number, Values }
struct DatatableFilterInput
struct DatatableRequest
  fn text(&self, name: &str) -> Option<&str>
  fn bool(&self, name: &str) -> Option<bool>
  fn date(&self, name: &str) -> Option<Date>
  fn datetime(&self, name: &str) -> Option<DateTime>
  fn values(&self, name: &str) -> Collection<String>
  fn from_query_params(params: &HashMap<String, String>) -> Self
struct DatatableSortInput
```

## foundry::datatable::response

```rust
struct DatatableActorSnapshot
struct DatatableColumnMeta
struct DatatableExportAccepted
struct DatatableJsonResponse
struct DatatablePaginationMeta
  fn new(page: u64, per_page: u64, total: u64) -> Self
```

## foundry::datatable::sort

```rust
struct DatatableSort
  fn asc(field: impl Into<DatatableFieldRef<Row>>) -> Self
  fn desc(field: impl Into<DatatableFieldRef<Row>>) -> Self
```

## foundry::datatable::value

```rust
enum DatatableValue { Null, String, Number, Bool, Date, DateTime }
  fn null() -> Self
  fn string(value: impl Into<String>) -> Self
  fn number(value: impl Into<Number>) -> Self
  fn bool(value: bool) -> Self
  fn date(value: Date) -> Self
  fn datetime(value: DateTime) -> Self
```

## Notes

- JSON responses clamp `DatatableRequest.per_page` to `datatable.max_per_page` unless the cap is `0`.
- XLSX downloads and queued exports apply `datatable.max_export_rows` before loading rows into memory; `0` disables the cap.
