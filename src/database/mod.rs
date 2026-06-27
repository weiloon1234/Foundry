mod aggregate;
pub mod ast;
mod collection_ext;
pub mod compiler;
pub(crate) mod extensions;
pub(crate) mod lifecycle;
mod model;
mod projection;
mod query;
pub mod relation;
mod runtime;
mod scaffold;

pub use aggregate::AggregateProjection;
pub use ast::{
    AggregateExpr, AggregateFn, AggregateNode, BinaryExpr, BinaryOperator, CaseExpr, CaseWhen,
    ColumnRef, ComparisonOp, Condition, CteMaterialization, CteNode, DbType, DbValue, DeleteNode,
    Expr, FromItem, FunctionCall, InsertNode, InsertSource, JoinKind, JoinNode, JsonPathExpr,
    JsonPathMode, JsonPathSegment, JsonPredicateOp, JsonPredicateValue, LockBehavior, LockClause,
    LockStrength, Numeric, OnConflictAction, OnConflictNode, OnConflictTarget, OrderBy,
    OrderDirection, QueryAst, QueryBody, RelationKind, RelationNode, SelectItem, SelectNode,
    SetOperationNode, SetOperator, TableRef, UnaryExpr, UnaryOperator, UpdateNode, WindowExpr,
    WindowFrame, WindowFrameBound, WindowFrameUnits, WindowSpec,
};
pub use collection_ext::{IntoLoadableRelation, ModelCollectionExt};
pub use compiler::{CompiledSql, PostgresCompiler};
pub use extensions::scope_model_extensions;
pub use lifecycle::{MigrationContext, MigrationFile, SeederContext, SeederFile};
pub use model::{
    AfterCommitCallback, AfterCommitSink, Column, ColumnInfo, CreateDraft, FromDbValue,
    IntoColumnValue, IntoFieldValue, Loaded, Model, ModelBehavior, ModelCreatedEvent,
    ModelCreatingEvent, ModelDeletedEvent, ModelDeletingEvent, ModelFeatureSetting,
    ModelHookContext, ModelInstanceWriteExt, ModelLifecycle, ModelLifecycleSnapshot,
    ModelPrimaryKeyStrategy, ModelUpdatedEvent, ModelUpdatingEvent, ModelWriteExecutor,
    NoModelLifecycle, PersistedModel, TableMeta, ToDbValue, UpdateDraft,
};
pub use projection::{Projection, ProjectionField, ProjectionFieldInfo, ProjectionMeta};
pub use query::{
    Case, CreateManyModel, CreateModel, CreateRow, Cte, CursorInfo, CursorMeta, CursorPaginated,
    CursorPagination, DeleteModel, JsonExprBuilder, ModelQuery, Paginated, PaginatedResponse,
    Pagination, PaginationLinks, PaginationMeta, ProjectionQuery, Query, RestoreModel, Sql,
    UpdateModel, Window, WindowBuilder,
};
pub use relation::{
    belongs_to, has_many, has_one, many_to_many, AnyRelation, ManyToManyDef, RelationAggregateDef,
    RelationDef, RelationLoader,
};
pub use runtime::{
    DatabaseManager, DatabaseTransaction, DbRecord, DbRecordStream, NPlusOneSuspect,
    QueryExecutionOptions, QueryExecutor, SlowQueryEntry, SqlObservabilitySnapshot,
    SqlObservabilityStats,
};

pub(crate) use lifecycle::{
    builtin_cli_registrar, MigrationRegistryBuilder, MigrationRegistryHandle,
    SeederRegistryBuilder, SeederRegistryHandle,
};
pub(crate) use model::set_runtime_model_defaults;
pub(crate) use runtime::{scope_http_sql_query_trace, sql_observability_snapshot};
pub(crate) use scaffold::scaffold_cli_registrar;
