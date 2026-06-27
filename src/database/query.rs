use std::{collections::VecDeque, future::Future, marker::PhantomData, pin::Pin};

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::audit::{record_with_assignments, write_model_audit, AuditEventType};
use crate::config::DatabaseConfig;
use crate::events::{Event, EventOrigin};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{
    catch_async_panic, catch_sync_panic, current_actor, current_request, panic_payload_message,
};
use crate::support::{Collection, ModelId};
use futures_util::stream::{self, BoxStream, StreamExt};

use super::aggregate::{
    count_query_ast, execute_scalar_projection_on_ast, wrap_query_for_alias_aggregate,
    AggregateProjection,
};
use super::ast::{
    AggregateExpr, BinaryOperator, CaseExpr, CaseWhen, ColumnRef, ComparisonOp, Condition, CteNode,
    DbType, DbValue, Expr, FromItem, InsertNode, InsertSource, JoinKind, JoinNode, JsonPathExpr,
    JsonPathMode, JsonPathSegment, JsonPredicateOp, JsonPredicateValue, LockBehavior, LockClause,
    LockStrength, OnConflictAction, OnConflictNode, OnConflictTarget, OnConflictUpdate, OrderBy,
    QueryAst, QueryBody, SelectItem, SelectNode, SetOperationNode, SetOperator, UpdateNode,
    WindowFrame, WindowFrameBound, WindowFrameUnits, WindowSpec,
};
use super::compiler::PostgresCompiler;
use super::extensions::{register_model_records, AnyModelExtension};
use super::model::{
    upsert_assignment, AfterCommitSink, Column, CreateDraft, FromDbValue, IntoColumnValue,
    IntoFieldValue, Model, ModelCreatedEvent, ModelCreatingEvent, ModelDeletedEvent,
    ModelDeletingEvent, ModelFieldWriteMutator, ModelHookContext, ModelLifecycle,
    ModelLifecycleSnapshot, ModelPrimaryKeyStrategy, ModelUpdatedEvent, ModelUpdatingEvent,
    ModelWriteExecutor, TableMeta, ToDbValue, UpdateDraft,
};
use super::projection::{Projection, ProjectionField, ProjectionMeta};
use super::relation::{
    AnyRelation, AnyRelationAggregate, ManyToManyDef, RelationAggregateDef, RelationDef,
};
use super::runtime::{DbRecord, DbRecordStream, QueryExecutionOptions, QueryExecutor};

#[derive(Clone, Copy, Debug, PartialEq, Eq, foundry_macros::TS)]
pub struct Pagination {
    pub page: u64,
    pub per_page: u64,
}

#[derive(Default, Deserialize)]
struct PaginationFields {
    #[serde(default)]
    page: Option<u64>,
    #[serde(default)]
    per_page: Option<u64>,
}

impl Pagination {
    pub const DEFAULT_PAGE: u64 = 1;
    pub const DEFAULT_PER_PAGE: u64 = 15;

    pub fn new(page: u64, per_page: u64) -> Self {
        Self {
            page: page.max(1),
            per_page: per_page.max(1),
        }
    }

    pub fn from_config(config: &DatabaseConfig, page: Option<u64>, per_page: Option<u64>) -> Self {
        Self::from_options_with_default_per_page(page, per_page, config.default_per_page)
    }

    pub fn from_options_with_default_per_page(
        page: Option<u64>,
        per_page: Option<u64>,
        default_per_page: u64,
    ) -> Self {
        Self::new(
            page.unwrap_or(Self::DEFAULT_PAGE),
            per_page.unwrap_or(default_per_page),
        )
    }

    pub fn offset(&self) -> u64 {
        (self.page - 1) * self.per_page
    }
}

impl Default for Pagination {
    fn default() -> Self {
        Self::new(Self::DEFAULT_PAGE, Self::DEFAULT_PER_PAGE)
    }
}

impl<'de> Deserialize<'de> for Pagination {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let fields = PaginationFields::deserialize(deserializer)?;
        Ok(Self::from_options_with_default_per_page(
            fields.page,
            fields.per_page,
            Self::DEFAULT_PER_PAGE,
        ))
    }
}

impl<S> FromRequestParts<S> for Pagination
where
    S: Send + Sync,
    AppContext: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let app = AppContext::from_ref(state);
        let database = app
            .config()
            .database()
            .map_err(|error| error.into_response())?;

        pagination_from_query(parts.uri.query(), database.default_per_page)
            .map_err(|error| error.into_response())
    }
}

fn pagination_from_query(query: Option<&str>, default_per_page: u64) -> Result<Pagination> {
    let mut fields = PaginationFields::default();
    let Some(query) = query else {
        return Ok(Pagination::from_options_with_default_per_page(
            fields.page,
            fields.per_page,
            default_per_page,
        ));
    };

    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "page" => fields.page = Some(parse_pagination_query_value("page", &value)?),
            "per_page" => {
                fields.per_page = Some(parse_pagination_query_value("per_page", &value)?);
            }
            _ => {}
        }
    }

    Ok(Pagination::from_options_with_default_per_page(
        fields.page,
        fields.per_page,
        default_per_page,
    ))
}

fn parse_pagination_query_value(field: &'static str, value: &str) -> Result<u64> {
    value.parse::<u64>().map_err(|_| {
        Error::http_with_code(
            400,
            format!(
                "invalid pagination query parameter `{field}`: expected unsigned integer, got `{value}`"
            ),
            "invalid_pagination",
        )
    })
}

impl ts_rs::TS for Pagination {
    type WithoutGenerics = Self;

    fn ident() -> String {
        "Pagination".to_string()
    }

    fn name() -> String {
        "Pagination".to_string()
    }

    fn decl() -> String {
        "type Pagination = { page?: number, per_page?: number, };".to_string()
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn inline() -> String {
        "{ page?: number, per_page?: number }".to_string()
    }

    fn inline_flattened() -> String {
        Self::inline()
    }

    fn output_path() -> Option<&'static std::path::Path> {
        Some(std::path::Path::new("Pagination.ts"))
    }
}

impl crate::openapi::ApiSchema for Pagination {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "page": { "type": "integer" },
                "per_page": { "type": "integer" }
            }
        })
    }

    fn schema_name() -> &'static str {
        "Pagination"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paginated<T> {
    pub data: Collection<T>,
    pub pagination: Pagination,
    pub total: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub data: Vec<T>,
    pub meta: PaginationMeta,
    pub links: PaginationLinks,
}

#[derive(
    Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct PaginationMeta {
    #[ts(type = "number")]
    pub current_page: u64,
    #[ts(type = "number")]
    pub per_page: u64,
    #[ts(type = "number")]
    pub total: u64,
    #[ts(type = "number")]
    pub last_page: u64,
}

#[derive(
    Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct PaginationLinks {
    pub next: Option<String>,
    pub prev: Option<String>,
}

impl<T> crate::openapi::ApiSchema for PaginatedResponse<T>
where
    T: Serialize + crate::openapi::ApiSchema,
{
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "x-foundry-wrapper-schema": "PaginatedResponse",
            "x-foundry-data-schema": T::schema_name(),
            "properties": {
                "data": {
                    "type": "array",
                    "items": T::schema(),
                    "x-foundry-item-schema": T::schema_name(),
                },
                "meta": <PaginationMeta as crate::openapi::ApiSchema>::schema(),
                "links": <PaginationLinks as crate::openapi::ApiSchema>::schema(),
            },
            "required": ["data", "meta", "links"],
        })
    }

    fn schema_name() -> &'static str {
        "PaginatedResponse"
    }
}

impl<T> Paginated<T> {
    pub fn meta(&self) -> PaginationMeta {
        let per_page = self.pagination.per_page;
        let current_page = self.pagination.page;
        let last_page = if self.total == 0 {
            1
        } else {
            self.total.div_ceil(per_page)
        };

        PaginationMeta {
            current_page,
            per_page,
            total: self.total,
            last_page,
        }
    }

    pub fn links(&self, base_url: &str) -> PaginationLinks {
        let (_, links) = self.response_meta_and_links(base_url);
        links
    }

    pub fn map_response<R: Serialize>(
        &self,
        base_url: &str,
        map_item: impl FnMut(&T) -> R,
    ) -> PaginatedResponse<R> {
        let (meta, links) = self.response_meta_and_links(base_url);
        let data = self.data.iter().map(map_item).collect();

        PaginatedResponse { data, meta, links }
    }

    fn response_meta_and_links(&self, base_url: &str) -> (PaginationMeta, PaginationLinks) {
        let meta = self.meta();
        let links = pagination_links_from_meta(&meta, base_url);

        (meta, links)
    }
}

impl<T: Serialize> Paginated<T> {
    pub fn to_response(&self, base_url: &str) -> PaginatedResponse<&T> {
        let (meta, links) = self.response_meta_and_links(base_url);
        let data = self.data.iter().collect();

        PaginatedResponse { data, meta, links }
    }
}

fn pagination_links_from_meta(meta: &PaginationMeta, base_url: &str) -> PaginationLinks {
    let next = if meta.current_page < meta.last_page {
        Some(pagination_link(
            base_url,
            meta.current_page + 1,
            meta.per_page,
        ))
    } else {
        None
    };

    let prev = if meta.current_page > 1 {
        Some(pagination_link(
            base_url,
            meta.current_page - 1,
            meta.per_page,
        ))
    } else {
        None
    };

    PaginationLinks { next, prev }
}

fn pagination_link(base_url: &str, page: u64, per_page: u64) -> String {
    let (base_without_fragment, fragment) = base_url
        .split_once('#')
        .map_or((base_url, None), |(base, fragment)| (base, Some(fragment)));
    let (path, query) = base_without_fragment
        .split_once('?')
        .map_or((base_without_fragment, ""), |(path, query)| (path, query));
    let mut params = query
        .split('&')
        .filter(|part| {
            if part.is_empty() {
                return false;
            }
            let key = part.split_once('=').map_or(*part, |(key, _)| key);
            key != "page" && key != "per_page"
        })
        .map(str::to_string)
        .collect::<Vec<_>>();

    params.push(format!("page={page}"));
    params.push(format!("per_page={per_page}"));

    let query = params.join("&");
    match fragment {
        Some(fragment) => format!("{path}?{query}#{fragment}"),
        None => format!("{path}?{query}"),
    }
}

// --- Cursor-based pagination ---

#[derive(Clone, Debug)]
pub struct CursorPagination {
    pub after: Option<String>,
    pub before: Option<String>,
    pub per_page: u64,
}

impl CursorPagination {
    pub fn new(per_page: u64) -> Self {
        Self {
            after: None,
            before: None,
            per_page: per_page.max(1),
        }
    }

    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    pub fn before(mut self, cursor: impl Into<String>) -> Self {
        self.before = Some(cursor.into());
        self
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CursorPaginated<T: Serialize> {
    pub data: Vec<T>,
    pub meta: CursorMeta,
    pub cursors: CursorInfo,
}

#[derive(
    Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct CursorMeta {
    pub has_next: bool,
    pub has_prev: bool,
    #[ts(type = "number")]
    pub per_page: u64,
}

#[derive(
    Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct CursorInfo {
    pub next: Option<String>,
    pub prev: Option<String>,
}

impl<T> crate::openapi::ApiSchema for CursorPaginated<T>
where
    T: Serialize + crate::openapi::ApiSchema,
{
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "x-foundry-wrapper-schema": "CursorPaginated",
            "x-foundry-data-schema": T::schema_name(),
            "properties": {
                "data": {
                    "type": "array",
                    "items": T::schema(),
                    "x-foundry-item-schema": T::schema_name(),
                },
                "meta": <CursorMeta as crate::openapi::ApiSchema>::schema(),
                "cursors": <CursorInfo as crate::openapi::ApiSchema>::schema(),
            },
            "required": ["data", "meta", "cursors"],
        })
    }

    fn schema_name() -> &'static str {
        "CursorPaginated"
    }
}

impl<T: Serialize> CursorPaginated<T> {
    /// Encode a value as a cursor string (base64url).
    pub fn encode_cursor(value: &impl std::fmt::Display) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        URL_SAFE_NO_PAD.encode(value.to_string().as_bytes())
    }

    /// Set cursor values from the first and last items in the result.
    pub fn with_cursors(
        mut self,
        first_value: Option<&impl std::fmt::Display>,
        last_value: Option<&impl std::fmt::Display>,
    ) -> Self {
        if self.meta.has_prev {
            self.cursors.prev = first_value.map(|v| Self::encode_cursor(v));
        }
        if self.meta.has_next {
            self.cursors.next = last_value.map(|v| Self::encode_cursor(v));
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SoftDeleteScope {
    ActiveOnly,
    WithTrashed,
    OnlyTrashed,
}

pub struct Case;
pub struct Sql;
pub struct Window;

#[derive(Clone, Debug, Default)]
pub struct CaseBuilder {
    expr: CaseExpr,
}

impl Case {
    pub fn when(condition: Condition, result: impl Into<Expr>) -> CaseBuilder {
        CaseBuilder::default().when(condition, result)
    }
}

impl Sql {
    pub fn count_all() -> Expr {
        Expr::from(AggregateExpr::count_all())
    }

    pub fn count(expr: impl Into<Expr>) -> Expr {
        Expr::from(AggregateExpr::count(expr.into()))
    }

    pub fn count_distinct(expr: impl Into<Expr>) -> Expr {
        Expr::from(AggregateExpr::count_distinct(expr.into()))
    }

    pub fn count_when(condition: Condition) -> Expr {
        Self::count(Case::when(condition, Expr::value(1_i64)).end())
    }

    pub fn sum(expr: impl Into<Expr>) -> Expr {
        Expr::from(AggregateExpr::sum(expr.into()))
    }

    pub fn avg(expr: impl Into<Expr>) -> Expr {
        Expr::from(AggregateExpr::avg(expr.into()))
    }

    pub fn min(expr: impl Into<Expr>) -> Expr {
        Expr::from(AggregateExpr::min(expr.into()))
    }

    pub fn max(expr: impl Into<Expr>) -> Expr {
        Expr::from(AggregateExpr::max(expr.into()))
    }

    pub fn function(name: impl Into<String>, args: impl IntoIterator<Item = Expr>) -> Expr {
        Expr::function(name, args)
    }

    pub fn coalesce(args: impl IntoIterator<Item = Expr>) -> Expr {
        Expr::function("COALESCE", args)
    }

    pub fn concat_ws(separator: impl Into<String>, args: impl IntoIterator<Item = Expr>) -> Expr {
        let mut args = args.into_iter().collect::<Vec<_>>();
        args.insert(0, Expr::text(separator));
        Expr::function("CONCAT_WS", args)
    }

    pub fn lower(expr: impl Into<Expr>) -> Expr {
        Expr::function("LOWER", [expr.into()])
    }

    pub fn upper(expr: impl Into<Expr>) -> Expr {
        Expr::function("UPPER", [expr.into()])
    }

    pub fn date_trunc(granularity: impl Into<String>, expr: impl Into<Expr>) -> Expr {
        Expr::function("DATE_TRUNC", [Expr::value(granularity.into()), expr.into()])
    }

    /// `EXTRACT(<field> FROM expr)`. The field is written into the SQL text
    /// (it is a keyword, not a bindable value), so the compiler validates it
    /// against the known Postgres date/time fields and rejects anything else.
    pub fn extract(field: impl Into<String>, expr: impl Into<Expr>) -> Expr {
        Expr::function("EXTRACT", [Expr::value(field.into()), expr.into()])
    }

    pub fn json_text_or_first(expr: impl Into<Expr>, preferred_key: impl Into<String>) -> Expr {
        Expr::function(
            "JSONB_TEXT_OR_FIRST",
            [expr.into(), Expr::value(preferred_key.into())],
        )
    }

    pub fn to_timestamp_millis(millis: impl Into<Expr>) -> Expr {
        Expr::function(
            "TO_TIMESTAMP",
            [Self::divide(
                Expr::cast(millis, DbType::Float64),
                Expr::value(DbValue::Float64(1000.0)),
            )],
        )
    }

    pub fn now() -> Expr {
        Expr::function("NOW", std::iter::empty())
    }

    pub fn uuid_v7() -> Expr {
        Expr::function("uuidv7", std::iter::empty())
    }

    pub fn not(expr: impl Into<Expr>) -> Expr {
        Expr::unary(super::ast::UnaryOperator::Not, expr)
    }

    pub fn negate(expr: impl Into<Expr>) -> Expr {
        Expr::unary(super::ast::UnaryOperator::Negate, expr)
    }

    pub fn add(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr {
        Expr::binary(left, BinaryOperator::Add, right)
    }

    pub fn subtract(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr {
        Expr::binary(left, BinaryOperator::Subtract, right)
    }

    pub fn multiply(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr {
        Expr::binary(left, BinaryOperator::Multiply, right)
    }

    pub fn divide(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr {
        Expr::binary(left, BinaryOperator::Divide, right)
    }

    pub fn concat(left: impl Into<Expr>, right: impl Into<Expr>) -> Expr {
        Expr::binary(left, BinaryOperator::Concat, right)
    }

    /// Binary expression with a custom SQL operator (e.g. `->>`, `@>`,
    /// `ILIKE`). The operator is written into the SQL text, so the compiler
    /// restricts it to legal operator characters and keyword operators; it
    /// must never be built from untrusted input.
    pub fn op(left: impl Into<Expr>, operator: impl Into<String>, right: impl Into<Expr>) -> Expr {
        Expr::binary(left, BinaryOperator::Custom(operator.into()), right)
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowBuilder {
    spec: WindowSpec,
}

impl Window {
    pub fn partition_by(expr: impl Into<Expr>) -> WindowBuilder {
        WindowBuilder::default().partition_by(expr)
    }

    pub fn order_by(order: OrderBy) -> WindowBuilder {
        WindowBuilder::default().order_by(order)
    }

    pub fn over(function: impl Into<Expr>, builder: WindowBuilder) -> Expr {
        Expr::window(function, builder.finish())
    }
}

impl WindowBuilder {
    pub fn partition_by(mut self, expr: impl Into<Expr>) -> Self {
        self.spec.partition_by.push(expr.into());
        self
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.spec.order_by.push(order);
        self
    }

    pub fn rows_between(mut self, start: WindowFrameBound, end: WindowFrameBound) -> Self {
        self.spec.frame = Some(WindowFrame {
            units: WindowFrameUnits::Rows,
            start,
            end: Some(end),
        });
        self
    }

    pub fn range_between(mut self, start: WindowFrameBound, end: WindowFrameBound) -> Self {
        self.spec.frame = Some(WindowFrame {
            units: WindowFrameUnits::Range,
            start,
            end: Some(end),
        });
        self
    }

    pub fn finish(self) -> WindowSpec {
        self.spec
    }
}

impl CaseBuilder {
    pub fn when(mut self, condition: Condition, result: impl Into<Expr>) -> Self {
        self.expr.whens.push(CaseWhen {
            condition,
            result: Box::new(result.into()),
        });
        self
    }

    pub fn else_(mut self, result: impl Into<Expr>) -> Expr {
        self.expr.else_expr = Some(Box::new(result.into()));
        Expr::from(self.expr)
    }

    pub fn end(self) -> Expr {
        Expr::from(self.expr)
    }
}

#[derive(Clone, Debug)]
pub struct JsonExprBuilder {
    expr: Expr,
    path: Vec<JsonPathSegment>,
}

impl JsonExprBuilder {
    pub fn new(expr: impl Into<Expr>) -> Self {
        Self {
            expr: expr.into(),
            path: Vec::new(),
        }
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.path.push(JsonPathSegment::Key(key.into()));
        self
    }

    pub fn index(mut self, index: i64) -> Self {
        self.path.push(JsonPathSegment::Index(index));
        self
    }

    pub fn as_json(self) -> Expr {
        Expr::from(JsonPathExpr {
            expr: Box::new(self.expr),
            path: self.path,
            mode: JsonPathMode::Json,
        })
    }

    pub fn as_text(self) -> Expr {
        Expr::from(JsonPathExpr {
            expr: Box::new(self.expr),
            path: self.path,
            mode: JsonPathMode::Text,
        })
    }

    pub fn like(self, value: impl Into<String>) -> Condition {
        self.as_text().like(value)
    }

    pub fn not_like(self, value: impl Into<String>) -> Condition {
        self.as_text().not_like(value)
    }

    pub fn ilike(self, value: impl Into<String>) -> Condition {
        self.as_text().ilike(value)
    }

    pub fn contains(self, value: impl Into<serde_json::Value>) -> Condition {
        Condition::json(
            self.as_json(),
            JsonPredicateOp::Contains,
            JsonPredicateValue::Json(value.into()),
        )
    }

    pub fn contained_by(self, value: impl Into<serde_json::Value>) -> Condition {
        Condition::json(
            self.as_json(),
            JsonPredicateOp::ContainedBy,
            JsonPredicateValue::Json(value.into()),
        )
    }

    pub fn has_key(self, key: impl Into<String>) -> Condition {
        Condition::json(
            self.as_json(),
            JsonPredicateOp::HasKey,
            JsonPredicateValue::Key(key.into()),
        )
    }

    pub fn has_any_keys<I, S>(self, keys: I) -> Condition
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Condition::json(
            self.as_json(),
            JsonPredicateOp::HasAnyKeys,
            JsonPredicateValue::Keys(keys.into_iter().map(Into::into).collect()),
        )
    }

    pub fn has_all_keys<I, S>(self, keys: I) -> Condition
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Condition::json(
            self.as_json(),
            JsonPredicateOp::HasAllKeys,
            JsonPredicateValue::Keys(keys.into_iter().map(Into::into).collect()),
        )
    }
}

impl Expr {
    pub fn json(self) -> JsonExprBuilder {
        JsonExprBuilder::new(self)
    }

    pub fn compare(self, op: ComparisonOp, right: impl Into<Expr>) -> Condition {
        Condition::compare(self, op, right.into())
    }

    pub fn compare_value(self, op: ComparisonOp, value: impl Into<DbValue>) -> Condition {
        self.compare(op, Expr::value(value.into()))
    }

    pub fn eq_value(self, value: impl Into<DbValue>) -> Condition {
        self.compare_value(ComparisonOp::Eq, value)
    }

    pub fn not_eq_value(self, value: impl Into<DbValue>) -> Condition {
        self.compare_value(ComparisonOp::NotEq, value)
    }

    pub fn gt_value(self, value: impl Into<DbValue>) -> Condition {
        self.compare_value(ComparisonOp::Gt, value)
    }

    pub fn gte_value(self, value: impl Into<DbValue>) -> Condition {
        self.compare_value(ComparisonOp::Gte, value)
    }

    pub fn lt_value(self, value: impl Into<DbValue>) -> Condition {
        self.compare_value(ComparisonOp::Lt, value)
    }

    pub fn lte_value(self, value: impl Into<DbValue>) -> Condition {
        self.compare_value(ComparisonOp::Lte, value)
    }

    pub fn is_null(self) -> Condition {
        Condition::expr_is_null(self)
    }

    pub fn is_not_null(self) -> Condition {
        Condition::expr_is_not_null(self)
    }

    pub fn like(self, value: impl Into<String>) -> Condition {
        self.compare(ComparisonOp::Like, Expr::text(value))
    }

    pub fn not_like(self, value: impl Into<String>) -> Condition {
        self.compare(ComparisonOp::NotLike, Expr::text(value))
    }

    pub fn ilike(self, value: impl Into<String>) -> Condition {
        self.compare(ComparisonOp::ILike, Expr::text(value))
    }
}

#[derive(Clone, Debug)]
pub struct Cte {
    node: CteNode,
}

impl Cte {
    pub fn new(name: impl Into<String>, query: impl Into<QueryAst>) -> Self {
        Self {
            node: CteNode {
                name: name.into(),
                query: Box::new(query.into()),
                recursive: false,
                materialization: None,
            },
        }
    }

    pub fn materialized(mut self) -> Self {
        self.node.materialization = Some(super::ast::CteMaterialization::Materialized);
        self
    }

    pub fn not_materialized(mut self) -> Self {
        self.node.materialization = Some(super::ast::CteMaterialization::NotMaterialized);
        self
    }

    pub fn recursive(mut self) -> Self {
        self.node.recursive = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    ast: QueryAst,
    options: QueryExecutionOptions,
    deferred_error: Option<String>,
}

impl From<Query> for QueryAst {
    fn from(value: Query) -> Self {
        value.ast
    }
}

impl Query {
    pub fn table(source: impl Into<FromItem>) -> Self {
        Self {
            ast: QueryAst::select(SelectNode::from(source)),
            options: QueryExecutionOptions::default(),
            deferred_error: None,
        }
    }

    pub fn insert_into(table: impl Into<super::ast::TableRef>) -> Self {
        Self::insert_many_into(table)
    }

    pub fn insert_many_into(table: impl Into<super::ast::TableRef>) -> Self {
        Self {
            ast: QueryAst::insert(InsertNode {
                into: table.into(),
                source: InsertSource::Values(Vec::new()),
                on_conflict: None,
                returning: Vec::new(),
            }),
            options: QueryExecutionOptions::default(),
            deferred_error: None,
        }
    }

    pub fn insert_select_into(
        table: impl Into<super::ast::TableRef>,
        select: impl Into<QueryAst>,
    ) -> Self {
        Self {
            ast: QueryAst::insert(InsertNode {
                into: table.into(),
                source: InsertSource::Select(Box::new(select.into())),
                on_conflict: None,
                returning: Vec::new(),
            }),
            options: QueryExecutionOptions::default(),
            deferred_error: None,
        }
    }

    pub fn update_table(table: impl Into<super::ast::TableRef>) -> Self {
        Self {
            ast: QueryAst::update(UpdateNode {
                table: table.into(),
                values: Vec::new(),
                from: Vec::new(),
                condition: None,
                returning: Vec::new(),
            }),
            options: QueryExecutionOptions::default(),
            deferred_error: None,
        }
    }

    pub fn delete_from(table: impl Into<super::ast::TableRef>) -> Self {
        Self {
            ast: QueryAst::delete(super::ast::DeleteNode {
                from: table.into(),
                using: Vec::new(),
                condition: None,
                returning: Vec::new(),
            }),
            options: QueryExecutionOptions::default(),
            deferred_error: None,
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.options.label = Some(label.into());
        self
    }

    pub fn with_cte(mut self, cte: Cte) -> Self {
        self.ast.with.push(cte.node);
        self
    }

    pub fn distinct(mut self) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.distinct = true;
        }
        self
    }

    pub fn select<I, C>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<super::ast::ColumnRef>,
    {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.columns = columns
                .into_iter()
                .map(|column| SelectItem::new(Expr::column(column.into())))
                .collect();
        }
        self
    }

    pub fn select_item(mut self, item: SelectItem) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.columns.push(item);
        }
        self
    }

    pub fn select_expr(mut self, expr: impl Into<Expr>, alias: impl Into<String>) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select
                .columns
                .push(SelectItem::new(expr).aliased(alias.into()));
        }
        self
    }

    pub fn select_aggregate<T>(mut self, projection: AggregateProjection<T>) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.aggregates.push(projection.node());
        }
        self
    }

    pub fn join(mut self, kind: JoinKind, table: impl Into<FromItem>, on: Condition) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.joins.push(JoinNode {
                kind,
                table: table.into(),
                lateral: false,
                on: Some(on),
            });
        }
        self
    }

    pub fn join_lateral(
        mut self,
        kind: JoinKind,
        table: impl Into<FromItem>,
        on: Option<Condition>,
    ) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.joins.push(JoinNode {
                kind,
                table: table.into(),
                lateral: true,
                on,
            });
        }
        self
    }

    pub fn inner_join(self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.join(JoinKind::Inner, table, on)
    }

    pub fn left_join(self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.join(JoinKind::Left, table, on)
    }

    pub fn right_join(self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.join(JoinKind::Right, table, on)
    }

    pub fn full_outer_join(self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.join(JoinKind::Full, table, on)
    }

    pub fn cross_join(mut self, table: impl Into<FromItem>) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.joins.push(JoinNode {
                kind: JoinKind::Cross,
                table: table.into(),
                lateral: false,
                on: None,
            });
        }
        self
    }

    pub fn left_join_lateral(self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.join_lateral(JoinKind::Left, table, Some(on))
    }

    pub fn cross_join_lateral(self, table: impl Into<FromItem>) -> Self {
        self.join_lateral(JoinKind::Cross, table, None)
    }

    pub fn inner_join_lateral(self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.join_lateral(JoinKind::Inner, table, Some(on))
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        match &mut self.ast.body {
            QueryBody::Select(select) => {
                select.condition = merge_condition(select.condition.take(), condition);
            }
            QueryBody::Insert(insert) => {
                if let Some(OnConflictNode {
                    action: OnConflictAction::DoUpdate(conflict),
                    ..
                }) = &mut insert.on_conflict
                {
                    conflict.condition = merge_condition(conflict.condition.take(), condition);
                }
            }
            QueryBody::Update(update) => {
                update.condition = merge_condition(update.condition.take(), condition);
            }
            QueryBody::Delete(delete) => {
                delete.condition = merge_condition(delete.condition.take(), condition);
            }
            QueryBody::SetOperation(_) => {}
        }
        self
    }

    pub fn where_eq(
        self,
        column: impl Into<super::ast::ColumnRef>,
        value: impl Into<super::ast::DbValue>,
    ) -> Self {
        self.where_(Condition::compare(
            Expr::column(column.into()),
            ComparisonOp::Eq,
            Expr::value(value.into()),
        ))
    }

    pub fn where_ieq(
        self,
        column: impl Into<super::ast::ColumnRef>,
        value: impl Into<String>,
    ) -> Self {
        self.where_(Condition::compare(
            Expr::column(column.into()),
            ComparisonOp::IEq,
            Expr::value(DbValue::Text(value.into())),
        ))
    }

    pub fn where_in<I, V>(self, column: impl Into<super::ast::ColumnRef>, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<super::ast::DbValue>,
    {
        self.where_(Condition::InList {
            expr: Expr::column(column.into()),
            values: values.into_iter().map(Into::into).collect(),
        })
    }

    pub fn where_not_in<I, V>(self, column: impl Into<super::ast::ColumnRef>, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<super::ast::DbValue>,
    {
        self.where_(Condition::negate(Condition::InList {
            expr: Expr::column(column.into()),
            values: values.into_iter().map(Into::into).collect(),
        }))
    }

    pub fn group_by(mut self, expr: impl Into<Expr>) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.group_by.push(expr.into());
        }
        self
    }

    pub fn having(mut self, condition: Condition) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            select.having = merge_condition(select.having.take(), condition);
        }
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        match &mut self.ast.body {
            QueryBody::Select(select) => select.limit = Some(limit),
            QueryBody::SetOperation(set) => set.limit = Some(limit),
            QueryBody::Insert(_) | QueryBody::Update(_) | QueryBody::Delete(_) => {}
        }
        self
    }

    pub fn offset(mut self, offset: u64) -> Self {
        match &mut self.ast.body {
            QueryBody::Select(select) => select.offset = Some(offset),
            QueryBody::SetOperation(set) => set.offset = Some(offset),
            QueryBody::Insert(_) | QueryBody::Update(_) | QueryBody::Delete(_) => {}
        }
        self
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        match &mut self.ast.body {
            QueryBody::Select(select) => select.order_by.push(order),
            QueryBody::SetOperation(set) => set.order_by.push(order),
            QueryBody::Insert(_) | QueryBody::Update(_) | QueryBody::Delete(_) => {}
        }
        self
    }

    pub fn value(
        mut self,
        column: impl Into<super::ast::ColumnRef>,
        value: impl Into<super::ast::DbValue>,
    ) -> Self {
        self = self.value_expr(column, Expr::value(value.into()));
        self
    }

    pub fn value_expr(
        mut self,
        column: impl Into<super::ast::ColumnRef>,
        expr: impl Into<Expr>,
    ) -> Self {
        let column = column.into();
        let expr = expr.into();
        match &mut self.ast.body {
            QueryBody::Insert(insert) => {
                push_insert_expr_value(insert, (column, expr));
            }
            QueryBody::Update(update) => {
                update.values.push((column, expr));
            }
            QueryBody::Select(_) | QueryBody::Delete(_) | QueryBody::SetOperation(_) => {}
        }
        self
    }

    pub fn values<I, C, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = (C, V)>,
        C: Into<super::ast::ColumnRef>,
        V: Into<super::ast::DbValue>,
    {
        for (column, value) in values {
            self = self.value(column, value);
        }
        self
    }

    pub fn row<I, C, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = (C, V)>,
        C: Into<super::ast::ColumnRef>,
        V: Into<super::ast::DbValue>,
    {
        if let QueryBody::Insert(insert) = &mut self.ast.body {
            push_insert_expr_row(
                insert,
                values
                    .into_iter()
                    .map(|(column, value)| (column.into(), Expr::value(value.into())))
                    .collect(),
            );
        }
        self
    }

    pub fn rows<R, I, C, V>(mut self, rows: R) -> Self
    where
        R: IntoIterator<Item = I>,
        I: IntoIterator<Item = (C, V)>,
        C: Into<super::ast::ColumnRef>,
        V: Into<super::ast::DbValue>,
    {
        for row in rows {
            self = self.row(row);
        }
        self
    }

    pub fn on_conflict_columns<I, C>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<super::ast::ColumnRef>,
    {
        if let QueryBody::Insert(insert) = &mut self.ast.body {
            insert.on_conflict = Some(OnConflictNode {
                target: Some(OnConflictTarget::Columns(
                    columns.into_iter().map(Into::into).collect(),
                )),
                action: current_conflict_action(insert.on_conflict.take()),
            });
        }
        self
    }

    pub fn on_conflict_constraint(mut self, constraint: impl Into<String>) -> Self {
        if let QueryBody::Insert(insert) = &mut self.ast.body {
            insert.on_conflict = Some(OnConflictNode {
                target: Some(OnConflictTarget::Constraint(constraint.into())),
                action: current_conflict_action(insert.on_conflict.take()),
            });
        }
        self
    }

    pub fn do_nothing(mut self) -> Self {
        if let QueryBody::Insert(insert) = &mut self.ast.body {
            upsert_node(insert).action = OnConflictAction::DoNothing;
        }
        self
    }

    pub fn do_update(mut self) -> Self {
        if let QueryBody::Insert(insert) = &mut self.ast.body {
            upsert_node(insert).action = OnConflictAction::DoUpdate(Box::new(OnConflictUpdate {
                assignments: Vec::new(),
                condition: None,
            }));
        }
        self
    }

    pub fn set(
        mut self,
        column: impl Into<super::ast::ColumnRef>,
        value: impl Into<super::ast::DbValue>,
    ) -> Self {
        self = self.set_expr(column, Expr::value(value.into()));
        self
    }

    pub fn set_expr(
        mut self,
        column: impl Into<super::ast::ColumnRef>,
        expr: impl Into<Expr>,
    ) -> Self {
        let column = column.into();
        let expr = expr.into();
        match &mut self.ast.body {
            QueryBody::Insert(insert) => {
                if let Some(OnConflictNode {
                    action: OnConflictAction::DoUpdate(conflict),
                    ..
                }) = &mut insert.on_conflict
                {
                    conflict.assignments.push((column, expr));
                }
            }
            QueryBody::Update(update) => update.values.push((column, expr)),
            QueryBody::Select(_) | QueryBody::Delete(_) | QueryBody::SetOperation(_) => {}
        }
        self
    }

    pub fn set_excluded(mut self, column: impl Into<super::ast::ColumnRef>) -> Self {
        let column = column.into();
        self = self.set_expr(column.clone(), Expr::excluded(column));
        self
    }

    pub fn from(mut self, source: impl Into<FromItem>) -> Self {
        if let QueryBody::Update(update) = &mut self.ast.body {
            update.from.push(source.into());
        }
        self
    }

    pub fn using(mut self, source: impl Into<FromItem>) -> Self {
        if let QueryBody::Delete(delete) = &mut self.ast.body {
            delete.using.push(source.into());
        }
        self
    }

    pub fn returning<I, C>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<super::ast::ColumnRef>,
    {
        let items = columns
            .into_iter()
            .map(|column| SelectItem::new(Expr::column(column.into())))
            .collect::<Vec<_>>();
        match &mut self.ast.body {
            QueryBody::Insert(insert) => insert.returning = items,
            QueryBody::Update(update) => update.returning = items,
            QueryBody::Delete(delete) => delete.returning = items,
            QueryBody::Select(_) | QueryBody::SetOperation(_) => {}
        }
        self
    }

    pub fn union(self, other: Self) -> Self {
        let Self {
            ast: left,
            deferred_error: left_error,
            ..
        } = self;
        let Self {
            ast: right,
            deferred_error: right_error,
            ..
        } = other;
        let deferred_error = first_deferred_error(left_error, right_error);
        Self {
            ast: QueryAst::set_operation(SetOperationNode {
                left: Box::new(left),
                operator: SetOperator::Union,
                right: Box::new(right),
                order_by: Vec::new(),
                limit: None,
                offset: None,
            }),
            options: QueryExecutionOptions::default(),
            deferred_error,
        }
    }

    pub fn union_all(self, other: Self) -> Self {
        let Self {
            ast: left,
            deferred_error: left_error,
            ..
        } = self;
        let Self {
            ast: right,
            deferred_error: right_error,
            ..
        } = other;
        let deferred_error = first_deferred_error(left_error, right_error);
        Self {
            ast: QueryAst::set_operation(SetOperationNode {
                left: Box::new(left),
                operator: SetOperator::UnionAll,
                right: Box::new(right),
                order_by: Vec::new(),
                limit: None,
                offset: None,
            }),
            options: QueryExecutionOptions::default(),
            deferred_error,
        }
    }

    pub fn ast(&self) -> &QueryAst {
        &self.ast
    }

    pub fn compile(&self) -> Result<super::compiler::CompiledSql> {
        self.deferred_result()?;
        PostgresCompiler::compile(&self.ast)
    }

    pub fn to_compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.compile()
    }

    pub fn for_update(mut self) -> Self {
        self = self.lock(LockStrength::Update);
        self
    }

    pub fn for_no_key_update(mut self) -> Self {
        self = self.lock(LockStrength::NoKeyUpdate);
        self
    }

    pub fn for_share(mut self) -> Self {
        self = self.lock(LockStrength::Share);
        self
    }

    pub fn for_key_share(mut self) -> Self {
        self = self.lock(LockStrength::KeyShare);
        self
    }

    pub fn of<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if let QueryBody::Select(select) = &mut self.ast.body {
            let lock = select.lock.get_or_insert(LockClause {
                strength: LockStrength::Update,
                of: Vec::new(),
                behavior: LockBehavior::Wait,
            });
            lock.of.extend(aliases.into_iter().map(Into::into));
        }
        self
    }

    pub fn skip_locked(mut self) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            let lock = select.lock.get_or_insert(LockClause {
                strength: LockStrength::Update,
                of: Vec::new(),
                behavior: LockBehavior::Wait,
            });
            lock.behavior = LockBehavior::SkipLocked;
        }
        self
    }

    pub fn nowait(mut self) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            let lock = select.lock.get_or_insert(LockClause {
                strength: LockStrength::Update,
                of: Vec::new(),
                behavior: LockBehavior::Wait,
            });
            lock.behavior = LockBehavior::NoWait;
        }
        self
    }

    pub async fn get<E>(&self, executor: &E) -> Result<Collection<DbRecord>>
    where
        E: QueryExecutor + ?Sized,
    {
        let compiled = self.compile()?;
        executor
            .query_records_with(&compiled, self.options.clone())
            .await
            .map(Collection::from)
    }

    pub async fn all<E>(&self, executor: &E) -> Result<Collection<DbRecord>>
    where
        E: QueryExecutor + ?Sized,
    {
        self.get(executor).await
    }

    pub async fn first<E>(&self, executor: &E) -> Result<Option<DbRecord>>
    where
        E: QueryExecutor + ?Sized,
    {
        let query = match &self.ast.body {
            QueryBody::Select(_) | QueryBody::SetOperation(_) => self.clone().limit(1),
            QueryBody::Insert(_) | QueryBody::Update(_) | QueryBody::Delete(_) => self.clone(),
        };
        Ok(query.get(executor).await?.into_iter().next())
    }

    pub async fn execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor + ?Sized,
    {
        match &self.ast.body {
            QueryBody::Select(_) | QueryBody::SetOperation(_) => Err(Error::message(
                "execute() is not available for select queries; use get() or first() instead",
            )),
            QueryBody::Insert(_) | QueryBody::Update(_) | QueryBody::Delete(_) => {
                let compiled = self.compile()?;
                executor
                    .execute_compiled_with(&compiled, self.options.clone())
                    .await
            }
        }
    }

    pub fn stream<'a, E>(&'a self, executor: &'a E) -> Result<DbRecordStream<'a>>
    where
        E: QueryExecutor,
    {
        let compiled = self.compile()?;
        Ok(executor.stream_records(compiled, self.options.clone()))
    }

    pub async fn paginate<E>(
        &self,
        executor: &E,
        pagination: Pagination,
    ) -> Result<Paginated<DbRecord>>
    where
        E: QueryExecutor + ?Sized,
    {
        self.deferred_result()?;
        let total = count_query_ast(executor, &self.ast).await?;
        let data = self
            .clone()
            .limit(pagination.per_page)
            .offset(pagination.offset())
            .get(executor)
            .await?;

        Ok(Paginated {
            data,
            pagination,
            total,
        })
    }

    pub async fn count<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor + ?Sized,
    {
        self.deferred_result()?;
        count_query_ast(executor, &self.ast).await
    }

    pub async fn count_distinct<E>(&self, executor: &E, expr: impl Into<Expr>) -> Result<u64>
    where
        E: QueryExecutor + ?Sized,
    {
        self.deferred_result()?;
        Ok(execute_scalar_projection_on_ast(
            executor,
            &self.ast,
            AggregateProjection::<i64>::internal_count_distinct(expr.into()),
        )
        .await? as u64)
    }

    pub async fn sum<E, T>(&self, executor: &E, expr: impl Into<Expr>) -> Result<Option<T>>
    where
        E: QueryExecutor + ?Sized,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast,
            AggregateProjection::<Option<T>>::internal_sum(expr.into()),
        )
        .await
    }

    pub async fn avg<E, T>(&self, executor: &E, expr: impl Into<Expr>) -> Result<Option<T>>
    where
        E: QueryExecutor + ?Sized,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast,
            AggregateProjection::<Option<T>>::internal_avg(expr.into()),
        )
        .await
    }

    pub async fn min<E, T>(&self, executor: &E, expr: impl Into<Expr>) -> Result<Option<T>>
    where
        E: QueryExecutor + ?Sized,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast,
            AggregateProjection::<Option<T>>::internal_min(expr.into()),
        )
        .await
    }

    pub async fn max<E, T>(&self, executor: &E, expr: impl Into<Expr>) -> Result<Option<T>>
    where
        E: QueryExecutor + ?Sized,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast,
            AggregateProjection::<Option<T>>::internal_max(expr.into()),
        )
        .await
    }

    pub(crate) async fn aggregate_over_alias<E, T>(
        &self,
        executor: &E,
        alias: &str,
        projection: AggregateProjection<T>,
    ) -> Result<T>
    where
        E: QueryExecutor + ?Sized,
        T: FromDbValue,
    {
        self.deferred_result()?;
        let wrapped = wrap_query_for_alias_aggregate(
            &self.ast,
            alias,
            super::ast::DbType::Int64,
            projection.node(),
        );
        let compiled = PostgresCompiler::compile(&wrapped)?;
        let record = executor
            .query_records(&compiled)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::message("aggregate query returned no rows"))?;
        projection.decode(&record)
    }

    pub async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor + ?Sized,
    {
        explain_query(executor, &self.compile()?, false, self.options.clone()).await
    }

    pub async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor + ?Sized,
    {
        explain_query(executor, &self.compile()?, true, self.options.clone()).await
    }

    fn lock(mut self, strength: LockStrength) -> Self {
        if let QueryBody::Select(select) = &mut self.ast.body {
            let existing = select.lock.take().unwrap_or(LockClause {
                strength,
                of: Vec::new(),
                behavior: LockBehavior::Wait,
            });
            select.lock = Some(LockClause {
                strength,
                ..existing
            });
        }
        self
    }

    fn with_deferred_error(mut self, error: String) -> Self {
        if self.deferred_error.is_none() {
            self.deferred_error = Some(error);
        }
        self
    }

    fn deferred_result(&self) -> Result<()> {
        match &self.deferred_error {
            Some(error) => Err(Error::message(error.clone())),
            None => Ok(()),
        }
    }
}

#[derive(Clone)]
pub struct ProjectionQuery<P: 'static> {
    query: Query,
    meta: &'static ProjectionMeta<P>,
}

impl<P> ProjectionQuery<P>
where
    P: Projection,
{
    pub fn table(source: impl Into<FromItem>) -> Self {
        Self::new(source, P::projection_meta())
    }
}

impl<P> ProjectionQuery<P>
where
    P: Clone + Send + Sync + 'static,
{
    pub(crate) fn new(source: impl Into<FromItem>, meta: &'static ProjectionMeta<P>) -> Self {
        Self {
            query: Query::table(source),
            meta,
        }
    }

    pub fn with_cte(mut self, cte: Cte) -> Self {
        self.query = self.query.with_cte(cte);
        self
    }

    pub fn distinct(mut self) -> Self {
        self.query = self.query.distinct();
        self
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.query = self.query.with_timeout(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.query = self.query.with_label(label);
        self
    }

    pub fn select_field<T>(mut self, field: ProjectionField<P, T>, expr: impl Into<Expr>) -> Self {
        self.query = self.query.select_item(field.select(expr));
        self
    }

    pub fn select_source<T>(mut self, field: ProjectionField<P, T>, table_alias: &str) -> Self {
        self.query = self
            .query
            .select_item(field.select_from(table_alias).unwrap_or_else(|_| {
                field.select(Expr::column(
                    super::ast::ColumnRef::new(table_alias, field.alias()).typed(field.db_type()),
                ))
            }));
        self
    }

    pub fn select_aggregate<T>(mut self, projection: AggregateProjection<T>) -> Self {
        self.query = self.query.select_aggregate(projection);
        self
    }

    pub fn join(mut self, kind: JoinKind, table: impl Into<FromItem>, on: Condition) -> Self {
        self.query = self.query.join(kind, table, on);
        self
    }

    pub fn inner_join(mut self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.query = self.query.inner_join(table, on);
        self
    }

    pub fn left_join(mut self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.query = self.query.left_join(table, on);
        self
    }

    pub fn right_join(mut self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.query = self.query.right_join(table, on);
        self
    }

    pub fn full_outer_join(mut self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.query = self.query.full_outer_join(table, on);
        self
    }

    pub fn cross_join(mut self, table: impl Into<FromItem>) -> Self {
        self.query = self.query.cross_join(table);
        self
    }

    pub fn inner_join_lateral(mut self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.query = self.query.inner_join_lateral(table, on);
        self
    }

    pub fn left_join_lateral(mut self, table: impl Into<FromItem>, on: Condition) -> Self {
        self.query = self.query.left_join_lateral(table, on);
        self
    }

    pub fn cross_join_lateral(mut self, table: impl Into<FromItem>) -> Self {
        self.query = self.query.cross_join_lateral(table);
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        self.query = self.query.where_(condition);
        self
    }

    pub fn group_by(mut self, expr: impl Into<Expr>) -> Self {
        self.query = self.query.group_by(expr);
        self
    }

    pub fn having(mut self, condition: Condition) -> Self {
        self.query = self.query.having(condition);
        self
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.query = self.query.order_by(order);
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.query = self.query.limit(limit);
        self
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.query = self.query.offset(offset);
        self
    }

    /// Apply a reusable query scope.
    pub fn scope(self, f: impl FnOnce(Self) -> Self) -> Self {
        let fallback = self.clone();
        match catch_sync_panic(|| f(self)) {
            Ok(query) => query,
            Err(panic) => fallback
                .with_deferred_error(projection_query_dsl_panic_message::<P>("scope", panic)),
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            query: self.query.union(other.query),
            meta: self.meta,
        }
    }

    pub fn union_all(self, other: Self) -> Self {
        Self {
            query: self.query.union_all(other.query),
            meta: self.meta,
        }
    }

    pub fn ast(&self) -> &QueryAst {
        self.query.ast()
    }

    pub fn to_compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.query.to_compiled_sql()
    }

    pub fn for_update(mut self) -> Self {
        self.query = self.query.for_update();
        self
    }

    pub fn for_no_key_update(mut self) -> Self {
        self.query = self.query.for_no_key_update();
        self
    }

    pub fn for_share(mut self) -> Self {
        self.query = self.query.for_share();
        self
    }

    pub fn for_key_share(mut self) -> Self {
        self.query = self.query.for_key_share();
        self
    }

    pub fn of<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.query = self.query.of(aliases);
        self
    }

    pub fn skip_locked(mut self) -> Self {
        self.query = self.query.skip_locked();
        self
    }

    pub fn nowait(mut self) -> Self {
        self.query = self.query.nowait();
        self
    }

    pub async fn get<E>(&self, executor: &E) -> Result<Collection<P>>
    where
        E: QueryExecutor,
    {
        self.query
            .get(executor)
            .await?
            .iter()
            .map(|record| self.meta.hydrate_record(record))
            .collect()
    }

    pub fn stream<'a, E>(&'a self, executor: &'a E) -> Result<BoxStream<'a, Result<P>>>
    where
        E: QueryExecutor,
    {
        Ok(self
            .query
            .stream(executor)?
            .map(|record| record.and_then(|record| self.meta.hydrate_record(&record)))
            .boxed())
    }

    pub async fn first<E>(&self, executor: &E) -> Result<Option<P>>
    where
        E: QueryExecutor,
    {
        Ok(self.get(executor).await?.into_iter().next())
    }

    pub async fn paginate<E>(&self, executor: &E, pagination: Pagination) -> Result<Paginated<P>>
    where
        E: QueryExecutor,
    {
        let total = self.query.count(executor).await?;
        let data = self
            .clone()
            .limit(pagination.per_page)
            .offset(pagination.offset())
            .get(executor)
            .await?;
        Ok(Paginated {
            data,
            pagination,
            total,
        })
    }

    pub async fn count<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor,
    {
        self.query.count(executor).await
    }

    pub async fn count_distinct<E, T>(
        &self,
        executor: &E,
        field: ProjectionField<P, T>,
    ) -> Result<u64>
    where
        E: QueryExecutor,
    {
        Ok(self
            .query
            .aggregate_over_alias(
                executor,
                field.alias(),
                AggregateProjection::<i64>::internal_count_distinct(field.alias()),
            )
            .await? as u64)
    }

    pub async fn sum<E, T>(&self, executor: &E, field: ProjectionField<P, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        let alias = field.alias();
        self.query.deferred_result()?;
        let wrapped = wrap_query_for_alias_aggregate(
            self.query.ast(),
            alias,
            field.db_type(),
            AggregateProjection::<Option<T>>::internal_sum(alias).node(),
        );
        decode_wrapped_projection(
            executor,
            wrapped,
            AggregateProjection::<Option<T>>::internal_sum(alias),
        )
        .await
    }

    pub async fn avg<E, T>(&self, executor: &E, field: ProjectionField<P, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        let alias = field.alias();
        self.query.deferred_result()?;
        let wrapped = wrap_query_for_alias_aggregate(
            self.query.ast(),
            alias,
            field.db_type(),
            AggregateProjection::<Option<T>>::internal_avg(alias).node(),
        );
        decode_wrapped_projection(
            executor,
            wrapped,
            AggregateProjection::<Option<T>>::internal_avg(alias),
        )
        .await
    }

    pub async fn min<E, T>(&self, executor: &E, field: ProjectionField<P, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        let alias = field.alias();
        self.query.deferred_result()?;
        let wrapped = wrap_query_for_alias_aggregate(
            self.query.ast(),
            alias,
            field.db_type(),
            AggregateProjection::<Option<T>>::internal_min(alias).node(),
        );
        decode_wrapped_projection(
            executor,
            wrapped,
            AggregateProjection::<Option<T>>::internal_min(alias),
        )
        .await
    }

    pub async fn max<E, T>(&self, executor: &E, field: ProjectionField<P, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        let alias = field.alias();
        self.query.deferred_result()?;
        let wrapped = wrap_query_for_alias_aggregate(
            self.query.ast(),
            alias,
            field.db_type(),
            AggregateProjection::<Option<T>>::internal_max(alias).node(),
        );
        decode_wrapped_projection(
            executor,
            wrapped,
            AggregateProjection::<Option<T>>::internal_max(alias),
        )
        .await
    }

    pub async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        self.query.explain(executor).await
    }

    pub async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        self.query.explain_analyze(executor).await
    }

    fn with_deferred_error(mut self, error: String) -> Self {
        self.query = self.query.with_deferred_error(error);
        self
    }
}

#[derive(Clone)]
pub struct ModelQuery<M: 'static> {
    table: &'static TableMeta<M>,
    with: Vec<CteNode>,
    select: SelectNode,
    relations: Vec<AnyRelation<M>>,
    model_extensions: Vec<AnyModelExtension<M>>,
    relation_aggregates: Vec<AnyRelationAggregate<M>>,
    soft_delete_scope: SoftDeleteScope,
    stream_batch_size: usize,
    options: QueryExecutionOptions,
    skip_defaults: bool,
    deferred_error: Option<String>,
}

impl<M> ModelQuery<M>
where
    M: Model,
{
    pub fn new(table: &'static TableMeta<M>) -> Self {
        Self {
            table,
            with: Vec::new(),
            select: SelectNode {
                from: FromItem::Table(table.table_ref()),
                distinct: false,
                columns: table.all_select_items(),
                joins: Vec::new(),
                condition: None,
                group_by: Vec::new(),
                having: None,
                order_by: Vec::new(),
                limit: None,
                offset: None,
                lock: None,
                relations: Vec::new(),
                aggregates: Vec::new(),
            },
            relations: Vec::new(),
            model_extensions: Vec::new(),
            relation_aggregates: Vec::new(),
            soft_delete_scope: SoftDeleteScope::ActiveOnly,
            stream_batch_size: 256,
            options: QueryExecutionOptions::default(),
            skip_defaults: false,
            deferred_error: None,
        }
    }

    /// Skip auto-eager-loading of default relations (set via `always_with` derive attribute).
    pub fn without_defaults(mut self) -> Self {
        self.skip_defaults = true;
        self
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.options.label = Some(label.into());
        self
    }

    /// Force this query to use the write (primary) pool instead of the read replica.
    ///
    /// Useful when you need to read data that was just written and may not
    /// have replicated yet.
    pub fn use_write_pool(mut self) -> Self {
        self.options.use_write_pool = true;
        self
    }

    pub fn with_stream_batch_size(mut self, batch_size: usize) -> Self {
        self.stream_batch_size = batch_size.max(1);
        self
    }

    pub fn with_cte(mut self, cte: Cte) -> Self {
        self.with.push(cte.node);
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        self.select.condition = merge_condition(self.select.condition.take(), condition);
        self
    }

    pub fn where_in<T, I, V>(self, column: Column<M, T>, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: IntoColumnValue<T>,
        T: ToDbValue,
    {
        self.where_(column.in_list(values))
    }

    pub fn where_not_in<T, I, V>(self, column: Column<M, T>, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: IntoColumnValue<T>,
        T: ToDbValue,
    {
        self.where_(column.not_in_list(values))
    }

    pub fn group_by(mut self, expr: impl Into<Expr>) -> Self {
        self.select.group_by.push(expr.into());
        self
    }

    pub fn having(mut self, condition: Condition) -> Self {
        self.select.having = merge_condition(self.select.having.take(), condition);
        self
    }

    /// Filter results using PostgreSQL full-text search.
    ///
    /// Generates a `WHERE to_tsvector('english', col1 || ' ' || col2) @@ plainto_tsquery('english', ?)`
    /// condition across the given columns.
    ///
    /// **Important:** For performance, create a GIN index matching the expression:
    /// ```sql
    /// CREATE INDEX idx_users_search ON users
    ///     USING GIN (to_tsvector('english', COALESCE(name, '') || ' ' || COALESCE(email, '')));
    /// ```
    /// Without a matching index, PostgreSQL performs a full sequential scan.
    /// The index expression must match the columns passed to `search()` exactly.
    pub fn search<T>(self, columns: &[Column<M, T>], query: &str) -> Self {
        self.where_(Condition::full_text(
            columns.iter().map(|column| column.column_ref()),
            query,
        ))
    }

    pub fn with_trashed(mut self) -> Self {
        self.soft_delete_scope = SoftDeleteScope::WithTrashed;
        self
    }

    pub fn only_trashed(mut self) -> Self {
        self.soft_delete_scope = SoftDeleteScope::OnlyTrashed;
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.select.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.select.offset = Some(offset);
        self
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.select.order_by.push(order);
        self
    }

    /// Apply a reusable query scope.
    ///
    /// Scopes are functions that modify the query, e.g.:
    /// ```ignore
    /// impl User {
    ///     pub fn active(query: ModelQuery<Self>) -> ModelQuery<Self> {
    ///         query.where_(User::ACTIVE.eq(true))
    ///     }
    /// }
    /// User::query().scope(User::active).get(&app).await?;
    /// ```
    pub fn scope(self, f: impl FnOnce(Self) -> Self) -> Self {
        let fallback = self.clone();
        match catch_sync_panic(|| f(self)) {
            Ok(query) => query,
            Err(panic) => {
                fallback.with_deferred_error(model_query_dsl_panic_message::<M>("scope", panic))
            }
        }
    }

    pub fn with<To>(mut self, relation: RelationDef<M, To>) -> Self
    where
        To: Model,
    {
        self.select.relations.push(relation.node());
        self.relations.push(std::sync::Arc::new(relation));
        self
    }

    pub fn with_attachments(mut self, collection: impl Into<String>) -> Self
    where
        M: crate::attachments::HasAttachments,
    {
        self.model_extensions
            .push(crate::attachments::attachment_extension_loader(
                collection.into(),
            ));
        self
    }

    pub fn with_translated_field(mut self, field: impl Into<String>) -> Self
    where
        M: crate::translations::HasTranslations,
    {
        self.model_extensions
            .push(crate::translations::translated_field_extension_loader(
                field.into(),
            ));
        self
    }

    pub fn with_translations_for(mut self, locale: impl Into<String>) -> Self
    where
        M: crate::translations::HasTranslations,
    {
        self.model_extensions
            .push(crate::translations::translations_for_extension_loader(
                locale.into(),
            ));
        self
    }

    pub fn with_all_translations(mut self) -> Self
    where
        M: crate::translations::HasTranslations,
    {
        self.model_extensions
            .push(crate::translations::all_translations_extension_loader());
        self
    }

    pub fn with_many_to_many<To, Pivot>(mut self, relation: ManyToManyDef<M, To, Pivot>) -> Self
    where
        To: Model,
        Pivot: Clone + Send + Sync + 'static,
    {
        self.select.relations.push(relation.node());
        self.relations.push(std::sync::Arc::new(relation));
        self
    }

    pub fn with_aggregate<Value>(mut self, aggregate: RelationAggregateDef<M, Value>) -> Self {
        self.select.relations.push(aggregate.node());
        self.relation_aggregates.push(aggregate.into_loader());
        self
    }

    pub(crate) fn with_aggregate_boxed(mut self, aggregate: AnyRelationAggregate<M>) -> Self {
        self.select.relations.push(aggregate.node());
        self.relation_aggregates.push(aggregate);
        self
    }

    pub(crate) fn with_boxed(mut self, relation: AnyRelation<M>) -> Self {
        self.select.relations.push(relation.node());
        self.relations.push(relation);
        self
    }

    pub(crate) fn with_extension_boxed(mut self, extension: AnyModelExtension<M>) -> Self {
        self.model_extensions.push(extension);
        self
    }

    pub fn where_has<To, F>(mut self, relation: RelationDef<M, To>, scope: F) -> Self
    where
        To: Model,
        F: FnOnce(ModelQuery<To>) -> ModelQuery<To>,
    {
        let scoped = match catch_sync_panic(|| scope(ModelQuery::new(To::table_meta()))) {
            Ok(scoped) => scoped,
            Err(panic) => {
                return self
                    .with_deferred_error(model_query_dsl_panic_message::<M>("where_has", panic));
            }
        };
        if let Some(error) = scoped.deferred_error.clone() {
            return self.with_deferred_error(error);
        }
        let relation = relation.scoped_with_filter(scoped.effective_condition());
        self.select.condition =
            merge_condition(self.select.condition.take(), relation.exists_condition());
        self
    }

    pub fn where_has_many_to_many<To, Pivot, F>(
        mut self,
        relation: ManyToManyDef<M, To, Pivot>,
        scope: F,
    ) -> Self
    where
        To: Model,
        Pivot: Clone + Send + Sync + 'static,
        F: FnOnce(ModelQuery<To>) -> ModelQuery<To>,
    {
        let scoped = match catch_sync_panic(|| scope(ModelQuery::new(To::table_meta()))) {
            Ok(scoped) => scoped,
            Err(panic) => {
                return self.with_deferred_error(model_query_dsl_panic_message::<M>(
                    "where_has_many_to_many",
                    panic,
                ));
            }
        };
        if let Some(error) = scoped.deferred_error.clone() {
            return self.with_deferred_error(error);
        }
        let relation = relation.scoped_with_filter(scoped.effective_condition());
        self.select.condition =
            merge_condition(self.select.condition.take(), relation.exists_condition());
        self
    }

    pub fn ast(&self) -> QueryAst {
        let mut select = self.select.clone();
        select.condition = self.effective_condition();
        QueryAst {
            with: self.with.clone(),
            body: QueryBody::Select(Box::new(select)),
        }
    }

    pub fn to_compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.deferred_result()?;
        PostgresCompiler::compile(&self.ast())
    }

    pub fn for_update(mut self) -> Self {
        self = self.lock(LockStrength::Update);
        self
    }

    pub fn for_no_key_update(mut self) -> Self {
        self = self.lock(LockStrength::NoKeyUpdate);
        self
    }

    pub fn for_share(mut self) -> Self {
        self = self.lock(LockStrength::Share);
        self
    }

    pub fn for_key_share(mut self) -> Self {
        self = self.lock(LockStrength::KeyShare);
        self
    }

    pub fn of<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let lock = self.select.lock.get_or_insert(LockClause {
            strength: LockStrength::Update,
            of: Vec::new(),
            behavior: LockBehavior::Wait,
        });
        lock.of.extend(aliases.into_iter().map(Into::into));
        self
    }

    pub fn skip_locked(mut self) -> Self {
        let lock = self.select.lock.get_or_insert(LockClause {
            strength: LockStrength::Update,
            of: Vec::new(),
            behavior: LockBehavior::Wait,
        });
        lock.behavior = LockBehavior::SkipLocked;
        self
    }

    pub fn nowait(mut self) -> Self {
        let lock = self.select.lock.get_or_insert(LockClause {
            strength: LockStrength::Update,
            of: Vec::new(),
            behavior: LockBehavior::Wait,
        });
        lock.behavior = LockBehavior::NoWait;
        self
    }

    fn effective_condition(&self) -> Option<Condition> {
        merge_optional_condition(self.select.condition.clone(), self.soft_delete_condition())
    }

    fn soft_delete_condition(&self) -> Option<Condition> {
        if !self.table.soft_deletes_enabled() {
            return None;
        }

        let deleted_at = self.table.deleted_at_column_info()?;
        let deleted_at_ref =
            ColumnRef::new(self.table.name(), deleted_at.name).typed(deleted_at.db_type);

        match self.soft_delete_scope {
            SoftDeleteScope::ActiveOnly => Some(Condition::is_null(deleted_at_ref)),
            SoftDeleteScope::WithTrashed => None,
            SoftDeleteScope::OnlyTrashed => Some(Condition::is_not_null(deleted_at_ref)),
        }
    }

    pub async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: QueryExecutor,
    {
        let mut entries = self.fetch_entries_dyn(executor).await?;
        Ok(entries.drain(..).map(|(_, model)| model).collect())
    }

    pub async fn all<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: QueryExecutor,
    {
        self.get(executor).await
    }

    pub fn stream<'a, E>(&'a self, executor: &'a E) -> Result<BoxStream<'a, Result<M>>>
    where
        E: QueryExecutor,
    {
        let compiled = self.to_compiled_sql()?;
        Ok(model_query_stream(ModelStreamState {
            executor,
            root_stream: executor.stream_records(compiled, self.options.clone()),
            table: self.table,
            relations: self.relations.clone(),
            relation_aggregates: self.relation_aggregates.clone(),
            model_extensions: self.model_extensions.clone(),
            stream_batch_size: self.stream_batch_size.max(1),
            buffered: VecDeque::new(),
            pending_error: None,
            finished: false,
            options: self.options.clone(),
        }))
    }

    pub async fn first<E>(&self, executor: &E) -> Result<Option<M>>
    where
        E: QueryExecutor,
    {
        Ok(self
            .clone()
            .limit(1)
            .get(executor)
            .await?
            .into_iter()
            .next())
    }

    pub async fn first_or_fail<E>(&self, executor: &E) -> Result<M>
    where
        E: QueryExecutor,
    {
        self.first(executor).await?.ok_or_else(|| {
            Error::message(format!(
                "model query for `{}` returned no records",
                self.table.name()
            ))
        })
    }

    pub async fn find<E, K>(&self, executor: &E, key: K) -> Result<Option<M>>
    where
        E: QueryExecutor,
        K: ToDbValue,
    {
        Ok(self
            .clone()
            .where_(self.primary_key_condition(key)?)
            .limit(1)
            .get(executor)
            .await?
            .into_iter()
            .next())
    }

    pub async fn find_or_fail<E, K>(&self, executor: &E, key: K) -> Result<M>
    where
        E: QueryExecutor,
        K: ToDbValue,
    {
        self.find(executor, key).await?.ok_or_else(|| {
            Error::message(format!(
                "model query for `{}` did not find a matching primary key",
                self.table.name()
            ))
        })
    }

    pub async fn find_many<E, I, K>(&self, executor: &E, keys: I) -> Result<Collection<M>>
    where
        E: QueryExecutor,
        I: IntoIterator<Item = K>,
        K: ToDbValue,
    {
        let values = keys
            .into_iter()
            .map(ToDbValue::to_db_value)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return Ok(Collection::new());
        }

        self.clone()
            .where_(Condition::InList {
                expr: Expr::column(self.primary_key_column_ref()?),
                values,
            })
            .get(executor)
            .await
    }

    pub async fn exists<E>(&self, executor: &E) -> Result<bool>
    where
        E: QueryExecutor,
    {
        self.deferred_result()?;
        let compiled = PostgresCompiler::compile(&self.exists_ast())?;
        Ok(!executor
            .query_records_with(&compiled, self.options.clone())
            .await?
            .is_empty())
    }

    pub async fn doesnt_exist<E>(&self, executor: &E) -> Result<bool>
    where
        E: QueryExecutor,
    {
        Ok(!self.exists(executor).await?)
    }

    pub async fn value<E, T>(&self, executor: &E, column: Column<M, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        self.deferred_result()?;
        let mut select = self.select.clone();
        select.columns =
            vec![SelectItem::new(Expr::column(column.column_ref())).aliased(column.name())];
        select.aggregates.clear();
        select.relations.clear();
        select.condition = self.effective_condition();
        select.limit = Some(1);

        let compiled = PostgresCompiler::compile(&QueryAst {
            with: self.with.clone(),
            body: QueryBody::Select(Box::new(select)),
        })?;
        executor
            .query_records_with(&compiled, self.options.clone())
            .await?
            .into_iter()
            .next()
            .map(|record| record.decode::<T>(column.name()))
            .transpose()
    }

    pub async fn chunk<E, F, Fut>(&self, executor: &E, size: u64, mut handler: F) -> Result<()>
    where
        E: QueryExecutor,
        F: FnMut(Collection<M>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let size = size.max(1);
        let mut offset = 0_u64;

        loop {
            let batch = self
                .clone()
                .limit(size)
                .offset(offset)
                .get(executor)
                .await?;
            let batch_len = batch.len();
            if batch_len == 0 {
                break;
            }

            run_query_iteration_callback::<M, _, _, _>("chunk", &mut handler, batch).await?;
            if batch_len < size as usize {
                break;
            }
            offset += size;
        }

        Ok(())
    }

    pub async fn chunk_by_id<E, T, F, Fut>(
        &self,
        executor: &E,
        column: Column<M, T>,
        size: u64,
        mut handler: F,
    ) -> Result<()>
    where
        E: QueryExecutor,
        T: Clone + FromDbValue + ToDbValue,
        F: FnMut(Collection<M>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let size = size.max(1);
        let mut last_value = None::<T>;

        loop {
            let mut query = self.clone();
            query.select.order_by.clear();
            if let Some(value) = last_value.clone() {
                query = query.where_(column.gt(value));
            }
            let entries = query
                .order_by(column.asc())
                .limit(size)
                .fetch_entries_dyn(executor)
                .await?;
            let batch_len = entries.len();
            if batch_len == 0 {
                break;
            }

            let next_last_value = entries
                .last()
                .map(|(record, _)| record.decode::<T>(column.name()))
                .transpose()?;
            let models = entries
                .into_iter()
                .map(|(_, model)| model)
                .collect::<Collection<_>>();
            run_query_iteration_callback::<M, _, _, _>("chunk_by_id", &mut handler, models).await?;
            last_value = next_last_value;

            if batch_len < size as usize {
                break;
            }
        }

        Ok(())
    }

    pub async fn each_by_id<E, T, F, Fut>(
        &self,
        executor: &E,
        column: Column<M, T>,
        size: u64,
        mut handler: F,
    ) -> Result<()>
    where
        E: QueryExecutor,
        T: Clone + FromDbValue + ToDbValue,
        F: FnMut(M) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let size = size.max(1);
        let mut last_value = None::<T>;

        loop {
            let mut query = self.clone();
            query.select.order_by.clear();
            if let Some(value) = last_value.clone() {
                query = query.where_(column.gt(value));
            }
            let entries = query
                .order_by(column.asc())
                .limit(size)
                .fetch_entries_dyn(executor)
                .await?;
            let batch_len = entries.len();
            if batch_len == 0 {
                break;
            }

            let next_last_value = entries
                .last()
                .map(|(record, _)| record.decode::<T>(column.name()))
                .transpose()?;
            for (_, model) in entries {
                run_query_iteration_callback::<M, _, _, _>("each_by_id", &mut handler, model)
                    .await?;
            }
            last_value = next_last_value;

            if batch_len < size as usize {
                break;
            }
        }

        Ok(())
    }

    pub async fn paginate<E>(&self, executor: &E, pagination: Pagination) -> Result<Paginated<M>>
    where
        E: QueryExecutor,
    {
        self.deferred_result()?;
        let total = count_query_ast(executor, &self.ast()).await?;
        let data = self
            .clone()
            .limit(pagination.per_page)
            .offset(pagination.offset())
            .get(executor)
            .await?;
        Ok(Paginated {
            data,
            pagination,
            total,
        })
    }

    /// Cursor-based pagination for large datasets.
    ///
    /// Uses the given column as cursor. The cursor value is the base64url-encoded
    /// string representation of the column value for the boundary item.
    ///
    /// Returns `CursorPaginated` with `has_next`/`has_prev` metadata. Use
    /// `CursorPaginated::with_cursors` to attach encoded cursor values from
    /// the first/last items in the result.
    pub async fn cursor_paginate<E, V>(
        self,
        executor: &E,
        column: Column<M, V>,
        cursor: CursorPagination,
    ) -> Result<CursorPaginated<M>>
    where
        E: QueryExecutor,
        V: ToDbValue + FromDbValue + std::fmt::Display + std::str::FromStr,
        M: Serialize,
    {
        let per_page = cursor.per_page;
        let mut query = self;
        let is_forward = cursor.before.is_none();

        if let Some(ref after_cursor) = cursor.after {
            let value: V = decode_cursor(after_cursor)?;
            query = query.where_(column.gt(value));
            query = query.order_by(column.asc());
        } else if let Some(ref before_cursor) = cursor.before {
            let value: V = decode_cursor(before_cursor)?;
            query = query.where_(column.lt(value));
            query = query.order_by(column.desc());
        } else {
            query = query.order_by(column.asc());
        }

        let mut items: Vec<M> = query
            .limit(per_page + 1)
            .get(executor)
            .await?
            .into_iter()
            .collect();
        let has_more = items.len() as u64 > per_page;
        if has_more {
            items.pop();
        }

        // If we queried backwards, reverse to restore natural order
        if !is_forward {
            items.reverse();
        }

        let (has_next, has_prev) = if is_forward {
            (has_more, cursor.after.is_some())
        } else {
            (cursor.before.is_some(), has_more)
        };

        Ok(CursorPaginated {
            data: items,
            meta: CursorMeta {
                has_next,
                has_prev,
                per_page,
            },
            cursors: CursorInfo {
                next: None,
                prev: None,
            },
        })
    }

    pub async fn count<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor,
    {
        self.deferred_result()?;
        count_query_ast(executor, &self.ast()).await
    }

    pub async fn count_distinct<E, T>(&self, executor: &E, column: Column<M, T>) -> Result<u64>
    where
        E: QueryExecutor,
    {
        self.deferred_result()?;
        Ok(execute_scalar_projection_on_ast(
            executor,
            &self.ast(),
            AggregateProjection::<i64>::internal_count_distinct(column.column_ref()),
        )
        .await? as u64)
    }

    pub async fn sum<E, T>(&self, executor: &E, column: Column<M, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast(),
            AggregateProjection::<Option<T>>::internal_sum(column.column_ref()),
        )
        .await
    }

    pub async fn avg<E, T>(&self, executor: &E, column: Column<M, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast(),
            AggregateProjection::<Option<T>>::internal_avg(column.column_ref()),
        )
        .await
    }

    pub async fn min<E, T>(&self, executor: &E, column: Column<M, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast(),
            AggregateProjection::<Option<T>>::internal_min(column.column_ref()),
        )
        .await
    }

    pub async fn max<E, T>(&self, executor: &E, column: Column<M, T>) -> Result<Option<T>>
    where
        E: QueryExecutor,
        T: FromDbValue,
    {
        self.deferred_result()?;
        execute_scalar_projection_on_ast(
            executor,
            &self.ast(),
            AggregateProjection::<Option<T>>::internal_max(column.column_ref()),
        )
        .await
    }

    pub(crate) async fn fetch_entries_dyn(
        &self,
        executor: &dyn QueryExecutor,
    ) -> Result<Vec<(DbRecord, M)>> {
        let compiled = self.to_compiled_sql()?;
        let records = executor
            .query_records_with(&compiled, self.options.clone())
            .await?;
        let models = hydrate_model_batch(
            executor,
            self.table,
            &self.relations,
            &self.model_extensions,
            &self.relation_aggregates,
            &records,
            &self.options,
        )
        .await?;

        Ok(records.into_iter().zip(models).collect())
    }

    pub async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            false,
            self.options.clone(),
        )
        .await
    }

    pub async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            true,
            self.options.clone(),
        )
        .await
    }

    fn lock(mut self, strength: LockStrength) -> Self {
        let existing = self.select.lock.take().unwrap_or(LockClause {
            strength,
            of: Vec::new(),
            behavior: LockBehavior::Wait,
        });
        self.select.lock = Some(LockClause {
            strength,
            ..existing
        });
        self
    }

    fn with_deferred_error(mut self, error: String) -> Self {
        if self.deferred_error.is_none() {
            self.deferred_error = Some(error);
        }
        self
    }

    fn deferred_result(&self) -> Result<()> {
        match &self.deferred_error {
            Some(error) => Err(Error::message(error.clone())),
            None => Ok(()),
        }
    }

    fn exists_ast(&self) -> QueryAst {
        let mut select = self.select.clone();
        select.columns = vec![SelectItem::new(Expr::value(1_i64)).aliased("__foundry_exists")];
        select.aggregates.clear();
        select.relations.clear();
        select.order_by.clear();
        select.condition = self.effective_condition();
        select.limit = Some(1);

        QueryAst {
            with: self.with.clone(),
            body: QueryBody::Select(Box::new(select)),
        }
    }

    fn primary_key_column_ref(&self) -> Result<ColumnRef> {
        let column = self.table.primary_key_column_info().ok_or_else(|| {
            Error::message(format!(
                "missing primary key column `{}` on table `{}`",
                self.table.primary_key_name(),
                self.table.name()
            ))
        })?;
        Ok(ColumnRef::new(self.table.name(), column.name).typed(column.db_type))
    }

    fn primary_key_condition<K>(&self, key: K) -> Result<Condition>
    where
        K: ToDbValue,
    {
        Ok(Condition::compare(
            Expr::column(self.primary_key_column_ref()?),
            ComparisonOp::Eq,
            Expr::value(key.to_db_value()),
        ))
    }
}

#[derive(Clone)]
pub struct CreateModel<M: 'static> {
    table: &'static TableMeta<M>,
    rows: Vec<Vec<(super::ast::ColumnRef, Expr)>>,
    on_conflict: Option<OnConflictNode>,
    options: QueryExecutionOptions,
}

#[derive(Clone)]
pub struct CreateManyModel<M: 'static> {
    table: &'static TableMeta<M>,
    rows: Vec<Vec<(super::ast::ColumnRef, Expr)>>,
    on_conflict: Option<OnConflictNode>,
    without_lifecycle: bool,
    options: QueryExecutionOptions,
    row_builder_error: Option<String>,
}

pub struct CreateRow<M: 'static> {
    values: Vec<(super::ast::ColumnRef, Expr)>,
    _marker: PhantomData<fn() -> M>,
}

impl<M> CreateRow<M> {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            _marker: PhantomData,
        }
    }

    fn into_values(self) -> Vec<(super::ast::ColumnRef, Expr)> {
        self.values
    }

    pub fn set<T, V>(mut self, column: Column<M, T>, value: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        self.values.push((
            column.column_ref(),
            Expr::value(value.into_field_value(column.db_type())),
        ));
        self
    }

    pub fn set_expr<T>(mut self, column: Column<M, T>, expr: impl Into<Expr>) -> Self {
        self.values.push((column.column_ref(), expr.into()));
        self
    }

    pub fn set_null<T>(mut self, column: Column<M, T>) -> Self {
        self.values.push((
            column.column_ref(),
            Expr::value(super::ast::DbValue::Null(column.db_type())),
        ));
        self
    }
}

impl<M> CreateModel<M>
where
    M: Model,
{
    pub(crate) fn new(table: &'static TableMeta<M>) -> Self {
        Self {
            table,
            rows: Vec::new(),
            on_conflict: None,
            options: QueryExecutionOptions::default(),
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.options.label = Some(label.into());
        self
    }

    pub fn set<T, V>(mut self, column: Column<M, T>, value: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        ensure_insert_row(&mut self.rows).push((
            column.column_ref(),
            Expr::value(value.into_field_value(column.db_type())),
        ));
        self
    }

    pub fn set_expr<T>(mut self, column: Column<M, T>, expr: impl Into<Expr>) -> Self {
        ensure_insert_row(&mut self.rows).push((column.column_ref(), expr.into()));
        self
    }

    pub(crate) fn set_column_value(
        mut self,
        column: super::ast::ColumnRef,
        value: super::ast::DbValue,
    ) -> Self {
        ensure_insert_row(&mut self.rows).push((column, Expr::value(value)));
        self
    }

    pub fn on_conflict_columns<I, C>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<super::ast::ColumnRef>,
    {
        self.on_conflict = Some(OnConflictNode {
            target: Some(OnConflictTarget::Columns(
                columns.into_iter().map(Into::into).collect(),
            )),
            action: current_conflict_action(self.on_conflict.take()),
        });
        self
    }

    pub fn on_conflict_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.on_conflict = Some(OnConflictNode {
            target: Some(OnConflictTarget::Constraint(constraint.into())),
            action: current_conflict_action(self.on_conflict.take()),
        });
        self
    }

    pub fn do_nothing(mut self) -> Self {
        upsert_node_model(&mut self.on_conflict).action = OnConflictAction::DoNothing;
        self
    }

    pub fn do_update(mut self) -> Self {
        upsert_node_model(&mut self.on_conflict).action =
            OnConflictAction::DoUpdate(Box::new(OnConflictUpdate {
                assignments: Vec::new(),
                condition: None,
            }));
        self
    }

    pub fn set_conflict<T, V>(mut self, column: Column<M, T>, value: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        let db_type = column.db_type();
        let column_ref = column.column_ref();
        self = self.set_conflict_expr(column_ref, Expr::value(value.into_field_value(db_type)));
        self
    }

    pub fn set_conflict_expr(
        mut self,
        column: impl Into<super::ast::ColumnRef>,
        expr: impl Into<Expr>,
    ) -> Self {
        if let Some(OnConflictNode {
            action: OnConflictAction::DoUpdate(conflict),
            ..
        }) = &mut self.on_conflict
        {
            conflict.assignments.push((column.into(), expr.into()));
        }
        self
    }

    pub fn set_excluded<T>(mut self, column: Column<M, T>) -> Self {
        let column_ref = column.column_ref();
        self = self.set_conflict_expr(column_ref.clone(), Expr::excluded(column_ref));
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        if let Some(OnConflictNode {
            action: OnConflictAction::DoUpdate(conflict),
            ..
        }) = &mut self.on_conflict
        {
            conflict.condition = merge_condition(conflict.condition.take(), condition);
        }
        self
    }

    fn ast(&self, returning_all: bool) -> QueryAst {
        QueryAst::insert(InsertNode {
            into: self.table.table_ref(),
            source: InsertSource::Values(self.rows.clone()),
            on_conflict: self.on_conflict.clone(),
            returning: if returning_all {
                self.table.all_select_items()
            } else {
                Vec::new()
            },
        })
    }

    fn validate_rows(&self) -> Result<()> {
        if self.rows.len() != 1 {
            return Err(Error::message(
                "create() expects exactly one row; use create_many() for bulk inserts",
            ));
        }

        if self.rows[0].is_empty() {
            return Err(Error::message(
                "create() requires at least one assigned column before save() or execute()",
            ));
        }

        Ok(())
    }

    fn compiled_sql(&self, returning_all: bool) -> Result<super::compiler::CompiledSql> {
        self.validate_rows()?;
        PostgresCompiler::compile(&self.ast(returning_all))
    }

    pub async fn execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: ModelWriteExecutor,
    {
        Ok(create_model_records(self, executor).await?.len() as u64)
    }

    pub async fn save<E>(&self, executor: &E) -> Result<M>
    where
        E: ModelWriteExecutor,
    {
        let records = self.get(executor).await?;
        let len = records.len();
        let mut records = records.into_iter();
        match len {
            1 => records
                .next()
                .ok_or_else(|| Error::message("create() did not return a record")),
            0 => Err(Error::message("create() did not return a record")),
            _ => Err(Error::message(
                "create() returned more than one record; use get() instead",
            )),
        }
    }

    pub async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: ModelWriteExecutor,
    {
        create_model_records(self, executor)
            .await
            .map(Collection::from)
    }

    pub async fn first<E>(&self, executor: &E) -> Result<Option<M>>
    where
        E: ModelWriteExecutor,
    {
        Ok(self.get(executor).await?.into_iter().next())
    }

    pub fn to_compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.compiled_sql(true)
    }

    pub async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            false,
            self.options.clone(),
        )
        .await
    }

    pub async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            true,
            self.options.clone(),
        )
        .await
    }
}

impl<M> CreateManyModel<M>
where
    M: Model,
{
    pub(crate) fn new(table: &'static TableMeta<M>) -> Self {
        Self {
            table,
            rows: Vec::new(),
            on_conflict: None,
            without_lifecycle: false,
            options: QueryExecutionOptions::default(),
            row_builder_error: None,
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.options.label = Some(label.into());
        self
    }

    /// Skip model lifecycle hooks and framework lifecycle events for this bulk insert.
    ///
    /// Built-in model conventions, write mutators, validation, and audit recording still apply.
    /// When auditing is inactive this enables Foundry's single-statement insert path.
    pub fn without_lifecycle(mut self) -> Self {
        self.without_lifecycle = true;
        self
    }

    pub fn row<F>(mut self, build: F) -> Self
    where
        F: FnOnce(CreateRow<M>) -> CreateRow<M>,
    {
        match catch_sync_panic(|| build(CreateRow::new())) {
            Ok(row) => self.rows.push(row.into_values()),
            Err(panic) => {
                if self.row_builder_error.is_none() {
                    self.row_builder_error =
                        Some(create_many_row_builder_panic_message::<M>(panic));
                }
            }
        }
        self
    }

    pub fn on_conflict_columns<I, C>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<super::ast::ColumnRef>,
    {
        self.on_conflict = Some(OnConflictNode {
            target: Some(OnConflictTarget::Columns(
                columns.into_iter().map(Into::into).collect(),
            )),
            action: current_conflict_action(self.on_conflict.take()),
        });
        self
    }

    pub fn on_conflict_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.on_conflict = Some(OnConflictNode {
            target: Some(OnConflictTarget::Constraint(constraint.into())),
            action: current_conflict_action(self.on_conflict.take()),
        });
        self
    }

    pub fn do_nothing(mut self) -> Self {
        upsert_node_model(&mut self.on_conflict).action = OnConflictAction::DoNothing;
        self
    }

    pub fn do_update(mut self) -> Self {
        upsert_node_model(&mut self.on_conflict).action =
            OnConflictAction::DoUpdate(Box::new(OnConflictUpdate {
                assignments: Vec::new(),
                condition: None,
            }));
        self
    }

    pub fn set_conflict<T, V>(mut self, column: Column<M, T>, value: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        let db_type = column.db_type();
        let column_ref = column.column_ref();
        self = self.set_conflict_expr(column_ref, Expr::value(value.into_field_value(db_type)));
        self
    }

    pub fn set_conflict_expr(
        mut self,
        column: impl Into<super::ast::ColumnRef>,
        expr: impl Into<Expr>,
    ) -> Self {
        if let Some(OnConflictNode {
            action: OnConflictAction::DoUpdate(conflict),
            ..
        }) = &mut self.on_conflict
        {
            conflict.assignments.push((column.into(), expr.into()));
        }
        self
    }

    pub fn set_excluded<T>(mut self, column: Column<M, T>) -> Self {
        let column_ref = column.column_ref();
        self = self.set_conflict_expr(column_ref.clone(), Expr::excluded(column_ref));
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        if let Some(OnConflictNode {
            action: OnConflictAction::DoUpdate(conflict),
            ..
        }) = &mut self.on_conflict
        {
            conflict.condition = merge_condition(conflict.condition.take(), condition);
        }
        self
    }

    fn ast(&self, returning_all: bool) -> QueryAst {
        QueryAst::insert(InsertNode {
            into: self.table.table_ref(),
            source: InsertSource::Values(self.rows.clone()),
            on_conflict: self.on_conflict.clone(),
            returning: if returning_all {
                self.table.all_select_items()
            } else {
                Vec::new()
            },
        })
    }

    fn validate_rows(&self) -> Result<()> {
        if let Some(error) = &self.row_builder_error {
            return Err(Error::message(error.clone()));
        }

        if self.rows.is_empty() {
            return Err(Error::message(
                "create_many() requires at least one row before execute() or get()",
            ));
        }

        if self.rows.iter().any(Vec::is_empty) {
            return Err(Error::message(
                "create_many() does not allow empty rows; each row needs at least one assigned column",
            ));
        }

        Ok(())
    }

    fn compiled_sql(&self, returning_all: bool) -> Result<super::compiler::CompiledSql> {
        self.validate_rows()?;
        PostgresCompiler::compile(&self.ast(returning_all))
    }

    async fn fast_execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: ModelWriteExecutor,
    {
        let create_many = self.clone();
        with_model_write_transaction(executor, |app, transaction, origin, _after_commit| {
            Box::pin(async move {
                let prepared =
                    prepare_create_many_for_execution(&create_many, app, transaction, origin)
                        .await?;
                transaction
                    .execute_compiled_with(
                        &prepared.compiled_sql(false)?,
                        create_many.options.clone(),
                    )
                    .await
            })
        })
        .await
    }

    async fn fast_get<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: ModelWriteExecutor,
    {
        let create_many = self.clone();
        with_model_write_transaction(executor, |app, transaction, origin, _after_commit| {
            Box::pin(async move {
                let prepared =
                    prepare_create_many_for_execution(&create_many, app, transaction, origin)
                        .await?;
                let records = transaction
                    .query_records_with(&prepared.compiled_sql(true)?, create_many.options.clone())
                    .await?;
                records
                    .iter()
                    .map(|record| prepared.table.hydrate_record(record))
                    .collect()
            })
        })
        .await
    }

    pub async fn execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: ModelWriteExecutor,
    {
        if self.without_lifecycle {
            if executor.app_context().audit()?.active_for::<M>() {
                Ok(create_many_model_records_without_lifecycle(self, executor)
                    .await?
                    .len() as u64)
            } else {
                self.fast_execute(executor).await
            }
        } else {
            Ok(create_many_model_records(self, executor).await?.len() as u64)
        }
    }

    pub async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: ModelWriteExecutor,
    {
        if self.without_lifecycle {
            if executor.app_context().audit()?.active_for::<M>() {
                create_many_model_records_without_lifecycle(self, executor)
                    .await
                    .map(Collection::from)
            } else {
                self.fast_get(executor).await
            }
        } else {
            create_many_model_records(self, executor)
                .await
                .map(Collection::from)
        }
    }

    pub async fn first<E>(&self, executor: &E) -> Result<Option<M>>
    where
        E: ModelWriteExecutor,
    {
        Ok(self.get(executor).await?.into_iter().next())
    }

    pub fn to_compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.compiled_sql(true)
    }

    pub async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            false,
            self.options.clone(),
        )
        .await
    }

    pub async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            true,
            self.options.clone(),
        )
        .await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SystemUpdateAction {
    None,
    Restore,
}

#[derive(Clone)]
pub struct UpdateModel<M: 'static> {
    table: &'static TableMeta<M>,
    values: Vec<(super::ast::ColumnRef, Expr)>,
    from: Vec<FromItem>,
    condition: Option<Condition>,
    allow_all: bool,
    without_lifecycle: bool,
    system_action: SystemUpdateAction,
    options: QueryExecutionOptions,
}

pub type RestoreModel<M> = UpdateModel<M>;

impl<M> UpdateModel<M>
where
    M: Model,
{
    pub(crate) fn new(table: &'static TableMeta<M>) -> Self {
        Self {
            table,
            values: Vec::new(),
            from: Vec::new(),
            condition: None,
            allow_all: false,
            without_lifecycle: false,
            system_action: SystemUpdateAction::None,
            options: QueryExecutionOptions::default(),
        }
    }

    pub(crate) fn new_restore(table: &'static TableMeta<M>) -> Self {
        Self {
            table,
            values: Vec::new(),
            from: Vec::new(),
            condition: None,
            allow_all: false,
            without_lifecycle: false,
            system_action: SystemUpdateAction::Restore,
            options: QueryExecutionOptions::default(),
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.options.label = Some(label.into());
        self
    }

    pub fn set<T, V>(mut self, column: Column<M, T>, value: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        self.values.push((
            column.column_ref(),
            Expr::value(value.into_field_value(column.db_type())),
        ));
        self
    }

    pub fn set_expr<T>(mut self, column: Column<M, T>, expr: impl Into<Expr>) -> Self {
        self.values.push((column.column_ref(), expr.into()));
        self
    }

    pub fn set_null<T>(mut self, column: Column<M, T>) -> Self {
        self.values.push((
            column.column_ref(),
            Expr::value(super::ast::DbValue::Null(column.db_type())),
        ));
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        self.condition = merge_condition(self.condition.take(), condition);
        self
    }

    pub fn from(mut self, source: impl Into<FromItem>) -> Self {
        self.from.push(source.into());
        self
    }

    pub fn allow_all(mut self) -> Self {
        self.allow_all = true;
        self
    }

    /// Skip model lifecycle hooks and framework lifecycle events for this update.
    ///
    /// Built-in model conventions, write mutators, validation, and audit recording still apply.
    /// When auditing is inactive this enables Foundry's single-statement update path.
    pub fn without_lifecycle(mut self) -> Self {
        self.without_lifecycle = true;
        self
    }

    fn ast(&self, returning_all: bool) -> QueryAst {
        QueryAst::update(UpdateNode {
            table: self.table.table_ref(),
            values: self.values_for_ast(),
            from: self.from.clone(),
            condition: self.condition.clone(),
            returning: if returning_all {
                self.table.all_select_items()
            } else {
                Vec::new()
            },
        })
    }

    fn validate(&self) -> Result<()> {
        if self.values.is_empty() && self.system_action == SystemUpdateAction::None {
            return Err(Error::message(
                "update() requires at least one assigned column before save() or execute()",
            ));
        }

        if self.condition.is_none() && !self.allow_all {
            return Err(Error::message(
                "update() requires a where clause; call allow_all() to update every row explicitly",
            ));
        }

        Ok(())
    }

    fn values_for_ast(&self) -> Vec<(super::ast::ColumnRef, Expr)> {
        let mut values = self.values.clone();
        if self.system_action == SystemUpdateAction::Restore {
            let _ = apply_restore_assignments(self.table, &mut values);
        }
        values
    }

    fn compiled_sql(&self, returning_all: bool) -> Result<super::compiler::CompiledSql> {
        self.validate()?;
        PostgresCompiler::compile(&self.ast(returning_all))
    }

    async fn fast_execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: ModelWriteExecutor,
    {
        let update = self.clone();
        with_model_write_transaction(executor, |app, transaction, origin, _after_commit| {
            Box::pin(async move {
                let prepared =
                    prepare_update_for_execution(&update, app, transaction, origin).await?;
                transaction
                    .execute_compiled_with(&prepared.compiled_sql(false)?, update.options.clone())
                    .await
            })
        })
        .await
    }

    async fn fast_get<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: ModelWriteExecutor,
    {
        let update = self.clone();
        with_model_write_transaction(executor, |app, transaction, origin, _after_commit| {
            Box::pin(async move {
                let prepared =
                    prepare_update_for_execution(&update, app, transaction, origin).await?;
                let records = transaction
                    .query_records_with(&prepared.compiled_sql(true)?, update.options.clone())
                    .await?;
                records
                    .iter()
                    .map(|record| prepared.table.hydrate_record(record))
                    .collect()
            })
        })
        .await
    }

    pub async fn execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: ModelWriteExecutor,
    {
        if self.without_lifecycle {
            if executor.app_context().audit()?.active_for::<M>() {
                Ok(update_model_records_without_lifecycle(self, executor)
                    .await?
                    .len() as u64)
            } else {
                self.fast_execute(executor).await
            }
        } else {
            Ok(update_model_records(self, executor).await?.len() as u64)
        }
    }

    pub async fn save<E>(&self, executor: &E) -> Result<M>
    where
        E: ModelWriteExecutor,
    {
        let records = self.get(executor).await?;
        let len = records.len();
        let mut records = records.into_iter();
        match len {
            1 => records
                .next()
                .ok_or_else(|| Error::message("update() did not return a record")),
            0 => Err(Error::message("update() did not return a record")),
            _ => Err(Error::message(
                "update() returned more than one record; use get() instead",
            )),
        }
    }

    pub async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: ModelWriteExecutor,
    {
        if self.without_lifecycle {
            if executor.app_context().audit()?.active_for::<M>() {
                update_model_records_without_lifecycle(self, executor)
                    .await
                    .map(Collection::from)
            } else {
                self.fast_get(executor).await
            }
        } else {
            update_model_records(self, executor)
                .await
                .map(Collection::from)
        }
    }

    pub async fn first<E>(&self, executor: &E) -> Result<Option<M>>
    where
        E: ModelWriteExecutor,
    {
        Ok(self.get(executor).await?.into_iter().next())
    }

    pub fn to_compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.compiled_sql(true)
    }

    pub async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            false,
            self.options.clone(),
        )
        .await
    }

    pub async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            true,
            self.options.clone(),
        )
        .await
    }
}

#[derive(Clone)]
pub struct DeleteModel<M: 'static> {
    table: &'static TableMeta<M>,
    using: Vec<FromItem>,
    condition: Option<Condition>,
    allow_all: bool,
    without_lifecycle: bool,
    force_delete: bool,
    options: QueryExecutionOptions,
}

impl<M> DeleteModel<M>
where
    M: Model,
{
    pub(crate) fn new(table: &'static TableMeta<M>) -> Self {
        Self {
            table,
            using: Vec::new(),
            condition: None,
            allow_all: false,
            without_lifecycle: false,
            force_delete: false,
            options: QueryExecutionOptions::default(),
        }
    }

    pub(crate) fn new_force(table: &'static TableMeta<M>) -> Self {
        Self {
            table,
            using: Vec::new(),
            condition: None,
            allow_all: false,
            without_lifecycle: false,
            force_delete: true,
            options: QueryExecutionOptions::default(),
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.options.label = Some(label.into());
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        self.condition = merge_condition(self.condition.take(), condition);
        self
    }

    pub fn using(mut self, source: impl Into<FromItem>) -> Self {
        self.using.push(source.into());
        self
    }

    pub fn allow_all(mut self) -> Self {
        self.allow_all = true;
        self
    }

    /// Skip model lifecycle hooks and framework lifecycle events for this delete.
    ///
    /// Built-in soft-delete conventions, validation, and audit recording still apply. When auditing
    /// is inactive this enables Foundry's direct delete or soft-delete update path.
    pub fn without_lifecycle(mut self) -> Self {
        self.without_lifecycle = true;
        self
    }

    fn ast(&self) -> QueryAst {
        QueryAst::delete(super::ast::DeleteNode {
            from: self.table.table_ref(),
            using: self.using.clone(),
            condition: self.condition.clone(),
            returning: Vec::new(),
        })
    }

    fn validate(&self) -> Result<()> {
        if self.condition.is_none() && !self.allow_all {
            return Err(Error::message(
                "delete() requires a where clause; call allow_all() to delete every row explicitly",
            ));
        }

        Ok(())
    }

    fn compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.validate()?;
        PostgresCompiler::compile(&self.ast())
    }

    async fn fast_execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: ModelWriteExecutor,
    {
        if self.table.soft_deletes_enabled() && !self.force_delete {
            let update = soft_delete_update(self, executor.app_context())?;
            return executor
                .execute_compiled_with(&update.compiled_sql(false)?, self.options.clone())
                .await;
        }

        executor
            .execute_compiled_with(&self.compiled_sql()?, self.options.clone())
            .await
    }

    pub async fn execute<E>(&self, executor: &E) -> Result<u64>
    where
        E: ModelWriteExecutor,
    {
        if self.without_lifecycle {
            if executor.app_context().audit()?.active_for::<M>() {
                delete_model_rows_without_lifecycle(self, executor).await
            } else {
                self.fast_execute(executor).await
            }
        } else {
            delete_model_rows(self, executor).await
        }
    }

    pub fn to_compiled_sql(&self) -> Result<super::compiler::CompiledSql> {
        self.compiled_sql()
    }

    pub async fn explain<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            false,
            self.options.clone(),
        )
        .await
    }

    pub async fn explain_analyze<E>(&self, executor: &E) -> Result<Vec<String>>
    where
        E: QueryExecutor,
    {
        explain_query(
            executor,
            &self.to_compiled_sql()?,
            true,
            self.options.clone(),
        )
        .await
    }
}

type ModelWriteFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

async fn with_model_write_transaction<E, T>(
    executor: &E,
    operation: impl for<'a> FnOnce(
        &'a AppContext,
        &'a super::runtime::DatabaseTransaction,
        Option<EventOrigin>,
        &'a dyn AfterCommitSink,
    ) -> ModelWriteFuture<'a, T>,
) -> Result<T>
where
    E: ModelWriteExecutor,
{
    let actor = executor.actor().cloned().or_else(current_actor);
    let origin = event_origin_from_context(actor, current_request());

    if let Some(transaction) = executor.active_transaction() {
        return operation(executor.app_context(), transaction, origin, executor).await;
    }

    let transaction = executor.app_context().begin_transaction().await?;
    let result = operation(
        transaction.app(),
        transaction.transaction(),
        origin,
        &transaction,
    )
    .await;
    match result {
        Ok(value) => {
            transaction.commit().await?;
            Ok(value)
        }
        Err(error) => {
            let rollback_result = transaction.rollback().await;
            if let Err(rollback_error) = rollback_result {
                return Err(Error::message(format!(
                    "{error}; rollback failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}

async fn dispatch_post_commit_model_event<E>(
    app: &AppContext,
    after_commit: &dyn AfterCommitSink,
    origin: Option<EventOrigin>,
    event: E,
) -> Result<()>
where
    E: Event,
{
    if after_commit.supports_after_commit() {
        after_commit.defer_after_commit(Box::new(move |app| {
            Box::pin(async move { app.events()?.dispatch_with_origin(event, origin).await })
        }));
        return Ok(());
    }

    app.events()?.dispatch_with_origin(event, origin).await
}

fn event_origin_from_context(
    actor: Option<crate::auth::Actor>,
    request: Option<crate::logging::CurrentRequest>,
) -> Option<EventOrigin> {
    EventOrigin::from_request(actor, request.as_ref())
}

async fn apply_assignment_write_mutators<M>(
    table: &'static TableMeta<M>,
    context: &ModelHookContext<'_>,
    values: &mut [(ColumnRef, Expr)],
) -> Result<()>
where
    M: Model,
{
    for (column, expr) in values.iter_mut() {
        let belongs_to_table = column.table.as_deref() == Some(table.name());
        if !belongs_to_table {
            continue;
        }

        let Some(column_info) = table.column_info(&column.name) else {
            continue;
        };
        let Some(write_mutator) = column_info.write_mutator() else {
            continue;
        };

        match expr {
            Expr::Value(value) => {
                let transformed =
                    run_model_write_mutator::<M>(column, write_mutator, context, value.clone())
                        .await?;
                *expr = Expr::value(transformed);
            }
            Expr::Excluded(_) => {}
            _ => {
                return Err(Error::message(format!(
                    "field `{}` on `{}` uses a write mutator and only supports literal values or EXCLUDED assignments in model write APIs",
                    column.name,
                    table.name()
                )));
            }
        }
    }

    Ok(())
}

async fn run_model_write_mutator<M>(
    column: &ColumnRef,
    write_mutator: ModelFieldWriteMutator,
    context: &ModelHookContext<'_>,
    value: DbValue,
) -> Result<DbValue>
where
    M: Model,
{
    match catch_async_panic(|| write_mutator(context, value)).await {
        Ok(result) => result,
        Err(panic) => Err(model_write_mutator_panic_error::<M>(&column.name, panic)),
    }
}

fn model_write_mutator_panic_error<M>(column: &str, panic: Box<dyn std::any::Any + Send>) -> Error
where
    M: Model,
{
    let model = std::any::type_name::<M>();
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        model = model,
        column = column,
        panic = %message,
        "model write mutator panicked"
    );
    Error::message(format!(
        "model `{model}` write mutator `{column}` panicked: {message}"
    ))
}

async fn run_query_iteration_callback<M, I, F, Fut>(
    operation: &'static str,
    handler: &mut F,
    input: I,
) -> Result<()>
where
    M: Model,
    F: FnMut(I) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    match catch_async_panic(|| handler(input)).await {
        Ok(result) => result,
        Err(panic) => Err(query_iteration_panic_error::<M>(operation, panic)),
    }
}

fn query_iteration_panic_error<M>(
    operation: &'static str,
    panic: Box<dyn std::any::Any + Send>,
) -> Error
where
    M: Model,
{
    let model = std::any::type_name::<M>();
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        model = model,
        operation = operation,
        panic = %message,
        "model query iteration callback panicked"
    );
    Error::message(format!(
        "model query `{model}` {operation} callback panicked: {message}"
    ))
}

fn first_deferred_error(left: Option<String>, right: Option<String>) -> Option<String> {
    left.or(right)
}

fn model_query_dsl_panic_message<M>(
    operation: &'static str,
    panic: Box<dyn std::any::Any + Send>,
) -> String
where
    M: Model,
{
    let model = std::any::type_name::<M>();
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        model = model,
        operation = operation,
        panic = %message,
        "model query DSL callback panicked"
    );
    format!("model query `{model}` {operation} callback panicked: {message}")
}

fn projection_query_dsl_panic_message<P>(
    operation: &'static str,
    panic: Box<dyn std::any::Any + Send>,
) -> String {
    let projection = std::any::type_name::<P>();
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        projection = projection,
        operation = operation,
        panic = %message,
        "projection query DSL callback panicked"
    );
    format!("projection query `{projection}` {operation} callback panicked: {message}")
}

fn create_many_row_builder_panic_message<M>(panic: Box<dyn std::any::Any + Send>) -> String
where
    M: Model,
{
    let model = std::any::type_name::<M>();
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        model = model,
        panic = %message,
        "create_many row builder panicked"
    );
    format!("model create_many `{model}` row callback panicked: {message}")
}

async fn apply_create_write_mutators<M>(
    create: &CreateModel<M>,
    context: &ModelHookContext<'_>,
    values: &mut [(ColumnRef, Expr)],
    on_conflict: &mut Option<OnConflictNode>,
) -> Result<()>
where
    M: Model,
{
    apply_assignment_write_mutators(create.table, context, values).await?;

    if let Some(OnConflictNode {
        action: OnConflictAction::DoUpdate(conflict),
        ..
    }) = on_conflict
    {
        apply_assignment_write_mutators(create.table, context, &mut conflict.assignments).await?;
    }

    Ok(())
}

async fn apply_update_write_mutators<M>(
    table: &'static TableMeta<M>,
    context: &ModelHookContext<'_>,
    values: &mut [(ColumnRef, Expr)],
) -> Result<()>
where
    M: Model,
{
    apply_assignment_write_mutators(table, context, values).await
}

async fn create_model_records<E, M>(create: &CreateModel<M>, executor: &E) -> Result<Vec<M>>
where
    E: ModelWriteExecutor,
    M: Model,
{
    create.validate_rows()?;
    let create = create.clone();
    with_model_write_transaction(executor, |app, transaction, origin, after_commit| {
        Box::pin(async move {
            create_model_records_in_transaction_inner(
                &create,
                app,
                transaction,
                origin,
                after_commit,
                LifecycleMode::Enabled,
            )
            .await
        })
    })
    .await
}

async fn create_many_model_records_without_lifecycle<E, M>(
    create_many: &CreateManyModel<M>,
    executor: &E,
) -> Result<Vec<M>>
where
    E: ModelWriteExecutor,
    M: Model,
{
    create_many.validate_rows()?;
    let create_many = create_many.clone();
    with_model_write_transaction(executor, |app, transaction, origin, after_commit| {
        Box::pin(async move {
            let mut created = Vec::new();
            for row in &create_many.rows {
                let create = CreateModel {
                    table: create_many.table,
                    rows: vec![row.clone()],
                    on_conflict: create_many.on_conflict.clone(),
                    options: create_many.options.clone(),
                };
                created.extend(
                    create_model_records_in_transaction_inner(
                        &create,
                        app,
                        transaction,
                        origin.clone(),
                        after_commit,
                        LifecycleMode::Disabled,
                    )
                    .await?,
                );
            }
            Ok(created)
        })
    })
    .await
}

async fn create_many_model_records<E, M>(
    create_many: &CreateManyModel<M>,
    executor: &E,
) -> Result<Vec<M>>
where
    E: ModelWriteExecutor,
    M: Model,
{
    create_many.validate_rows()?;
    let create_many = create_many.clone();
    with_model_write_transaction(executor, |app, transaction, origin, after_commit| {
        Box::pin(async move {
            let mut created = Vec::new();
            for row in &create_many.rows {
                let create = CreateModel {
                    table: create_many.table,
                    rows: vec![row.clone()],
                    on_conflict: create_many.on_conflict.clone(),
                    options: create_many.options.clone(),
                };
                created.extend(
                    create_model_records_in_transaction_inner(
                        &create,
                        app,
                        transaction,
                        origin.clone(),
                        after_commit,
                        LifecycleMode::Enabled,
                    )
                    .await?,
                );
            }
            Ok(created)
        })
    })
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecycleMode {
    Enabled,
    Disabled,
}

impl LifecycleMode {
    fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

async fn run_model_lifecycle_hook<M, F, Fut>(hook: &'static str, run: F) -> Result<()>
where
    M: Model,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    match catch_async_panic(run).await {
        Ok(result) => result,
        Err(panic) => Err(model_lifecycle_panic_error::<M>(hook, panic)),
    }
}

fn model_lifecycle_panic_error<M>(hook: &'static str, panic: Box<dyn std::any::Any + Send>) -> Error
where
    M: Model,
{
    let model = std::any::type_name::<M>();
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        model = model,
        hook = hook,
        panic = %message,
        "model lifecycle hook panicked"
    );
    Error::message(format!(
        "model lifecycle `{model}` {hook} hook panicked: {message}"
    ))
}

async fn create_model_records_in_transaction_inner<M>(
    create: &CreateModel<M>,
    app: &AppContext,
    transaction: &super::runtime::DatabaseTransaction,
    origin: Option<EventOrigin>,
    after_commit: &dyn AfterCommitSink,
    lifecycle_mode: LifecycleMode,
) -> Result<Vec<M>>
where
    M: Model,
{
    let database = app.database()?;
    let mut values = create.rows[0].clone();
    apply_create_model_conventions(create.table, app, &mut values)?;
    let mut draft = CreateDraft::<M>::new(values);
    let context = ModelHookContext::new(app, database, transaction, origin.clone());
    if lifecycle_mode.enabled() {
        run_model_lifecycle_hook::<M, _, _>("creating", || {
            M::Lifecycle::creating(&context, &mut draft)
        })
        .await?;
    }
    let mut on_conflict = create.on_conflict.clone();
    let mut values = draft.into_values();
    apply_create_write_mutators(create, &context, &mut values, &mut on_conflict).await?;
    if lifecycle_mode.enabled() {
        context
            .dispatch(ModelCreatingEvent {
                snapshot: ModelLifecycleSnapshot::for_model::<M>(
                    None,
                    None,
                    Some(CreateDraft::<M>::new(values.clone()).pending_record()),
                ),
            })
            .await?;
    }

    let records = transaction
        .query_records_with(
            &CreateModel {
                table: create.table,
                rows: vec![values],
                on_conflict,
                options: create.options.clone(),
            }
            .compiled_sql(true)?,
            create.options.clone(),
        )
        .await?;

    let mut models = Vec::with_capacity(records.len());
    for record in &records {
        let model = create.table.hydrate_record(record)?;
        if lifecycle_mode.enabled() {
            run_model_lifecycle_hook::<M, _, _>("created", || {
                M::Lifecycle::created(&context, &model, record)
            })
            .await?;
        }
        write_model_audit::<M>(&context, AuditEventType::Created, None, Some(record)).await?;
        if lifecycle_mode.enabled() {
            dispatch_post_commit_model_event(
                app,
                after_commit,
                origin.clone(),
                ModelCreatedEvent {
                    snapshot: ModelLifecycleSnapshot::for_model::<M>(
                        None,
                        Some(record.clone()),
                        None,
                    ),
                },
            )
            .await?;
        }
        models.push(model);
    }

    Ok(models)
}

async fn update_model_records<E, M>(update: &UpdateModel<M>, executor: &E) -> Result<Vec<M>>
where
    E: ModelWriteExecutor,
    M: Model,
{
    update.validate()?;
    let update = update.clone();
    with_model_write_transaction(executor, |app, transaction, origin, after_commit| {
        Box::pin(async move {
            update_model_records_in_transaction(&update, app, transaction, origin, after_commit)
                .await
        })
    })
    .await
}

async fn update_model_records_without_lifecycle<E, M>(
    update: &UpdateModel<M>,
    executor: &E,
) -> Result<Vec<M>>
where
    E: ModelWriteExecutor,
    M: Model,
{
    update.validate()?;
    let update = update.clone();
    with_model_write_transaction(executor, |app, transaction, origin, after_commit| {
        Box::pin(async move {
            update_model_records_in_transaction_inner(
                &update,
                app,
                transaction,
                origin,
                after_commit,
                LifecycleMode::Disabled,
            )
            .await
        })
    })
    .await
}

async fn update_model_records_in_transaction<M>(
    update: &UpdateModel<M>,
    app: &AppContext,
    transaction: &super::runtime::DatabaseTransaction,
    origin: Option<EventOrigin>,
    after_commit: &dyn AfterCommitSink,
) -> Result<Vec<M>>
where
    M: Model,
{
    update_model_records_in_transaction_inner(
        update,
        app,
        transaction,
        origin,
        after_commit,
        LifecycleMode::Enabled,
    )
    .await
}

async fn update_model_records_in_transaction_inner<M>(
    update: &UpdateModel<M>,
    app: &AppContext,
    transaction: &super::runtime::DatabaseTransaction,
    origin: Option<EventOrigin>,
    after_commit: &dyn AfterCommitSink,
    lifecycle_mode: LifecycleMode,
) -> Result<Vec<M>>
where
    M: Model,
{
    let current_records = select_update_target_records(update, transaction).await?;
    let database = app.database()?;
    let context = ModelHookContext::new(app, database, transaction, origin.clone());
    let mut updated_models = Vec::with_capacity(current_records.len());
    let audit_event_type = audit_event_type_for_update(update.system_action);

    for current_record in current_records {
        let current_model = update.table.hydrate_record(&current_record)?;
        let mut values = update.values.clone();
        apply_update_model_conventions(update.table, app, &mut values, update.system_action)?;
        let mut draft = UpdateDraft::<M>::new(values);
        if lifecycle_mode.enabled() {
            run_model_lifecycle_hook::<M, _, _>("updating", || {
                M::Lifecycle::updating(&context, &current_model, &mut draft)
            })
            .await?;
        }
        let mut values = draft.into_values();
        apply_update_write_mutators(update.table, &context, &mut values).await?;
        if lifecycle_mode.enabled() {
            context
                .dispatch(ModelUpdatingEvent {
                    snapshot: ModelLifecycleSnapshot::for_model::<M>(
                        Some(current_record.clone()),
                        None,
                        Some(UpdateDraft::<M>::new(values.clone()).pending_record()),
                    ),
                })
                .await?;
        }

        let pk_condition = record_primary_key_condition(update.table, &current_record)?;
        let records = transaction
            .query_records_with(
                &UpdateModel {
                    table: update.table,
                    values,
                    from: update.from.clone(),
                    condition: merge_optional_condition(
                        Some(pk_condition),
                        update.condition.clone(),
                    ),
                    allow_all: false,
                    without_lifecycle: false,
                    system_action: SystemUpdateAction::None,
                    options: update.options.clone(),
                }
                .compiled_sql(true)?,
                update.options.clone(),
            )
            .await?;

        let after_record = expect_single_record("update()", records)?;
        let after_model = update.table.hydrate_record(&after_record)?;
        if lifecycle_mode.enabled() {
            run_model_lifecycle_hook::<M, _, _>("updated", || {
                M::Lifecycle::updated(
                    &context,
                    &current_model,
                    &after_model,
                    &current_record,
                    &after_record,
                )
            })
            .await?;
        }
        write_model_audit::<M>(
            &context,
            audit_event_type,
            Some(&current_record),
            Some(&after_record),
        )
        .await?;
        if lifecycle_mode.enabled() {
            dispatch_post_commit_model_event(
                app,
                after_commit,
                origin.clone(),
                ModelUpdatedEvent {
                    snapshot: ModelLifecycleSnapshot::for_model::<M>(
                        Some(current_record),
                        Some(after_record.clone()),
                        None,
                    ),
                },
            )
            .await?;
        }
        updated_models.push(after_model);
    }

    Ok(updated_models)
}

async fn delete_model_rows<E, M>(delete: &DeleteModel<M>, executor: &E) -> Result<u64>
where
    E: ModelWriteExecutor,
    M: Model,
{
    delete.validate()?;
    let delete = delete.clone();
    with_model_write_transaction(executor, |app, transaction, origin, after_commit| {
        Box::pin(async move {
            delete_model_rows_in_transaction(&delete, app, transaction, origin, after_commit).await
        })
    })
    .await
}

async fn delete_model_rows_without_lifecycle<E, M>(
    delete: &DeleteModel<M>,
    executor: &E,
) -> Result<u64>
where
    E: ModelWriteExecutor,
    M: Model,
{
    delete.validate()?;
    let delete = delete.clone();
    with_model_write_transaction(executor, |app, transaction, origin, after_commit| {
        Box::pin(async move {
            delete_model_rows_in_transaction_inner(
                &delete,
                app,
                transaction,
                origin,
                after_commit,
                LifecycleMode::Disabled,
            )
            .await
        })
    })
    .await
}

async fn delete_model_rows_in_transaction<M>(
    delete: &DeleteModel<M>,
    app: &AppContext,
    transaction: &super::runtime::DatabaseTransaction,
    origin: Option<EventOrigin>,
    after_commit: &dyn AfterCommitSink,
) -> Result<u64>
where
    M: Model,
{
    delete_model_rows_in_transaction_inner(
        delete,
        app,
        transaction,
        origin,
        after_commit,
        LifecycleMode::Enabled,
    )
    .await
}

async fn delete_model_rows_in_transaction_inner<M>(
    delete: &DeleteModel<M>,
    app: &AppContext,
    transaction: &super::runtime::DatabaseTransaction,
    origin: Option<EventOrigin>,
    after_commit: &dyn AfterCommitSink,
    lifecycle_mode: LifecycleMode,
) -> Result<u64>
where
    M: Model,
{
    let current_records = select_delete_target_records(delete, transaction).await?;
    let database = app.database()?;
    let context = ModelHookContext::new(app, database, transaction, origin.clone());

    for current_record in &current_records {
        let current_model = delete.table.hydrate_record(current_record)?;
        if lifecycle_mode.enabled() {
            run_model_lifecycle_hook::<M, _, _>("deleting", || {
                M::Lifecycle::deleting(&context, &current_model, current_record)
            })
            .await?;
            context
                .dispatch(ModelDeletingEvent {
                    snapshot: ModelLifecycleSnapshot::for_model::<M>(
                        Some(current_record.clone()),
                        None,
                        None,
                    ),
                })
                .await?;
        }

        let pk_condition = record_primary_key_condition(delete.table, current_record)?;
        if delete.table.soft_deletes_enabled() && !delete.force_delete {
            let update = soft_delete_update(
                &DeleteModel {
                    table: delete.table,
                    using: delete.using.clone(),
                    condition: merge_optional_condition(
                        Some(pk_condition),
                        delete.condition.clone(),
                    ),
                    allow_all: false,
                    without_lifecycle: false,
                    force_delete: false,
                    options: delete.options.clone(),
                },
                app,
            )?;
            transaction
                .execute_compiled_with(&update.compiled_sql(false)?, delete.options.clone())
                .await?;
            let after_record = record_with_assignments(current_record, &update.values);
            write_model_audit::<M>(
                &context,
                AuditEventType::SoftDeleted,
                Some(current_record),
                Some(&after_record),
            )
            .await?;
        } else {
            transaction
                .execute_compiled_with(
                    &DeleteModel {
                        table: delete.table,
                        using: delete.using.clone(),
                        condition: merge_optional_condition(
                            Some(pk_condition),
                            delete.condition.clone(),
                        ),
                        allow_all: false,
                        without_lifecycle: false,
                        force_delete: true,
                        options: delete.options.clone(),
                    }
                    .compiled_sql()?,
                    delete.options.clone(),
                )
                .await?;
            write_model_audit::<M>(
                &context,
                AuditEventType::Deleted,
                Some(current_record),
                None,
            )
            .await?;
        }

        if lifecycle_mode.enabled() {
            run_model_lifecycle_hook::<M, _, _>("deleted", || {
                M::Lifecycle::deleted(&context, &current_model, current_record)
            })
            .await?;
            dispatch_post_commit_model_event(
                app,
                after_commit,
                origin.clone(),
                ModelDeletedEvent {
                    snapshot: ModelLifecycleSnapshot::for_model::<M>(
                        Some(current_record.clone()),
                        None,
                        None,
                    ),
                },
            )
            .await?;
        }
    }

    Ok(current_records.len() as u64)
}

async fn select_update_target_records<M>(
    update: &UpdateModel<M>,
    executor: &super::runtime::DatabaseTransaction,
) -> Result<Vec<DbRecord>>
where
    M: Model,
{
    let mut query = Query::table(update.table.table_ref());
    for item in update.table.all_select_items() {
        query = query.select_item(item);
    }
    for from in update.from.clone() {
        query = query.cross_join(from);
    }
    if let Some(condition) = update.condition.clone() {
        query = query.where_(condition);
    }
    query = query
        .order_by(OrderBy::asc(update.table.primary_key_ref()))
        .for_update()
        .of([update.table.name()]);
    query.get(executor).await.map(Collection::into_vec)
}

async fn select_delete_target_records<M>(
    delete: &DeleteModel<M>,
    executor: &super::runtime::DatabaseTransaction,
) -> Result<Vec<DbRecord>>
where
    M: Model,
{
    let mut query = Query::table(delete.table.table_ref());
    for item in delete.table.all_select_items() {
        query = query.select_item(item);
    }
    for using in delete.using.clone() {
        query = query.cross_join(using);
    }
    if let Some(condition) = delete.condition.clone() {
        query = query.where_(condition);
    }
    query = query
        .order_by(OrderBy::asc(delete.table.primary_key_ref()))
        .for_update()
        .of([delete.table.name()]);
    query.get(executor).await.map(Collection::into_vec)
}

fn record_primary_key_condition<M>(table: &TableMeta<M>, record: &DbRecord) -> Result<Condition> {
    let primary_key = table.primary_key_column_info().ok_or_else(|| {
        Error::message(format!(
            "missing primary key column `{}` on table `{}`",
            table.primary_key_name(),
            table.name()
        ))
    })?;
    let value = record.get(primary_key.name).cloned().ok_or_else(|| {
        Error::message(format!(
            "missing primary key `{}` in record",
            primary_key.name
        ))
    })?;
    Ok(Condition::compare(
        Expr::column(ColumnRef::new(table.name(), primary_key.name).typed(primary_key.db_type)),
        ComparisonOp::Eq,
        Expr::value(value),
    ))
}

fn expect_single_record(operation: &str, mut records: Vec<DbRecord>) -> Result<DbRecord> {
    match records.len() {
        1 => Ok(records.remove(0)),
        0 => Err(Error::message(format!(
            "{operation} did not return a record"
        ))),
        _ => Err(Error::message(format!(
            "{operation} returned more than one record unexpectedly"
        ))),
    }
}

async fn prepare_create_many_for_execution<M>(
    create_many: &CreateManyModel<M>,
    app: &AppContext,
    transaction: &super::runtime::DatabaseTransaction,
    origin: Option<EventOrigin>,
) -> Result<CreateManyModel<M>>
where
    M: Model,
{
    let mut prepared = create_many.clone();
    let database = app.database()?;
    let context = ModelHookContext::new(app, database, transaction, origin.clone());
    for row in &mut prepared.rows {
        apply_create_model_conventions(prepared.table, app, row)?;
        apply_assignment_write_mutators(prepared.table, &context, row).await?;
    }
    if let Some(OnConflictNode {
        action: OnConflictAction::DoUpdate(conflict),
        ..
    }) = &mut prepared.on_conflict
    {
        apply_assignment_write_mutators(prepared.table, &context, &mut conflict.assignments)
            .await?;
    }
    Ok(prepared)
}

async fn prepare_update_for_execution<M>(
    update: &UpdateModel<M>,
    app: &AppContext,
    transaction: &super::runtime::DatabaseTransaction,
    origin: Option<EventOrigin>,
) -> Result<UpdateModel<M>>
where
    M: Model,
{
    let mut prepared = update.clone();
    apply_update_model_conventions(
        prepared.table,
        app,
        &mut prepared.values,
        prepared.system_action,
    )?;
    let database = app.database()?;
    let context = ModelHookContext::new(app, database, transaction, origin.clone());
    apply_update_write_mutators(prepared.table, &context, &mut prepared.values).await?;
    prepared.system_action = SystemUpdateAction::None;
    Ok(prepared)
}

fn soft_delete_update<M>(delete: &DeleteModel<M>, app: &AppContext) -> Result<UpdateModel<M>>
where
    M: Model,
{
    let mut values = Vec::new();
    apply_soft_delete_model_conventions(delete.table, app, &mut values)?;
    Ok(UpdateModel {
        table: delete.table,
        values,
        from: delete.using.clone(),
        condition: delete.condition.clone(),
        allow_all: delete.allow_all,
        without_lifecycle: delete.without_lifecycle,
        system_action: SystemUpdateAction::None,
        options: delete.options.clone(),
    })
}

fn apply_create_model_conventions<M>(
    table: &TableMeta<M>,
    app: &AppContext,
    values: &mut Vec<(super::ast::ColumnRef, Expr)>,
) -> Result<()> {
    apply_primary_key_model_conventions(table, values)?;
    if table.timestamps_enabled(app)? {
        let now = app.clock().now();
        if !has_assignment(values, "created_at") {
            upsert_model_value(table, values, "created_at", now.to_db_value())?;
        }
        upsert_model_value(table, values, "updated_at", now.to_db_value())?;
    }
    Ok(())
}

fn apply_primary_key_model_conventions<M>(
    table: &TableMeta<M>,
    values: &mut Vec<(super::ast::ColumnRef, Expr)>,
) -> Result<()> {
    if has_assignment(values, table.primary_key_name()) {
        return Ok(());
    }

    match table.primary_key_strategy() {
        ModelPrimaryKeyStrategy::UuidV7 => upsert_model_value(
            table,
            values,
            table.primary_key_name(),
            DbValue::Uuid(ModelId::<M>::generate().into_uuid()),
        ),
        ModelPrimaryKeyStrategy::Manual => Err(Error::message(format!(
            "create() requires an explicit `{}` assignment for `{}` because its primary_key_strategy is manual",
            table.primary_key_name(),
            table.name()
        ))),
    }
}

fn apply_update_model_conventions<M>(
    table: &TableMeta<M>,
    app: &AppContext,
    values: &mut Vec<(super::ast::ColumnRef, Expr)>,
    action: SystemUpdateAction,
) -> Result<()> {
    if action == SystemUpdateAction::Restore {
        apply_restore_assignments(table, values)?;
    }
    if table.timestamps_enabled(app)? {
        upsert_model_value(table, values, "updated_at", app.clock().now().to_db_value())?;
    }
    Ok(())
}

fn apply_soft_delete_model_conventions<M>(
    table: &TableMeta<M>,
    app: &AppContext,
    values: &mut Vec<(super::ast::ColumnRef, Expr)>,
) -> Result<()> {
    upsert_model_value(table, values, "deleted_at", app.clock().now().to_db_value())?;
    if table.timestamps_enabled(app)? {
        upsert_model_value(table, values, "updated_at", app.clock().now().to_db_value())?;
    }
    Ok(())
}

fn apply_restore_assignments<M>(
    table: &TableMeta<M>,
    values: &mut Vec<(super::ast::ColumnRef, Expr)>,
) -> Result<()> {
    let deleted_at = table.deleted_at_column_info().ok_or_else(|| {
        Error::message(format!(
            "restore() requires a `deleted_at` column on `{}`",
            table.name()
        ))
    })?;
    upsert_assignment(
        values,
        ColumnRef::new(table.name(), deleted_at.name).typed(deleted_at.db_type),
        Expr::value(DbValue::Null(deleted_at.db_type)),
    );
    Ok(())
}

fn has_assignment(values: &[(super::ast::ColumnRef, Expr)], column_name: &str) -> bool {
    values.iter().any(|(column, _)| column.name == column_name)
}

fn audit_event_type_for_update(action: SystemUpdateAction) -> AuditEventType {
    match action {
        SystemUpdateAction::None => AuditEventType::Updated,
        SystemUpdateAction::Restore => AuditEventType::Restored,
    }
}

fn upsert_model_value<M>(
    table: &TableMeta<M>,
    values: &mut Vec<(super::ast::ColumnRef, Expr)>,
    column_name: &str,
    value: DbValue,
) -> Result<()> {
    let column = table.column_info(column_name).ok_or_else(|| {
        Error::message(format!(
            "missing `{column_name}` column on table `{}`",
            table.name()
        ))
    })?;
    upsert_assignment(
        values,
        ColumnRef::new(table.name(), column.name).typed(column.db_type),
        Expr::value(value),
    );
    Ok(())
}

fn merge_condition(existing: Option<Condition>, next: Condition) -> Option<Condition> {
    Some(match existing {
        Some(existing) => Condition::and([existing, next]),
        None => next,
    })
}

fn merge_optional_condition(
    existing: Option<Condition>,
    next: Option<Condition>,
) -> Option<Condition> {
    match next {
        Some(next) => merge_condition(existing, next),
        None => existing,
    }
}

fn ensure_insert_row(
    rows: &mut Vec<Vec<(super::ast::ColumnRef, Expr)>>,
) -> &mut Vec<(super::ast::ColumnRef, Expr)> {
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    let index = rows.len() - 1;
    &mut rows[index]
}

fn push_insert_expr_value(insert: &mut InsertNode, value: (super::ast::ColumnRef, Expr)) {
    match &mut insert.source {
        InsertSource::Values(rows) => {
            if rows.is_empty() {
                rows.push(Vec::new());
            }
            let index = rows.len() - 1;
            rows[index].push(value);
        }
        InsertSource::Select(_) => {
            insert.source = InsertSource::Values(vec![vec![value]]);
        }
    }
}

fn push_insert_expr_row(insert: &mut InsertNode, row: Vec<(super::ast::ColumnRef, Expr)>) {
    match &mut insert.source {
        InsertSource::Values(rows) => rows.push(row),
        InsertSource::Select(_) => {
            insert.source = InsertSource::Values(vec![row]);
        }
    }
}

fn current_conflict_action(existing: Option<OnConflictNode>) -> OnConflictAction {
    existing
        .map(|node| node.action)
        .unwrap_or(OnConflictAction::DoNothing)
}

fn upsert_node(insert: &mut InsertNode) -> &mut OnConflictNode {
    insert.on_conflict.get_or_insert(OnConflictNode {
        target: None,
        action: OnConflictAction::DoNothing,
    })
}

fn upsert_node_model(on_conflict: &mut Option<OnConflictNode>) -> &mut OnConflictNode {
    on_conflict.get_or_insert(OnConflictNode {
        target: None,
        action: OnConflictAction::DoNothing,
    })
}

async fn decode_wrapped_projection<E, T>(
    executor: &E,
    ast: QueryAst,
    projection: AggregateProjection<T>,
) -> Result<T>
where
    E: QueryExecutor,
    T: FromDbValue,
{
    let compiled = PostgresCompiler::compile(&ast)?;
    let record = executor
        .query_records(&compiled)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::message("aggregate query returned no rows"))?;
    projection.decode(&record)
}

async fn explain_query<E>(
    executor: &E,
    compiled: &super::compiler::CompiledSql,
    analyze: bool,
    options: QueryExecutionOptions,
) -> Result<Vec<String>>
where
    E: QueryExecutor + ?Sized,
{
    let sql = if analyze {
        format!("EXPLAIN ANALYZE {}", compiled.sql)
    } else {
        format!("EXPLAIN {}", compiled.sql)
    };
    let records = executor
        .raw_query_with(&sql, &compiled.bindings, options)
        .await?;
    records
        .iter()
        .map(|record| record.decode::<String>("QUERY PLAN"))
        .collect()
}

struct ModelStreamState<'a, M: Model> {
    executor: &'a dyn QueryExecutor,
    root_stream: DbRecordStream<'a>,
    table: &'static TableMeta<M>,
    relations: Vec<AnyRelation<M>>,
    model_extensions: Vec<AnyModelExtension<M>>,
    relation_aggregates: Vec<AnyRelationAggregate<M>>,
    stream_batch_size: usize,
    buffered: VecDeque<Result<M>>,
    pending_error: Option<Error>,
    finished: bool,
    options: QueryExecutionOptions,
}

fn model_query_stream<'a, M>(state: ModelStreamState<'a, M>) -> BoxStream<'a, Result<M>>
where
    M: Model,
{
    stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.buffered.pop_front() {
                return Some((item, state));
            }

            if let Some(error) = state.pending_error.take() {
                return Some((Err(error), state));
            }

            if state.finished {
                return None;
            }

            match fill_model_stream_buffer(&mut state).await {
                Ok(()) => {}
                Err(error) => {
                    state.finished = true;
                    return Some((Err(error), state));
                }
            }
        }
    })
    .boxed()
}

async fn fill_model_stream_buffer<M>(state: &mut ModelStreamState<'_, M>) -> Result<()>
where
    M: Model,
{
    let mut records = Vec::new();

    while records.len() < state.stream_batch_size {
        match state.root_stream.next().await {
            Some(Ok(record)) => records.push(record),
            Some(Err(error)) => {
                let error = wrap_model_stream_error(&state.options, "read root rows", error);
                if records.is_empty() {
                    return Err(error);
                }
                state.pending_error = Some(error);
                state.finished = true;
                break;
            }
            None => {
                state.finished = true;
                break;
            }
        }
    }

    if records.is_empty() {
        return Ok(());
    }

    let models = hydrate_model_batch(
        state.executor,
        state.table,
        &state.relations,
        &state.model_extensions,
        &state.relation_aggregates,
        &records,
        &state.options,
    )
    .await?;

    state.buffered.extend(models.into_iter().map(Ok));
    Ok(())
}

async fn hydrate_model_batch<M>(
    executor: &dyn QueryExecutor,
    table: &'static TableMeta<M>,
    relations: &[AnyRelation<M>],
    model_extensions: &[AnyModelExtension<M>],
    relation_aggregates: &[AnyRelationAggregate<M>],
    records: &[DbRecord],
    options: &QueryExecutionOptions,
) -> Result<Vec<M>>
where
    M: Model,
{
    let mut models = records
        .iter()
        .map(|record| table.hydrate_record(record))
        .collect::<Result<Vec<_>>>()
        .map_err(|error| wrap_model_query_batch_error(options, "hydrate root rows", error))?;

    register_model_records(table, records);

    for extension in model_extensions {
        extension.load(executor, &models).await.map_err(|error| {
            wrap_model_query_batch_error(options, "load model extensions", error)
        })?;
    }

    for relation in relations {
        relation
            .load(executor, &mut models)
            .await
            .map_err(|error| {
                wrap_model_query_batch_error(options, "load eager relations", error)
            })?;
    }

    for aggregate in relation_aggregates {
        aggregate
            .load(executor, &mut models)
            .await
            .map_err(|error| {
                wrap_model_query_batch_error(options, "load relation aggregates", error)
            })?;
    }

    Ok(models)
}

fn wrap_model_query_batch_error(
    options: &QueryExecutionOptions,
    action: &str,
    error: Error,
) -> Error {
    let label = options
        .label
        .as_deref()
        .map(|label| format!(" in `{label}`"))
        .unwrap_or_default();
    Error::message(format!("model query failed to {action}{label}: {error}"))
}

fn wrap_model_stream_error(options: &QueryExecutionOptions, action: &str, error: Error) -> Error {
    let label = options
        .label
        .as_deref()
        .map(|label| format!(" in `{label}`"))
        .unwrap_or_default();
    Error::message(format!(
        "model query stream failed to {action}{label}: {error}"
    ))
}

fn decode_cursor<V: std::str::FromStr>(raw: &str) -> Result<V> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let decoded = URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .map_err(|e| Error::message(format!("invalid cursor: {e}")))?;
    let value_str = String::from_utf8(decoded)
        .map_err(|e| Error::message(format!("invalid cursor encoding: {e}")))?;
    value_str
        .parse()
        .map_err(|_| Error::message("invalid cursor value"))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::{
        pagination_from_query, CreateManyModel, CreateRow, InsertSource, ModelQuery, Paginated,
        Pagination, PaginationLinks, PaginationMeta, PostgresCompiler, ProjectionQuery, Query,
        QueryBody, Sql,
    };
    use crate::config::DatabaseConfig;
    use crate::database::{
        has_many, ColumnRef, DbRecord, DbValue, Expr, Loaded, QueryExecutionOptions, QueryExecutor,
        RelationDef,
    };
    use crate::foundation::{Error, Result};
    use crate::support::sync::lock_unpoisoned;
    use crate::support::Collection;
    use crate::{Model, ModelId};

    #[test]
    fn pagination_deserializes_defaults_and_exports_query_contract() {
        let empty: Pagination = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(empty, Pagination::default());

        let zero: Pagination =
            serde_json::from_value(serde_json::json!({ "page": 0, "per_page": 0 })).unwrap();
        assert_eq!(zero, Pagination::new(1, 1));

        let custom: Pagination =
            serde_json::from_value(serde_json::json!({ "page": 3, "per_page": 25 })).unwrap();
        assert_eq!(custom, Pagination::new(3, 25));
        assert_eq!(
            <Pagination as ts_rs::TS>::decl(),
            "type Pagination = { page?: number, per_page?: number, };"
        );
        assert_eq!(
            <Pagination as crate::openapi::ApiSchema>::schema(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "page": { "type": "integer" },
                    "per_page": { "type": "integer" }
                }
            })
        );
    }

    #[test]
    fn pagination_config_helpers_use_database_default_per_page() {
        let config = DatabaseConfig {
            default_per_page: 50,
            ..DatabaseConfig::default()
        };

        assert_eq!(
            Pagination::from_config(&config, None, None),
            Pagination::new(1, 50)
        );
        assert_eq!(
            Pagination::from_config(&config, Some(3), None),
            Pagination::new(3, 50)
        );
        assert_eq!(
            Pagination::from_config(&config, Some(3), Some(25)),
            Pagination::new(3, 25)
        );
    }

    #[test]
    fn pagination_query_parser_uses_configured_default_and_rejects_invalid_values() {
        assert_eq!(
            pagination_from_query(None, 40).unwrap(),
            Pagination::new(1, 40)
        );
        assert_eq!(
            pagination_from_query(Some("search=active&page=2"), 40).unwrap(),
            Pagination::new(2, 40)
        );
        assert_eq!(
            pagination_from_query(Some("page=0&per_page=0"), 40).unwrap(),
            Pagination::new(1, 1)
        );

        let error = pagination_from_query(Some("page=soon"), 40).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid pagination query parameter `page`: expected unsigned integer, got `soon`"
        );
    }

    #[test]
    fn paginated_meta_and_links_are_framework_owned() {
        let paginated = Paginated {
            data: Collection::from_vec(vec![1, 2]),
            pagination: Pagination::new(2, 10),
            total: 25,
        };

        assert_eq!(
            paginated.meta(),
            PaginationMeta {
                current_page: 2,
                per_page: 10,
                total: 25,
                last_page: 3,
            }
        );
        assert_eq!(
            paginated.links("/users"),
            PaginationLinks {
                next: Some("/users?page=3&per_page=10".to_string()),
                prev: Some("/users?page=1&per_page=10".to_string()),
            }
        );
        assert_eq!(
            paginated.links("/users?search=active&sort=name&page=2&per_page=25#results"),
            PaginationLinks {
                next: Some("/users?search=active&sort=name&page=3&per_page=10#results".to_string()),
                prev: Some("/users?search=active&sort=name&page=1&per_page=10#results".to_string()),
            }
        );

        let empty = Paginated {
            data: Collection::<i32>::new(),
            pagination: Pagination::new(1, 10),
            total: 0,
        };

        assert_eq!(
            empty.meta(),
            PaginationMeta {
                current_page: 1,
                per_page: 10,
                total: 0,
                last_page: 1,
            }
        );
        assert_eq!(
            empty.links("/users"),
            PaginationLinks {
                next: None,
                prev: None,
            }
        );

        let response = paginated.map_response("/users", |value| format!("user-{value}"));
        assert_eq!(response.data, vec!["user-1", "user-2"]);
        assert_eq!(
            response.meta,
            PaginationMeta {
                current_page: 2,
                per_page: 10,
                total: 25,
                last_page: 3,
            }
        );
        assert_eq!(
            response.links,
            PaginationLinks {
                next: Some("/users?page=3&per_page=10".to_string()),
                prev: Some("/users?page=1&per_page=10".to_string()),
            }
        );
    }

    #[test]
    fn insert_builder_switches_from_select_source_to_values_without_panicking() {
        let query = Query::insert_select_into("audit_logs", Query::table("users").select(["id"]))
            .value("id", 1_i64)
            .value("label", "one");
        let ast = query.ast();

        let QueryBody::Insert(insert) = &ast.body else {
            panic!("expected insert query body");
        };

        match &insert.source {
            InsertSource::Values(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].len(), 2);
            }
            InsertSource::Select(_) => panic!("insert source should have normalized to values"),
        }
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "exists_users")]
    struct ExistsUser {
        id: ModelId<Self>,
        active: bool,
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "query_helper_users")]
    struct QueryHelperUser {
        id: ModelId<Self>,
        name: String,
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "iteration_users", primary_key_strategy = "manual")]
    struct IterationUser {
        id: i64,
        children: Loaded<Vec<IterationChild>>,
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "iteration_children", primary_key_strategy = "manual")]
    struct IterationChild {
        id: i64,
        user_id: i64,
    }

    #[derive(Clone, Debug, PartialEq, crate::Projection)]
    struct IterationProjection {
        id: i64,
    }

    struct IterationExecutor {
        batches: Mutex<VecDeque<Vec<DbRecord>>>,
    }

    impl IterationExecutor {
        fn new(batches: impl IntoIterator<Item = Vec<DbRecord>>) -> Self {
            Self {
                batches: Mutex::new(batches.into_iter().collect()),
            }
        }
    }

    #[async_trait::async_trait]
    impl QueryExecutor for IterationExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            Ok(lock_unpoisoned(&self.batches, "iteration executor batches")
                .pop_front()
                .unwrap_or_default())
        }

        async fn raw_execute_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<u64> {
            Ok(0)
        }
    }

    #[test]
    fn model_query_where_in_helpers_compile_typed_columns() {
        let first = ModelId::<QueryHelperUser>::generate();
        let second = ModelId::<QueryHelperUser>::generate();

        let include = QueryHelperUser::model_query()
            .where_in(QueryHelperUser::ID, [first, second])
            .to_compiled_sql()
            .unwrap();
        let exclude = QueryHelperUser::model_query()
            .where_not_in(QueryHelperUser::NAME, ["archived", "deleted"])
            .to_compiled_sql()
            .unwrap();

        assert!(include
            .sql
            .contains("\"query_helper_users\".\"id\" IN ($1::uuid, $2::uuid)"));
        assert!(exclude
            .sql
            .contains("NOT (\"query_helper_users\".\"name\" IN ($1::text, $2::text))"));
    }

    #[test]
    fn generic_query_where_in_helpers_compile() {
        let include = Query::table("users")
            .select(["id"])
            .where_in("id", [1_i64, 2_i64]);
        let exclude = Query::table("users")
            .select(["id"])
            .where_not_in("status", ["archived", "deleted"]);

        assert_eq!(
            PostgresCompiler::compile(include.ast()).unwrap().sql,
            "SELECT \"id\" FROM \"users\" WHERE \"id\" IN ($1::bigint, $2::bigint)"
        );
        assert_eq!(
            PostgresCompiler::compile(exclude.ast()).unwrap().sql,
            "SELECT \"id\" FROM \"users\" WHERE NOT (\"status\" IN ($1::text, $2::text))"
        );
    }

    #[test]
    fn sql_expression_helpers_compile_common_report_patterns() {
        let query = Query::table("users")
            .select_expr(
                Sql::count_when(
                    Expr::column(ColumnRef::new("claims", "released_at")).is_not_null(),
                ),
                "released_count",
            )
            .select_expr(
                Sql::max(ColumnRef::new("claims", "claimed_at")),
                "last_claimed_at",
            )
            .having(Sql::count(ColumnRef::new("claims", "id")).gt_value(0_i64))
            .where_(
                Sql::concat_ws(
                    " ",
                    [
                        Expr::column("username"),
                        Expr::column("email"),
                        Expr::column("name"),
                    ],
                )
                .ilike("%amy%"),
            );

        let compiled = PostgresCompiler::compile(query.ast()).unwrap();

        assert!(compiled.sql.contains("COUNT(CASE WHEN"));
        assert!(compiled
            .sql
            .contains("\"claims\".\"released_at\" IS NOT NULL"));
        assert!(compiled.sql.contains("MAX(\"claims\".\"claimed_at\")"));
        assert!(compiled
            .sql
            .contains("COUNT(\"claims\".\"id\") > $4::bigint"));
        assert!(compiled
            .sql
            .contains("CONCAT_WS($2::text, \"username\", \"email\", \"name\") ILIKE $3::text"));
    }

    #[test]
    fn insert_value_expr_helpers_compile_operational_timestamps() {
        let query = Query::insert_into("job_history")
            .value("job_id", "job-1")
            .value_expr(
                "started_at",
                Sql::to_timestamp_millis(DbValue::Int64(1_712_345_678_000)),
            )
            .value_expr("completed_at", Sql::now());

        let compiled = PostgresCompiler::compile(query.ast()).unwrap();

        assert!(compiled.sql.contains("INSERT INTO \"job_history\""));
        assert!(compiled.sql.contains("\"started_at\""));
        assert!(compiled.sql.contains("TO_TIMESTAMP"));
        assert!(compiled.sql.contains("::double precision"));
        assert!(compiled.sql.contains("NOW()"));
    }

    #[test]
    fn uuid_v7_expression_helper_compiles_for_seeders() {
        let query = Query::insert_into("users").value_expr("id", Sql::uuid_v7());
        let compiled = PostgresCompiler::compile(query.ast()).unwrap();

        assert_eq!(
            compiled.sql,
            "INSERT INTO \"users\" (\"id\") VALUES (uuidv7())"
        );
    }

    #[test]
    fn json_text_comparison_helper_compiles() {
        let query = Query::table("voucher_claims").select(["id"]).where_(
            Expr::column("voucher_snapshot")
                .json()
                .key("name")
                .ilike("%coffee%"),
        );

        let compiled = PostgresCompiler::compile(query.ast()).unwrap();

        assert!(compiled
            .sql
            .contains("(\"voucher_snapshot\") ->> $1::text ILIKE $2::text"));
    }

    fn iteration_record(id: i64) -> DbRecord {
        let mut record = DbRecord::new();
        record.insert("id", DbValue::from(id));
        record
    }

    fn iteration_user_id(user: &IterationUser) -> i64 {
        user.id
    }

    fn attach_iteration_children(user: &mut IterationUser, children: Vec<IterationChild>) {
        user.children = Loaded::new(children);
    }

    fn iteration_children_relation() -> RelationDef<IterationUser, IterationChild> {
        has_many(
            IterationUser::ID,
            IterationChild::USER_ID,
            iteration_user_id,
            attach_iteration_children,
        )
    }

    #[test]
    fn model_exists_query_drops_ordering() {
        let ast = ModelQuery::new(ExistsUser::table_meta())
            .where_(ExistsUser::ACTIVE.eq(true))
            .order_by(ExistsUser::ID.desc())
            .exists_ast();
        let compiled = PostgresCompiler::compile(&ast).unwrap();

        assert!(!compiled.sql.contains("ORDER BY"));
        assert!(compiled.sql.contains("LIMIT"));
    }

    #[test]
    fn model_scope_panic_becomes_deferred_query_error() {
        let error = ModelQuery::new(IterationUser::table_meta())
            .scope(|_| panic!("model scope boom"))
            .to_compiled_sql()
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("scope callback panicked: model scope boom"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn projection_scope_panic_becomes_deferred_query_error() {
        let error = ProjectionQuery::<IterationProjection>::table("iteration_users")
            .scope(|_| panic!("projection scope boom"))
            .to_compiled_sql()
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("scope callback panicked: projection scope boom"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn where_has_scope_panic_becomes_deferred_query_error() {
        let error = ModelQuery::new(IterationUser::table_meta())
            .where_has(iteration_children_relation(), |_| panic!("where has boom"))
            .to_compiled_sql()
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("where_has callback panicked: where has boom"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn where_has_propagates_child_scope_deferred_query_error() {
        let error = ModelQuery::new(IterationUser::table_meta())
            .where_has(iteration_children_relation(), |query| {
                query.scope(|_| panic!("child scope boom"))
            })
            .to_compiled_sql()
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("scope callback panicked: child scope boom"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn chunk_callback_error_remains_unchanged() {
        let executor = IterationExecutor::new([vec![iteration_record(1)]]);

        let error = ModelQuery::new(IterationUser::table_meta())
            .chunk(&executor, 10, |_batch| async {
                Err(Error::message("chunk callback failed"))
            })
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "chunk callback failed");
    }

    #[tokio::test]
    async fn chunk_callback_factory_panic_becomes_query_error() {
        let executor = IterationExecutor::new([vec![iteration_record(1)]]);

        let error = ModelQuery::new(IterationUser::table_meta())
            .chunk(&executor, 10, |_batch| -> std::future::Ready<Result<()>> {
                panic!("chunk factory boom");
            })
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("chunk callback panicked: chunk factory boom"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn chunk_callback_future_panic_becomes_query_error() {
        let executor = IterationExecutor::new([vec![iteration_record(1)]]);

        let error = ModelQuery::new(IterationUser::table_meta())
            .chunk(&executor, 10, |_batch| async {
                panic!("chunk future boom");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("chunk callback panicked: chunk future boom"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn chunk_by_id_callback_panic_becomes_query_error() {
        let executor = IterationExecutor::new([vec![iteration_record(1)]]);

        let error = ModelQuery::new(IterationUser::table_meta())
            .chunk_by_id(&executor, IterationUser::ID, 10, |_batch| async {
                panic!("chunk by id boom");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("chunk_by_id callback panicked: chunk by id boom"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn each_by_id_callback_panic_becomes_query_error() {
        let executor = IterationExecutor::new([vec![iteration_record(1)]]);

        let error = ModelQuery::new(IterationUser::table_meta())
            .each_by_id(&executor, IterationUser::ID, 10, |_model| async {
                panic!("each by id boom");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("each_by_id callback panicked: each by id boom"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn create_many_row_builder_panic_becomes_deferred_query_error() {
        let error = CreateManyModel::new(IterationUser::table_meta())
            .row(|_row| -> CreateRow<IterationUser> {
                panic!("row builder boom");
            })
            .to_compiled_sql()
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("row callback panicked: row builder boom"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn create_many_row_builder_panic_takes_precedence_over_later_valid_rows() {
        let error = CreateManyModel::new(IterationUser::table_meta())
            .row(|_row| -> CreateRow<IterationUser> {
                panic!("first row boom");
            })
            .row(|row| row.set(IterationUser::ID, 1_i64))
            .to_compiled_sql()
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("row callback panicked: first row boom"),
            "unexpected error: {error}"
        );
    }
}
