# database

AST-first query system: models, relations, projections, compiler

[Back to index](../index.md)

## foundry::database

```rust
pub type AfterCommitCallback = Box<dyn FnOnce(AppContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;
pub type DbRecordStream<'a> = BoxStream<'a, Result<DbRecord>>;
pub type RestoreModel<M> = UpdateModel<M>;
enum Loaded { Unloaded, Loaded }
  fn new(value: T) -> Self
  fn is_loaded(&self) -> bool
  fn as_ref(&self) -> Option<&T>
  fn into_option(self) -> Option<T>
enum ModelFeatureSetting { Default, Enabled, Disabled }
  const fn is_enabled(self) -> bool
enum ModelPrimaryKeyStrategy { UuidV7, Manual }
  const fn generates_value(self) -> bool
struct AggregateProjection
  fn alias(&self) -> &str
  fn decode(&self, record: &DbRecord) -> Result<T>
  fn count_all(alias: &'static str) -> Self
  fn count(expr: impl Into<Expr>, alias: &'static str) -> Self
  fn count_distinct(expr: impl Into<Expr>, alias: &'static str) -> Self
  fn sum(expr: impl Into<Expr>, alias: &'static str) -> Self
  fn avg(expr: impl Into<Expr>, alias: &'static str) -> Self
  fn min(expr: impl Into<Expr>, alias: &'static str) -> Self
  fn max(expr: impl Into<Expr>, alias: &'static str) -> Self
struct Case
  fn when(condition: Condition, result: impl Into<Expr>) -> CaseBuilder
struct Column
  const fn new( table: &'static str, name: &'static str, db_type: DbType, ) -> Self
  const fn info(&self) -> ColumnInfo
  const fn name(&self) -> &'static str
  const fn db_type(&self) -> DbType
  fn column_ref(&self) -> ColumnRef
  fn expr(&self) -> Expr
  fn cast(&self, db_type: DbType) -> Expr
  fn cast_text(&self) -> Expr
  fn asc(&self) -> OrderBy
  fn desc(&self) -> OrderBy
  fn eq<V>(&self, value: V) -> Condition
  fn not_eq<V>(&self, value: V) -> Condition
  fn gt<V>(&self, value: V) -> Condition
  fn gte<V>(&self, value: V) -> Condition
  fn lt<V>(&self, value: V) -> Condition
  fn lte<V>(&self, value: V) -> Condition
  fn in_list<I, V>(&self, values: I) -> Condition
  fn not_in_list<I, V>(&self, values: I) -> Condition
  fn is_null(&self) -> Condition
  fn is_not_null(&self) -> Condition
  fn like(&self, value: impl Into<String>) -> Condition
  fn ieq(&self, value: impl Into<String>) -> Condition
  fn not_like(&self, value: impl Into<String>) -> Condition
  fn ilike(&self, value: impl Into<String>) -> Condition
  fn json(&self) -> JsonExprBuilder
struct ColumnInfo
  const fn new(name: &'static str, db_type: DbType) -> Self
  const fn with_write_mutator( self, write_mutator: for<'a> fn(&'a ModelHookContext<'a>, DbValue) -> Pin<Box<dyn Future<Output = Result<DbValue>> + Send + 'a>>, ) -> Self
  const fn write_mutator( &self, ) -> Option<for<'a> fn(&'a ModelHookContext<'a>, DbValue) -> Pin<Box<dyn Future<Output = Result<DbValue>> + Send + 'a>>>
struct CreateDraft
  fn set<T, V>(&mut self, column: Column<M, T>, value: V) -> &mut Self
  fn set_expr<T>( &mut self, column: Column<M, T>, expr: impl Into<Expr>, ) -> &mut Self
  fn set_null<T>(&mut self, column: Column<M, T>) -> &mut Self
  fn assigned_columns(&self) -> Vec<&str>
  fn pending_record(&self) -> DbRecord
struct CreateManyModel
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn without_lifecycle(self) -> Self
  fn row<F>(self, build: F) -> Self
  fn on_conflict_columns<I, C>(self, columns: I) -> Self
  fn on_conflict_constraint(self, constraint: impl Into<String>) -> Self
  fn do_nothing(self) -> Self
  fn do_update(self) -> Self
  fn set_conflict<T, V>(self, column: Column<M, T>, value: V) -> Self
  fn set_conflict_expr( self, column: impl Into<ColumnRef>, expr: impl Into<Expr>, ) -> Self
  fn set_excluded<T>(self, column: Column<M, T>) -> Self
  fn where_(self, condition: Condition) -> Self
  async fn execute<E>(&self, executor: &E) -> Result<u64>
  async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
  async fn first<E>(&self, executor: &E) -> Result<Option<M>>
  fn to_compiled_sql(&self) -> Result<CompiledSql>
  async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
  async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
struct CreateModel
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn set<T, V>(self, column: Column<M, T>, value: V) -> Self
  fn set_expr<T>(self, column: Column<M, T>, expr: impl Into<Expr>) -> Self
  fn on_conflict_columns<I, C>(self, columns: I) -> Self
  fn on_conflict_constraint(self, constraint: impl Into<String>) -> Self
  fn do_nothing(self) -> Self
  fn do_update(self) -> Self
  fn set_conflict<T, V>(self, column: Column<M, T>, value: V) -> Self
  fn set_conflict_expr( self, column: impl Into<ColumnRef>, expr: impl Into<Expr>, ) -> Self
  fn set_excluded<T>(self, column: Column<M, T>) -> Self
  fn where_(self, condition: Condition) -> Self
  async fn execute<E>(&self, executor: &E) -> Result<u64>
  async fn save<E>(&self, executor: &E) -> Result<M>
  async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
  async fn first<E>(&self, executor: &E) -> Result<Option<M>>
  fn to_compiled_sql(&self) -> Result<CompiledSql>
  async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
  async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
struct CreateRow
  fn set<T, V>(self, column: Column<M, T>, value: V) -> Self
  fn set_expr<T>(self, column: Column<M, T>, expr: impl Into<Expr>) -> Self
  fn set_null<T>(self, column: Column<M, T>) -> Self
struct Cte
  fn new(name: impl Into<String>, query: impl Into<QueryAst>) -> Self
  fn materialized(self) -> Self
  fn not_materialized(self) -> Self
  fn recursive(self) -> Self
struct CursorInfo
struct CursorMeta
struct CursorPaginated
  fn encode_cursor(value: &impl Display) -> String
  fn with_cursors( self, first_value: Option<&impl Display>, last_value: Option<&impl Display>, ) -> Self
struct CursorPagination
  fn new(per_page: u64) -> Self
  fn after(self, cursor: impl Into<String>) -> Self
  fn before(self, cursor: impl Into<String>) -> Self
struct DatabaseManager
  fn disabled() -> Self
  async fn from_config(config: &DatabaseConfig) -> Result<Self>
  fn is_configured(&self) -> bool
  fn has_read_pool(&self) -> bool
  fn pool(&self) -> Result<&PgPool>
  fn register_type_adapter( &self, postgres_type_name: impl Into<String>, db_type: DbType, ) -> Result<()>
  fn registered_type_adapter( &self, postgres_type_name: &str, ) -> Result<Option<DbType>>
  async fn ping_write(&self) -> Result<()>
  async fn ping_read(&self) -> Result<()>
  async fn ping(&self) -> Result<()>
  async fn begin(&self) -> Result<DatabaseTransaction>
  async fn raw_query( &self, sql: &str, bindings: &[DbValue], ) -> Result<Vec<DbRecord>>
  async fn raw_query_with( &self, sql: &str, bindings: &[DbValue], options: QueryExecutionOptions, ) -> Result<Vec<DbRecord>>
  async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64>
  async fn raw_execute_with( &self, sql: &str, bindings: &[DbValue], options: QueryExecutionOptions, ) -> Result<u64>
  fn raw_stream<'a>( &'a self, sql: &'a str, bindings: &'a [DbValue], options: QueryExecutionOptions, ) -> DbRecordStream<'a>
struct DatabaseTransaction
  async fn raw_query( &self, sql: &str, bindings: &[DbValue], ) -> Result<Vec<DbRecord>>
  async fn raw_query_with( &self, sql: &str, bindings: &[DbValue], options: QueryExecutionOptions, ) -> Result<Vec<DbRecord>>
  async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64>
  async fn raw_execute_with( &self, sql: &str, bindings: &[DbValue], options: QueryExecutionOptions, ) -> Result<u64>
  async fn set_local_config(&self, name: &str, value: &str) -> Result<()>
  fn raw_stream<'a>( &'a self, sql: &'a str, bindings: &'a [DbValue], options: QueryExecutionOptions, ) -> DbRecordStream<'a>
  async fn commit(self) -> Result<()>
  async fn rollback(self) -> Result<()>
struct DbRecord
  fn decode<T>(&self, key: &str) -> Result<T>
  fn decode_column<M, T>(&self, column: Column<M, T>) -> Result<T>
  fn new() -> Self
  fn insert(&mut self, key: impl Into<String>, value: DbValue)
  fn get(&self, key: &str) -> Option<&DbValue>
  fn iter(&self) -> impl Iterator<Item = (&String, &DbValue)>
  fn text(&self, field: &str) -> String
  fn try_text(&self, field: &str) -> Result<String>
  fn text_or_uuid(&self, field: &str) -> String
  fn try_text_or_uuid(&self, field: &str) -> Result<String>
  fn optional_text(&self, field: &str) -> Option<String>
struct DeleteModel
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn where_(self, condition: Condition) -> Self
  fn using(self, source: impl Into<FromItem>) -> Self
  fn allow_all(self) -> Self
  fn without_lifecycle(self) -> Self
  async fn execute<E>(&self, executor: &E) -> Result<u64>
  fn to_compiled_sql(&self) -> Result<CompiledSql>
  async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
  async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
struct JsonExprBuilder
  fn new(expr: impl Into<Expr>) -> Self
  fn key(self, key: impl Into<String>) -> Self
  fn index(self, index: i64) -> Self
  fn as_json(self) -> Expr
  fn as_text(self) -> Expr
  fn like(self, value: impl Into<String>) -> Condition
  fn not_like(self, value: impl Into<String>) -> Condition
  fn ilike(self, value: impl Into<String>) -> Condition
  fn contains(self, value: impl Into<Value>) -> Condition
  fn contained_by(self, value: impl Into<Value>) -> Condition
  fn has_key(self, key: impl Into<String>) -> Condition
  fn has_any_keys<I, S>(self, keys: I) -> Condition
  fn has_all_keys<I, S>(self, keys: I) -> Condition
struct MigrationContext
  fn app(&self) -> &AppContext
  fn database(&self) -> &DatabaseManager
  fn executor(&self) -> &dyn QueryExecutor
  async fn raw_query( &self, sql: &str, bindings: &[DbValue], ) -> Result<Vec<DbRecord>>
  async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64>
struct ModelBehavior
  const fn new( timestamps: ModelFeatureSetting, soft_deletes: ModelFeatureSetting, ) -> Self
struct ModelCreatedEvent
struct ModelCreatingEvent
struct ModelDeletedEvent
struct ModelDeletingEvent
struct ModelHookContext
  fn app(&self) -> &AppContext
  fn database(&self) -> &DatabaseManager
  fn transaction(&self) -> &DatabaseTransaction
  fn actor(&self) -> Option<&Actor>
  fn origin(&self) -> Option<&EventOrigin>
  fn executor(&self) -> &dyn QueryExecutor
  fn events(&self) -> Result<Arc<EventBus>>
  async fn dispatch<E>(&self, event: E) -> Result<()>
struct ModelLifecycleSnapshot
  fn for_model<M: Model>( before: Option<DbRecord>, after: Option<DbRecord>, pending: Option<DbRecord>, ) -> Self
struct ModelQuery
  fn new(table: &'static TableMeta<M>) -> Self
  fn without_defaults(self) -> Self
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn use_write_pool(self) -> Self
  fn with_stream_batch_size(self, batch_size: usize) -> Self
  fn with_cte(self, cte: Cte) -> Self
  fn where_(self, condition: Condition) -> Self
  fn where_in<T, I, V>(self, column: Column<M, T>, values: I) -> Self
  fn where_not_in<T, I, V>(self, column: Column<M, T>, values: I) -> Self
  fn group_by(self, expr: impl Into<Expr>) -> Self
  fn having(self, condition: Condition) -> Self
  fn search<T>(self, columns: &[Column<M, T>], query: &str) -> Self
  fn with_trashed(self) -> Self
  fn only_trashed(self) -> Self
  fn limit(self, limit: u64) -> Self
  fn offset(self, offset: u64) -> Self
  fn order_by(self, order: OrderBy) -> Self
  fn scope(self, f: impl FnOnce(Self) -> Self) -> Self
  fn with<To>(self, relation: RelationDef<M, To>) -> Self
  fn with_attachments(self, collection: impl Into<String>) -> Self
  fn with_translated_field(self, field: impl Into<String>) -> Self
  fn with_translations_for(self, locale: impl Into<String>) -> Self
  fn with_all_translations(self) -> Self
  fn with_many_to_many<To, Pivot>( self, relation: ManyToManyDef<M, To, Pivot>, ) -> Self
  fn with_aggregate<Value>( self, aggregate: RelationAggregateDef<M, Value>, ) -> Self
  fn where_has<To, F>(self, relation: RelationDef<M, To>, scope: F) -> Self
  fn where_has_many_to_many<To, Pivot, F>( self, relation: ManyToManyDef<M, To, Pivot>, scope: F, ) -> Self
  fn ast(&self) -> QueryAst
  fn to_compiled_sql(&self) -> Result<CompiledSql>
  fn for_update(self) -> Self
  fn for_no_key_update(self) -> Self
  fn for_share(self) -> Self
  fn for_key_share(self) -> Self
  fn of<I, S>(self, aliases: I) -> Self
  fn skip_locked(self) -> Self
  fn nowait(self) -> Self
  async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
  async fn all<E>(&self, executor: &E) -> Result<Collection<M>>
  fn stream<'a, E>( &'a self, executor: &'a E, ) -> Result<BoxStream<'a, Result<M>>>
  async fn first<E>(&self, executor: &E) -> Result<Option<M>>
  async fn first_or_fail<E>(&self, executor: &E) -> Result<M>
  async fn find<E, K>(&self, executor: &E, key: K) -> Result<Option<M>>
  async fn find_or_fail<E, K>(&self, executor: &E, key: K) -> Result<M>
  async fn find_many<E, I, K>( &self, executor: &E, keys: I, ) -> Result<Collection<M>>
  async fn exists<E>(&self, executor: &E) -> Result<bool>
  async fn doesnt_exist<E>(&self, executor: &E) -> Result<bool>
  async fn value<E, T>( &self, executor: &E, column: Column<M, T>, ) -> Result<Option<T>>
  async fn chunk<E, F, Fut>( &self, executor: &E, size: u64, handler: F, ) -> Result<()>
  async fn chunk_by_id<E, T, F, Fut>( &self, executor: &E, column: Column<M, T>, size: u64, handler: F, ) -> Result<()>
  async fn each_by_id<E, T, F, Fut>( &self, executor: &E, column: Column<M, T>, size: u64, handler: F, ) -> Result<()>
  async fn paginate<E>( &self, executor: &E, pagination: Pagination, ) -> Result<Paginated<M>>
  async fn cursor_paginate<E, V>( self, executor: &E, column: Column<M, V>, cursor: CursorPagination, ) -> Result<CursorPaginated<M>>
  async fn count<E>(&self, executor: &E) -> Result<u64>
  async fn count_distinct<E, T>( &self, executor: &E, column: Column<M, T>, ) -> Result<u64>
  async fn sum<E, T>( &self, executor: &E, column: Column<M, T>, ) -> Result<Option<T>>
  async fn avg<E, T>( &self, executor: &E, column: Column<M, T>, ) -> Result<Option<T>>
  async fn min<E, T>( &self, executor: &E, column: Column<M, T>, ) -> Result<Option<T>>
  async fn max<E, T>( &self, executor: &E, column: Column<M, T>, ) -> Result<Option<T>>
  async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
  async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
struct ModelUpdatedEvent
struct ModelUpdatingEvent
struct NoModelLifecycle
struct Paginated
  fn to_response(&self, base_url: &str) -> PaginatedResponse<&T>
struct PaginatedResponse
struct Pagination
  fn new(page: u64, per_page: u64) -> Self
  fn offset(&self) -> u64
struct PaginationLinks
struct PaginationMeta
struct ProjectionField
  const fn new(alias: &'static str, db_type: DbType) -> Self
  const fn from_source( alias: &'static str, source_column: &'static str, db_type: DbType, ) -> Self
  const fn info(&self) -> ProjectionFieldInfo
  const fn alias(&self) -> &'static str
  const fn db_type(&self) -> DbType
  fn column_ref(&self) -> ColumnRef
  fn column_ref_from(&self, table_alias: &str) -> ColumnRef
  fn decode(&self, record: &DbRecord) -> Result<T>
  fn select(&self, expr: impl Into<Expr>) -> SelectItem
  fn select_from(&self, table_alias: &str) -> Result<SelectItem>
struct ProjectionFieldInfo
  const fn new(alias: &'static str, db_type: DbType) -> Self
  const fn from_source( alias: &'static str, source_column: &'static str, db_type: DbType, ) -> Self
  fn select_from(self, table_alias: &str) -> Result<SelectItem>
struct ProjectionMeta
  const fn new( fields: &'static [ProjectionFieldInfo], hydrate: fn(&DbRecord) -> Result<P>, ) -> Self
  const fn fields(&self) -> &'static [ProjectionFieldInfo]
  fn hydrate_record(&self, record: &DbRecord) -> Result<P>
  fn source_select_items(&self, table_alias: &str) -> Result<Vec<SelectItem>>
struct ProjectionQuery
  fn table(source: impl Into<FromItem>) -> Self
  fn with_cte(self, cte: Cte) -> Self
  fn distinct(self) -> Self
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn select_field<T>( self, field: ProjectionField<P, T>, expr: impl Into<Expr>, ) -> Self
  fn select_source<T>( self, field: ProjectionField<P, T>, table_alias: &str, ) -> Self
  fn select_aggregate<T>(self, projection: AggregateProjection<T>) -> Self
  fn join( self, kind: JoinKind, table: impl Into<FromItem>, on: Condition, ) -> Self
  fn inner_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn left_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn right_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn full_outer_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn cross_join(self, table: impl Into<FromItem>) -> Self
  fn inner_join_lateral( self, table: impl Into<FromItem>, on: Condition, ) -> Self
  fn left_join_lateral( self, table: impl Into<FromItem>, on: Condition, ) -> Self
  fn cross_join_lateral(self, table: impl Into<FromItem>) -> Self
  fn where_(self, condition: Condition) -> Self
  fn group_by(self, expr: impl Into<Expr>) -> Self
  fn having(self, condition: Condition) -> Self
  fn order_by(self, order: OrderBy) -> Self
  fn limit(self, limit: u64) -> Self
  fn offset(self, offset: u64) -> Self
  fn scope(self, f: impl FnOnce(Self) -> Self) -> Self
  fn union(self, other: Self) -> Self
  fn union_all(self, other: Self) -> Self
  fn ast(&self) -> &QueryAst
  fn to_compiled_sql(&self) -> Result<CompiledSql>
  fn for_update(self) -> Self
  fn for_no_key_update(self) -> Self
  fn for_share(self) -> Self
  fn for_key_share(self) -> Self
  fn of<I, S>(self, aliases: I) -> Self
  fn skip_locked(self) -> Self
  fn nowait(self) -> Self
  async fn get<E>(&self, executor: &E) -> Result<Collection<P>>
  fn stream<'a, E>( &'a self, executor: &'a E, ) -> Result<BoxStream<'a, Result<P>>>
  async fn first<E>(&self, executor: &E) -> Result<Option<P>>
  async fn paginate<E>( &self, executor: &E, pagination: Pagination, ) -> Result<Paginated<P>>
  async fn count<E>(&self, executor: &E) -> Result<u64>
  async fn count_distinct<E, T>( &self, executor: &E, field: ProjectionField<P, T>, ) -> Result<u64>
  async fn sum<E, T>( &self, executor: &E, field: ProjectionField<P, T>, ) -> Result<Option<T>>
  async fn avg<E, T>( &self, executor: &E, field: ProjectionField<P, T>, ) -> Result<Option<T>>
  async fn min<E, T>( &self, executor: &E, field: ProjectionField<P, T>, ) -> Result<Option<T>>
  async fn max<E, T>( &self, executor: &E, field: ProjectionField<P, T>, ) -> Result<Option<T>>
  async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
  async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
struct Query
  fn table(source: impl Into<FromItem>) -> Self
  fn insert_into(table: impl Into<TableRef>) -> Self
  fn insert_many_into(table: impl Into<TableRef>) -> Self
  fn insert_select_into( table: impl Into<TableRef>, select: impl Into<QueryAst>, ) -> Self
  fn update_table(table: impl Into<TableRef>) -> Self
  fn delete_from(table: impl Into<TableRef>) -> Self
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn with_cte(self, cte: Cte) -> Self
  fn distinct(self) -> Self
  fn select<I, C>(self, columns: I) -> Self
  fn select_item(self, item: SelectItem) -> Self
  fn select_expr( self, expr: impl Into<Expr>, alias: impl Into<String>, ) -> Self
  fn select_aggregate<T>(self, projection: AggregateProjection<T>) -> Self
  fn join( self, kind: JoinKind, table: impl Into<FromItem>, on: Condition, ) -> Self
  fn join_lateral( self, kind: JoinKind, table: impl Into<FromItem>, on: Option<Condition>, ) -> Self
  fn inner_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn left_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn right_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn full_outer_join(self, table: impl Into<FromItem>, on: Condition) -> Self
  fn cross_join(self, table: impl Into<FromItem>) -> Self
  fn left_join_lateral( self, table: impl Into<FromItem>, on: Condition, ) -> Self
  fn cross_join_lateral(self, table: impl Into<FromItem>) -> Self
  fn inner_join_lateral( self, table: impl Into<FromItem>, on: Condition, ) -> Self
  fn where_(self, condition: Condition) -> Self
  fn where_eq( self, column: impl Into<ColumnRef>, value: impl Into<DbValue>, ) -> Self
  fn where_ieq( self, column: impl Into<ColumnRef>, value: impl Into<String>, ) -> Self
  fn where_in<I, V>(self, column: impl Into<ColumnRef>, values: I) -> Self
  fn where_not_in<I, V>(self, column: impl Into<ColumnRef>, values: I) -> Self
  fn group_by(self, expr: impl Into<Expr>) -> Self
  fn having(self, condition: Condition) -> Self
  fn limit(self, limit: u64) -> Self
  fn offset(self, offset: u64) -> Self
  fn order_by(self, order: OrderBy) -> Self
  fn value( self, column: impl Into<ColumnRef>, value: impl Into<DbValue>, ) -> Self
  fn value_expr( self, column: impl Into<ColumnRef>, expr: impl Into<Expr>, ) -> Self
  fn values<I, C, V>(self, values: I) -> Self
  fn row<I, C, V>(self, values: I) -> Self
  fn rows<R, I, C, V>(self, rows: R) -> Self
  fn on_conflict_columns<I, C>(self, columns: I) -> Self
  fn on_conflict_constraint(self, constraint: impl Into<String>) -> Self
  fn do_nothing(self) -> Self
  fn do_update(self) -> Self
  fn set( self, column: impl Into<ColumnRef>, value: impl Into<DbValue>, ) -> Self
  fn set_expr( self, column: impl Into<ColumnRef>, expr: impl Into<Expr>, ) -> Self
  fn set_excluded(self, column: impl Into<ColumnRef>) -> Self
  fn from(self, source: impl Into<FromItem>) -> Self
  fn using(self, source: impl Into<FromItem>) -> Self
  fn returning<I, C>(self, columns: I) -> Self
  fn union(self, other: Self) -> Self
  fn union_all(self, other: Self) -> Self
  fn ast(&self) -> &QueryAst
  fn compile(&self) -> Result<CompiledSql>
  fn to_compiled_sql(&self) -> Result<CompiledSql>
  fn for_update(self) -> Self
  fn for_no_key_update(self) -> Self
  fn for_share(self) -> Self
  fn for_key_share(self) -> Self
  fn of<I, S>(self, aliases: I) -> Self
  fn skip_locked(self) -> Self
  fn nowait(self) -> Self
  async fn get<E>(&self, executor: &E) -> Result<Collection<DbRecord>>
  async fn all<E>(&self, executor: &E) -> Result<Collection<DbRecord>>
  async fn first<E>(&self, executor: &E) -> Result<Option<DbRecord>>
  async fn execute<E>(&self, executor: &E) -> Result<u64>
  fn stream<'a, E>(&'a self, executor: &'a E) -> Result<DbRecordStream<'a>>
  async fn paginate<E>( &self, executor: &E, pagination: Pagination, ) -> Result<Paginated<DbRecord>>
  async fn count<E>(&self, executor: &E) -> Result<u64>
  async fn count_distinct<E>( &self, executor: &E, expr: impl Into<Expr>, ) -> Result<u64>
  async fn sum<E, T>( &self, executor: &E, expr: impl Into<Expr>, ) -> Result<Option<T>>
  async fn avg<E, T>( &self, executor: &E, expr: impl Into<Expr>, ) -> Result<Option<T>>
  async fn min<E, T>( &self, executor: &E, expr: impl Into<Expr>, ) -> Result<Option<T>>
  async fn max<E, T>( &self, executor: &E, expr: impl Into<Expr>, ) -> Result<Option<T>>
  async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
  async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
struct QueryExecutionOptions
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn with_write_pool(self) -> Self
struct SeederContext
  fn app(&self) -> &AppContext
  fn database(&self) -> &DatabaseManager
  fn executor(&self) -> &dyn QueryExecutor
  async fn raw_query( &self, sql: &str, bindings: &[DbValue], ) -> Result<Vec<DbRecord>>
  async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64>
struct SlowQueryEntry
struct Sql
  fn count_all() -> Expr
  fn count(expr: impl Into<Expr>) -> Expr
  fn count_distinct(expr: impl Into<Expr>) -> Expr
  fn count_when(condition: Condition) -> Expr
  fn sum(expr: impl Into<Expr>) -> Expr
  fn avg(expr: impl Into<Expr>) -> Expr
  fn min(expr: impl Into<Expr>) -> Expr
  fn max(expr: impl Into<Expr>) -> Expr
  fn function( name: impl Into<String>, args: impl IntoIterator<Item = Expr>, ) -> Expr
  fn coalesce(args: impl IntoIterator<Item = Expr>) -> Expr
  fn concat_ws( separator: impl Into<String>, args: impl IntoIterator<Item = Expr>, ) -> Expr
  fn lower(expr: impl Into<Expr>) -> Expr
  fn upper(expr: impl Into<Expr>) -> Expr
  fn date_trunc(granularity: impl Into<String>, expr: impl Into<Expr>) -> Expr
  fn extract(field: impl Into<String>, expr: impl Into<Expr>) -> Expr
  fn json_text_or_first( expr: impl Into<Expr>, preferred_key: impl Into<String>, ) -> Expr
  fn to_timestamp_millis(millis: impl Into<Expr>) -> Expr
  fn now() -> Expr
  fn uuid_v7() -> Expr
  fn not(expr: impl Into<Expr>) -> Expr
  fn negate(expr: impl Into<Expr>) -> Expr
  fn add(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr
  fn subtract(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr
  fn multiply(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr
  fn divide(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr
  fn concat(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr
  fn op( left: impl Into<Expr>, operator: impl Into<String>, right: impl Into<Expr>, ) -> Expr
struct TableMeta
  fn new( name: &'static str, columns: &'static [ColumnInfo], primary_key: &'static str, primary_key_strategy: ModelPrimaryKeyStrategy, behavior: ModelBehavior, hydrate: fn(&DbRecord) -> Result<M>, ) -> Self
  const fn name(&self) -> &'static str
  fn table_ref(&self) -> TableRef
  fn primary_key_ref(&self) -> ColumnRef
  const fn primary_key_name(&self) -> &'static str
  const fn primary_key_strategy(&self) -> ModelPrimaryKeyStrategy
  const fn columns(&self) -> &'static [ColumnInfo]
  const fn behavior(&self) -> ModelBehavior
  fn column_info(&self, name: &str) -> Option<&ColumnInfo>
  fn primary_key_column_info(&self) -> Option<&ColumnInfo>
  fn created_at_column_info(&self) -> Option<&ColumnInfo>
  fn updated_at_column_info(&self) -> Option<&ColumnInfo>
  fn deleted_at_column_info(&self) -> Option<&ColumnInfo>
  fn timestamps_enabled(&self, _app: &AppContext) -> Result<bool>
  fn soft_deletes_enabled(&self) -> bool
  fn all_select_items(&self) -> Vec<SelectItem>
  fn hydrate_record(&self, record: &DbRecord) -> Result<M>
struct UpdateDraft
  fn set<T, V>(&mut self, column: Column<M, T>, value: V) -> &mut Self
  fn set_expr<T>( &mut self, column: Column<M, T>, expr: impl Into<Expr>, ) -> &mut Self
  fn set_null<T>(&mut self, column: Column<M, T>) -> &mut Self
  fn changed_columns(&self) -> Vec<&str>
  fn pending_record(&self) -> DbRecord
struct UpdateModel
  fn with_timeout(self, timeout: Duration) -> Self
  fn with_label(self, label: impl Into<String>) -> Self
  fn set<T, V>(self, column: Column<M, T>, value: V) -> Self
  fn set_expr<T>(self, column: Column<M, T>, expr: impl Into<Expr>) -> Self
  fn set_null<T>(self, column: Column<M, T>) -> Self
  fn where_(self, condition: Condition) -> Self
  fn from(self, source: impl Into<FromItem>) -> Self
  fn allow_all(self) -> Self
  fn without_lifecycle(self) -> Self
  async fn execute<E>(&self, executor: &E) -> Result<u64>
  async fn save<E>(&self, executor: &E) -> Result<M>
  async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
  async fn first<E>(&self, executor: &E) -> Result<Option<M>>
  fn to_compiled_sql(&self) -> Result<CompiledSql>
  async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
  async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
struct Window
  fn partition_by(expr: impl Into<Expr>) -> WindowBuilder
  fn order_by(order: OrderBy) -> WindowBuilder
  fn over(function: impl Into<Expr>, builder: WindowBuilder) -> Expr
struct WindowBuilder
  fn partition_by(self, expr: impl Into<Expr>) -> Self
  fn order_by(self, order: OrderBy) -> Self
  fn rows_between( self, start: WindowFrameBound, end: WindowFrameBound, ) -> Self
  fn range_between( self, start: WindowFrameBound, end: WindowFrameBound, ) -> Self
  fn finish(self) -> WindowSpec
trait AfterCommitSink
  fn supports_after_commit(&self) -> bool
  fn defer_after_commit(&self, callback: AfterCommitCallback)
trait FromDbValue
  fn from_db_value(value: &DbValue) -> Result<Self>
trait IntoColumnValue
  fn into_column_value(self) -> T
trait IntoFieldValue
  fn into_field_value(self, db_type: DbType) -> DbValue
trait IntoLoadableRelation: Model>
  fn into_relation(self) -> AnyRelation<M>
trait MigrationFile
  fn up<'life0, 'life1, 'async_trait>(
  fn down<'life0, 'life1, 'async_trait>(
  fn run_in_transaction() -> bool
trait Model
  fn table_meta() -> &'static TableMeta<Self>
  fn audit_enabled() -> bool
  fn audit_excluded_fields() -> &'static [&'static str]
trait ModelCollectionExt: Model>
  fn model_keys(&self, key_fn: impl Fn(&T) -> DbValue) -> Collection<DbValue>
  fn load<'life0, 'async_trait, E>(
  fn load_missing<'life0, 'async_trait, E>(
trait ModelInstanceWriteExt: PersistedModel
  fn update(&self) -> UpdateModel<Self>
  fn delete(&self) -> DeleteModel<Self>
  fn force_delete(&self) -> DeleteModel<Self>
  fn restore(&self) -> RestoreModel<Self>
trait ModelLifecycle
  fn creating<'life0, 'life1, 'life2, 'async_trait>(
  fn created<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn updating<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn updated<'life0, 'life1, 'life2, 'life3, 'life4, 'life5, 'async_trait>(
  fn deleting<'life0, 'life1, 'life2, 'life3, 'async_trait>(
  fn deleted<'life0, 'life1, 'life2, 'life3, 'async_trait>(
trait ModelWriteExecutor: AfterCommitSink
  fn app_context(&self) -> &AppContext
  fn active_transaction(&self) -> Option<&DatabaseTransaction>
  fn actor(&self) -> Option<&Actor>
trait PersistedModel: Model
  fn persisted_condition(&self) -> Condition
trait Projection
  fn projection_meta() -> &'static ProjectionMeta<Self>
  fn from_record(record: &DbRecord) -> Result<Self>
  fn source(source: impl Into<FromItem>) -> ProjectionQuery<Self>
trait QueryExecutor
  fn raw_query_with<'life0, 'life1, 'life2, 'async_trait>(
  fn raw_execute_with<'life0, 'life1, 'life2, 'async_trait>(
  fn stream_records<'a>(
  fn raw_query<'life0, 'life1, 'life2, 'async_trait>(
  fn raw_execute<'life0, 'life1, 'life2, 'async_trait>(
  fn query_records_with<'life0, 'life1, 'async_trait>(
  fn query_records<'life0, 'life1, 'async_trait>(
  fn execute_compiled_with<'life0, 'life1, 'async_trait>(
  fn execute_compiled<'life0, 'life1, 'async_trait>(
trait SeederFile
  fn run<'life0, 'life1, 'async_trait>(
  fn run_in_transaction() -> bool
trait ToDbValue
  fn to_db_value(self) -> DbValue
  fn db_type() -> DbType
async fn scope_model_extensions<F, T>(future: F) -> T
```

## foundry::database::ast

```rust
enum AggregateFn { Count, Sum, Avg, Min, Max }
enum BinaryOperator { Add, Subtract, Multiply, Divide, Concat, Custom }
enum ComparisonOp { Eq, IEq, NotEq, Gt, Gte, Lt, Lte, Like, NotLike, ILike }
enum Condition { Comparison, InList, JsonPredicate, FullText, And, Or, Not, IsNull, IsNotNull, Exists, Raw }
  fn compare(left: Expr, op: ComparisonOp, right: Expr) -> Self
  fn json(expr: Expr, op: JsonPredicateOp, value: JsonPredicateValue) -> Self
  fn full_text( columns: impl IntoIterator<Item = ColumnRef>, query: impl Into<String>, ) -> Self
  fn and(conditions: impl IntoIterator<Item = Condition>) -> Self
  fn or(conditions: impl IntoIterator<Item = Condition>) -> Self
  fn negate(condition: Condition) -> Self
  fn exists(query: impl Into<QueryAst>) -> Self
  fn is_null(column: impl Into<ColumnRef>) -> Self
  fn is_not_null(column: impl Into<ColumnRef>) -> Self
  fn expr_is_null(expr: impl Into<Expr>) -> Self
  fn expr_is_not_null(expr: impl Into<Expr>) -> Self
  fn false_() -> Self
  fn true_() -> Self
  fn raw(sql: impl Into<String>, bindings: Vec<DbValue>) -> Self
enum CteMaterialization { Materialized, NotMaterialized }
enum DbType { Show 30 variants    Int16, Int32, Int64, Bool, Float32, ... +25 more }
  fn postgres_cast(self) -> &'static str
  fn array_element(self) -> Option<Self>
enum DbValue { Show 31 variants    Null, Int16, Int32, Int64, Bool, ... +26 more }
  fn null(db_type: DbType) -> Self
  fn db_type(&self) -> DbType
  fn relation_key(&self) -> String
enum Expr { Show 13 variants    Column, Excluded, Value, Cast, Aggregate, Function, Unary, Binary, Subquery, Window, Case, JsonPath, Raw }
  fn column(column: impl Into<ColumnRef>) -> Self
  fn excluded(column: impl Into<ColumnRef>) -> Self
  fn value(value: impl Into<DbValue>) -> Self
  fn text(value: impl Into<String>) -> Self
  fn bool(value: bool) -> Self
  fn false_() -> Self
  fn true_() -> Self
  fn cast(expr: impl Into<Expr>, db_type: DbType) -> Self
  fn cast_text(expr: impl Into<Expr>) -> Self
  fn function( name: impl Into<String>, args: impl IntoIterator<Item = Expr>, ) -> Self
  fn unary(operator: UnaryOperator, expr: impl Into<Expr>) -> Self
  fn binary( left: impl Into<Expr>, operator: BinaryOperator, right: impl Into<Expr>, ) -> Self
  fn subquery(query: impl Into<QueryAst>) -> Self
  fn window(function: impl Into<Expr>, window: WindowSpec) -> Self
  fn raw(sql: impl Into<String>) -> Self
  fn json(self) -> JsonExprBuilder
  fn compare(self, op: ComparisonOp, right: impl Into<Expr>) -> Condition
  fn compare_value( self, op: ComparisonOp, value: impl Into<DbValue>, ) -> Condition
  fn eq_value(self, value: impl Into<DbValue>) -> Condition
  fn not_eq_value(self, value: impl Into<DbValue>) -> Condition
  fn gt_value(self, value: impl Into<DbValue>) -> Condition
  fn gte_value(self, value: impl Into<DbValue>) -> Condition
  fn lt_value(self, value: impl Into<DbValue>) -> Condition
  fn lte_value(self, value: impl Into<DbValue>) -> Condition
  fn is_null(self) -> Condition
  fn is_not_null(self) -> Condition
  fn like(self, value: impl Into<String>) -> Condition
  fn not_like(self, value: impl Into<String>) -> Condition
  fn ilike(self, value: impl Into<String>) -> Condition
enum FromItem { Table, Subquery }
  fn subquery(query: impl Into<QueryAst>, alias: impl Into<String>) -> Self
enum InsertSource { Values, Select }
enum JoinKind { Inner, Left, Right, Full, Cross }
enum JsonPathMode { Json, Text }
enum JsonPathSegment { Key, Index }
enum JsonPredicateOp { Contains, ContainedBy, HasKey, HasAnyKeys, HasAllKeys }
enum JsonPredicateValue { Json, Key, Keys }
enum LockBehavior { Wait, NoWait, SkipLocked }
enum LockStrength { Update, NoKeyUpdate, Share, KeyShare }
enum OnConflictAction { DoNothing, DoUpdate }
enum OnConflictTarget { Columns, Constraint }
enum OrderDirection { Asc, Desc }
enum QueryBody { Select, Insert, Update, Delete, SetOperation }
enum RelationKind { HasMany, HasOne, BelongsTo, ManyToMany }
enum SetOperator { Union, UnionAll }
enum UnaryOperator { Not, Negate }
enum WindowFrameBound { UnboundedPreceding, Preceding, CurrentRow, Following, UnboundedFollowing }
enum WindowFrameUnits { Rows, Range }
struct AggregateExpr
  fn count_all() -> Self
  fn count(expr: Expr) -> Self
  fn count_distinct(expr: Expr) -> Self
  fn sum(expr: Expr) -> Self
  fn avg(expr: Expr) -> Self
  fn min(expr: Expr) -> Self
  fn max(expr: Expr) -> Self
struct AggregateNode
  fn count_all(alias: impl Into<String>) -> Self
  fn count(expr: Expr, alias: impl Into<String>) -> Self
  fn count_distinct(expr: Expr, alias: impl Into<String>) -> Self
  fn sum(expr: Expr, alias: impl Into<String>) -> Self
  fn avg(expr: Expr, alias: impl Into<String>) -> Self
  fn min(expr: Expr, alias: impl Into<String>) -> Self
  fn max(expr: Expr, alias: impl Into<String>) -> Self
struct BinaryExpr
struct CaseExpr
struct CaseWhen
struct ColumnRef
  fn new(table: impl Into<String>, name: impl Into<String>) -> Self
  fn bare(name: impl Into<String>) -> Self
  fn typed(self, db_type: DbType) -> Self
  fn aliased(self, alias: impl Into<String>) -> Self
struct CteNode
struct DeleteNode
struct FunctionCall
struct InsertNode
struct JoinNode
struct JsonPathExpr
struct LockClause
struct Numeric
  fn new(value: impl Into<String>) -> Result<Self>
  fn as_str(&self) -> &str
struct OnConflictNode
struct OnConflictUpdate
struct OrderBy
  fn asc(expr: impl Into<Expr>) -> Self
  fn desc(expr: impl Into<Expr>) -> Self
struct PivotNode
struct QueryAst
  fn select(select: SelectNode) -> Self
  fn insert(insert: InsertNode) -> Self
  fn update(update: UpdateNode) -> Self
  fn delete(delete: DeleteNode) -> Self
  fn set_operation(set: SetOperationNode) -> Self
struct RelationNode
struct SelectItem
  fn new(expr: impl Into<Expr>) -> Self
  fn aliased(self, alias: impl Into<String>) -> Self
struct SelectNode
  fn from(source: impl Into<FromItem>) -> Self
  fn select(self, expr: impl Into<Expr>) -> Self
  fn select_as(self, expr: impl Into<Expr>, alias: impl Into<String>) -> Self
  fn where_(self, condition: Condition) -> Self
  fn limit(self, limit: u64) -> Self
  fn order_by(self, order: OrderBy) -> Self
struct SetOperationNode
struct TableRef
  fn new(name: impl Into<String>) -> Self
  fn aliased(self, alias: impl Into<String>) -> Self
struct UnaryExpr
struct UpdateNode
struct WindowExpr
struct WindowFrame
struct WindowSpec
```

## foundry::database::compiler

```rust
struct CompiledSql
struct PostgresCompiler
  fn compile(ast: &QueryAst) -> Result<CompiledSql>
```

## foundry::database::relation

```rust
pub type AnyRelation<M> = Arc<dyn RelationLoader<M>>;
struct ManyToManyDef
  fn named(self, name: impl Into<String>) -> Self
  fn with<Child>(self, child: RelationDef<To, Child>) -> Self
  fn with_many_to_many<Child, ChildPivot>( self, child: ManyToManyDef<To, Child, ChildPivot>, ) -> Self
  fn with_attachments(self, collection: impl Into<String>) -> Self
  fn with_translated_field(self, field: impl Into<String>) -> Self
  fn with_translations_for(self, locale: impl Into<String>) -> Self
  fn with_all_translations(self) -> Self
  fn with_aggregate<Value>( self, aggregate: RelationAggregateDef<To, Value>, ) -> Self
  fn where_(self, condition: Condition) -> Self
  fn is_loaded( self, f: impl Fn(&From) -> bool + Send + Sync + 'static, ) -> Self
  fn with_pivot<NewPivot>( self, meta: &'static ProjectionMeta<NewPivot>, attach: fn(&mut To, NewPivot), ) -> ManyToManyDef<From, To, NewPivot>
  fn count( self, attach: fn(&mut From, i64), ) -> RelationAggregateDef<From, i64>
  fn count_distinct<Value>( self, column: Column<To, Value>, attach: fn(&mut From, i64), ) -> RelationAggregateDef<From, i64>
  fn sum<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn avg<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn min<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn max<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn node(&self) -> RelationNode
struct RelationAggregateDef
struct RelationDef
  fn named(self, name: impl Into<String>) -> Self
  fn with<Child>(self, child: RelationDef<To, Child>) -> Self
  fn with_many_to_many<Child, Pivot>( self, child: ManyToManyDef<To, Child, Pivot>, ) -> Self
  fn with_attachments(self, collection: impl Into<String>) -> Self
  fn with_translated_field(self, field: impl Into<String>) -> Self
  fn with_translations_for(self, locale: impl Into<String>) -> Self
  fn with_all_translations(self) -> Self
  fn with_aggregate<Value>( self, aggregate: RelationAggregateDef<To, Value>, ) -> Self
  fn where_(self, condition: Condition) -> Self
  fn is_loaded( self, f: impl Fn(&From) -> bool + Send + Sync + 'static, ) -> Self
  fn count( self, attach: fn(&mut From, i64), ) -> RelationAggregateDef<From, i64>
  fn count_distinct<Value>( self, column: Column<To, Value>, attach: fn(&mut From, i64), ) -> RelationAggregateDef<From, i64>
  fn sum<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn avg<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn min<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn max<Value>( self, column: Column<To, Value>, attach: fn(&mut From, Option<Value>), ) -> RelationAggregateDef<From, Option<Value>>
  fn node(&self) -> RelationNode
trait RelationLoader: Send>
  fn node(&self) -> RelationNode
  fn load<'life0, 'life1, 'life2, 'async_trait>(
  fn load_missing<'life0, 'life1, 'life2, 'async_trait>(
fn belongs_to<From, To, Key>( foreign_key: Column<From, Key>, owner_key: Column<To, Key>, parent_key: fn(&From) -> Option<Key>, attach: fn(&mut From, Option<To>), ) -> RelationDef<From, To>
fn has_many<From, To, Key>( local_key: Column<From, Key>, foreign_key: Column<To, Key>, parent_key: fn(&From) -> Key, attach: fn(&mut From, Vec<To>), ) -> RelationDef<From, To>
fn has_one<From, To, Key>( local_key: Column<From, Key>, foreign_key: Column<To, Key>, parent_key: fn(&From) -> Key, attach: fn(&mut From, Option<To>), ) -> RelationDef<From, To>
fn many_to_many<From, To, Pivot, LocalKey, TargetKey>( local_key: Column<From, LocalKey>, pivot_table: &'static str, pivot_local_key: &'static str, pivot_related_key: &'static str, target_key: Column<To, TargetKey>, parent_key: fn(&From) -> LocalKey, attach: fn(&mut From, Vec<To>), ) -> ManyToManyDef<From, To, Pivot>
```
