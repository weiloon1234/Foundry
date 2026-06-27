use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::foundation::{Error, Result};
use crate::support::{Date, DateTime, LocalDateTime, ModelId, Time};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Numeric(String);

impl Numeric {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_numeric(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Numeric {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl ts_rs::TS for Numeric {
    type WithoutGenerics = Self;

    fn name() -> String {
        "string".to_string()
    }

    fn inline() -> String {
        "string".to_string()
    }

    fn inline_flattened() -> String {
        panic!("{} cannot be flattened", Self::name())
    }

    fn decl() -> String {
        panic!("{} cannot be declared", Self::name())
    }

    fn decl_concrete() -> String {
        panic!("{} cannot be declared", Self::name())
    }
}

impl crate::openapi::ApiSchema for Numeric {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type": "string", "format": "decimal"})
    }

    fn schema_name() -> &'static str {
        "Numeric"
    }
}

impl From<i64> for Numeric {
    fn from(value: i64) -> Self {
        Self(value.to_string())
    }
}

impl TryFrom<&str> for Numeric {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for Numeric {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

fn validate_numeric(value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::message("numeric value cannot be empty"));
    }

    let unsigned = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);

    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some() {
        return Err(Error::message(
            "numeric value may only contain one decimal point",
        ));
    }

    if integer.is_empty() && fraction.is_none() {
        return Err(Error::message("numeric value must contain digits"));
    }

    if !integer.is_empty() && !integer.chars().all(|char| char.is_ascii_digit()) {
        return Err(Error::message("numeric integer part must be ascii digits"));
    }

    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.chars().all(|char| char.is_ascii_digit()) {
            return Err(Error::message(
                "numeric fractional part must be ascii digits",
            ));
        }
    }

    if integer.is_empty() && fraction.is_none() {
        return Err(Error::message("numeric value must contain digits"));
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbType {
    Int16,
    Int32,
    Int64,
    Bool,
    Float32,
    Float64,
    Numeric,
    Text,
    Json,
    Uuid,
    TimestampTz,
    Timestamp,
    Date,
    Time,
    Bytea,
    Int16Array,
    Int32Array,
    Int64Array,
    BoolArray,
    Float32Array,
    Float64Array,
    NumericArray,
    TextArray,
    JsonArray,
    UuidArray,
    TimestampTzArray,
    TimestampArray,
    DateArray,
    TimeArray,
    ByteaArray,
}

impl DbType {
    pub fn postgres_cast(self) -> &'static str {
        match self {
            Self::Int16 => "smallint",
            Self::Int32 => "integer",
            Self::Int64 => "bigint",
            Self::Bool => "boolean",
            Self::Float32 => "real",
            Self::Float64 => "double precision",
            Self::Numeric => "numeric",
            Self::Text => "text",
            Self::Json => "jsonb",
            Self::Uuid => "uuid",
            Self::TimestampTz => "timestamptz",
            Self::Timestamp => "timestamp",
            Self::Date => "date",
            Self::Time => "time",
            Self::Bytea => "bytea",
            Self::Int16Array => "smallint[]",
            Self::Int32Array => "integer[]",
            Self::Int64Array => "bigint[]",
            Self::BoolArray => "boolean[]",
            Self::Float32Array => "real[]",
            Self::Float64Array => "double precision[]",
            Self::NumericArray => "numeric[]",
            Self::TextArray => "text[]",
            Self::JsonArray => "jsonb[]",
            Self::UuidArray => "uuid[]",
            Self::TimestampTzArray => "timestamptz[]",
            Self::TimestampArray => "timestamp[]",
            Self::DateArray => "date[]",
            Self::TimeArray => "time[]",
            Self::ByteaArray => "bytea[]",
        }
    }

    pub fn array_element(self) -> Option<Self> {
        match self {
            Self::Int16Array => Some(Self::Int16),
            Self::Int32Array => Some(Self::Int32),
            Self::Int64Array => Some(Self::Int64),
            Self::BoolArray => Some(Self::Bool),
            Self::Float32Array => Some(Self::Float32),
            Self::Float64Array => Some(Self::Float64),
            Self::NumericArray => Some(Self::Numeric),
            Self::TextArray => Some(Self::Text),
            Self::JsonArray => Some(Self::Json),
            Self::UuidArray => Some(Self::Uuid),
            Self::TimestampTzArray => Some(Self::TimestampTz),
            Self::TimestampArray => Some(Self::Timestamp),
            Self::DateArray => Some(Self::Date),
            Self::TimeArray => Some(Self::Time),
            Self::ByteaArray => Some(Self::Bytea),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DbValue {
    Null(DbType),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    Bool(bool),
    Float32(f32),
    Float64(f64),
    Numeric(Numeric),
    Text(String),
    Json(serde_json::Value),
    Uuid(Uuid),
    TimestampTz(DateTime),
    Timestamp(LocalDateTime),
    Date(Date),
    Time(Time),
    Bytea(Vec<u8>),
    Int16Array(Vec<i16>),
    Int32Array(Vec<i32>),
    Int64Array(Vec<i64>),
    BoolArray(Vec<bool>),
    Float32Array(Vec<f32>),
    Float64Array(Vec<f64>),
    NumericArray(Vec<Numeric>),
    TextArray(Vec<String>),
    JsonArray(Vec<serde_json::Value>),
    UuidArray(Vec<Uuid>),
    TimestampTzArray(Vec<DateTime>),
    TimestampArray(Vec<LocalDateTime>),
    DateArray(Vec<Date>),
    TimeArray(Vec<Time>),
    ByteaArray(Vec<Vec<u8>>),
}

impl DbValue {
    pub fn null(db_type: DbType) -> Self {
        Self::Null(db_type)
    }

    pub fn db_type(&self) -> DbType {
        match self {
            Self::Null(db_type) => *db_type,
            Self::Int16(_) => DbType::Int16,
            Self::Int32(_) => DbType::Int32,
            Self::Int64(_) => DbType::Int64,
            Self::Bool(_) => DbType::Bool,
            Self::Float32(_) => DbType::Float32,
            Self::Float64(_) => DbType::Float64,
            Self::Numeric(_) => DbType::Numeric,
            Self::Text(_) => DbType::Text,
            Self::Json(_) => DbType::Json,
            Self::Uuid(_) => DbType::Uuid,
            Self::TimestampTz(_) => DbType::TimestampTz,
            Self::Timestamp(_) => DbType::Timestamp,
            Self::Date(_) => DbType::Date,
            Self::Time(_) => DbType::Time,
            Self::Bytea(_) => DbType::Bytea,
            Self::Int16Array(_) => DbType::Int16Array,
            Self::Int32Array(_) => DbType::Int32Array,
            Self::Int64Array(_) => DbType::Int64Array,
            Self::BoolArray(_) => DbType::BoolArray,
            Self::Float32Array(_) => DbType::Float32Array,
            Self::Float64Array(_) => DbType::Float64Array,
            Self::NumericArray(_) => DbType::NumericArray,
            Self::TextArray(_) => DbType::TextArray,
            Self::JsonArray(_) => DbType::JsonArray,
            Self::UuidArray(_) => DbType::UuidArray,
            Self::TimestampTzArray(_) => DbType::TimestampTzArray,
            Self::TimestampArray(_) => DbType::TimestampArray,
            Self::DateArray(_) => DbType::DateArray,
            Self::TimeArray(_) => DbType::TimeArray,
            Self::ByteaArray(_) => DbType::ByteaArray,
        }
    }

    pub fn relation_key(&self) -> String {
        match self {
            Self::Null(db_type) => format!("null:{db_type:?}"),
            Self::Int16(value) => format!("i16:{value}"),
            Self::Int32(value) => format!("i32:{value}"),
            Self::Int64(value) => format!("i64:{value}"),
            Self::Bool(value) => format!("bool:{value}"),
            Self::Float32(value) => format!("f32:{value}"),
            Self::Float64(value) => format!("f64:{value}"),
            Self::Numeric(value) => format!("numeric:{value}"),
            Self::Text(value) => format!("text:{value}"),
            Self::Json(value) => format!("json:{value}"),
            Self::Uuid(value) => format!("uuid:{value}"),
            Self::TimestampTz(value) => format!("timestamptz:{value}"),
            Self::Timestamp(value) => format!("timestamp:{value}"),
            Self::Date(value) => format!("date:{value}"),
            Self::Time(value) => format!("time:{value}"),
            Self::Bytea(value) => format!("bytea:{}", value.len()),
            Self::Int16Array(value) => format!("i16[]:{value:?}"),
            Self::Int32Array(value) => format!("i32[]:{value:?}"),
            Self::Int64Array(value) => format!("i64[]:{value:?}"),
            Self::BoolArray(value) => format!("bool[]:{value:?}"),
            Self::Float32Array(value) => format!("f32[]:{value:?}"),
            Self::Float64Array(value) => format!("f64[]:{value:?}"),
            Self::NumericArray(value) => format!("numeric[]:{value:?}"),
            Self::TextArray(value) => format!("text[]:{value:?}"),
            Self::JsonArray(value) => format!("json[]:{value:?}"),
            Self::UuidArray(value) => format!("uuid[]:{value:?}"),
            Self::TimestampTzArray(value) => format!("timestamptz[]:{value:?}"),
            Self::TimestampArray(value) => format!("timestamp[]:{value:?}"),
            Self::DateArray(value) => format!("date[]:{value:?}"),
            Self::TimeArray(value) => format!("time[]:{value:?}"),
            Self::ByteaArray(value) => format!("bytea[]:{}", value.len()),
        }
    }
}

impl From<i16> for DbValue {
    fn from(value: i16) -> Self {
        Self::Int16(value)
    }
}

impl From<i64> for DbValue {
    fn from(value: i64) -> Self {
        Self::Int64(value)
    }
}

impl From<i32> for DbValue {
    fn from(value: i32) -> Self {
        Self::Int32(value)
    }
}

impl From<bool> for DbValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for DbValue {
    fn from(value: f64) -> Self {
        Self::Float64(value)
    }
}

impl From<f32> for DbValue {
    fn from(value: f32) -> Self {
        Self::Float32(value)
    }
}

impl From<Numeric> for DbValue {
    fn from(value: Numeric) -> Self {
        Self::Numeric(value)
    }
}

impl From<String> for DbValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for DbValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<serde_json::Value> for DbValue {
    fn from(value: serde_json::Value) -> Self {
        Self::Json(value)
    }
}

impl From<Uuid> for DbValue {
    fn from(value: Uuid) -> Self {
        Self::Uuid(value)
    }
}

impl<M> From<ModelId<M>> for DbValue {
    fn from(value: ModelId<M>) -> Self {
        Self::Uuid(value.into_uuid())
    }
}

impl From<DateTime> for DbValue {
    fn from(value: DateTime) -> Self {
        Self::TimestampTz(value)
    }
}

impl From<LocalDateTime> for DbValue {
    fn from(value: LocalDateTime) -> Self {
        Self::Timestamp(value)
    }
}

impl From<Date> for DbValue {
    fn from(value: Date) -> Self {
        Self::Date(value)
    }
}

impl From<Time> for DbValue {
    fn from(value: Time) -> Self {
        Self::Time(value)
    }
}

impl From<Vec<u8>> for DbValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytea(value)
    }
}

impl From<Vec<i16>> for DbValue {
    fn from(value: Vec<i16>) -> Self {
        Self::Int16Array(value)
    }
}

impl From<Vec<i32>> for DbValue {
    fn from(value: Vec<i32>) -> Self {
        Self::Int32Array(value)
    }
}

impl From<Vec<i64>> for DbValue {
    fn from(value: Vec<i64>) -> Self {
        Self::Int64Array(value)
    }
}

impl From<Vec<bool>> for DbValue {
    fn from(value: Vec<bool>) -> Self {
        Self::BoolArray(value)
    }
}

impl From<Vec<f32>> for DbValue {
    fn from(value: Vec<f32>) -> Self {
        Self::Float32Array(value)
    }
}

impl From<Vec<f64>> for DbValue {
    fn from(value: Vec<f64>) -> Self {
        Self::Float64Array(value)
    }
}

impl From<Vec<Numeric>> for DbValue {
    fn from(value: Vec<Numeric>) -> Self {
        Self::NumericArray(value)
    }
}

impl From<Vec<String>> for DbValue {
    fn from(value: Vec<String>) -> Self {
        Self::TextArray(value)
    }
}

impl From<Vec<serde_json::Value>> for DbValue {
    fn from(value: Vec<serde_json::Value>) -> Self {
        Self::JsonArray(value)
    }
}

impl From<Vec<Uuid>> for DbValue {
    fn from(value: Vec<Uuid>) -> Self {
        Self::UuidArray(value)
    }
}

impl<M> From<Vec<ModelId<M>>> for DbValue {
    fn from(value: Vec<ModelId<M>>) -> Self {
        Self::UuidArray(value.into_iter().map(ModelId::into_uuid).collect())
    }
}

impl From<Vec<DateTime>> for DbValue {
    fn from(value: Vec<DateTime>) -> Self {
        Self::TimestampTzArray(value)
    }
}

impl From<Vec<LocalDateTime>> for DbValue {
    fn from(value: Vec<LocalDateTime>) -> Self {
        Self::TimestampArray(value)
    }
}

impl From<Vec<Date>> for DbValue {
    fn from(value: Vec<Date>) -> Self {
        Self::DateArray(value)
    }
}

impl From<Vec<Time>> for DbValue {
    fn from(value: Vec<Time>) -> Self {
        Self::TimeArray(value)
    }
}

impl From<Vec<Vec<u8>>> for DbValue {
    fn from(value: Vec<Vec<u8>>) -> Self {
        Self::ByteaArray(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRef {
    pub name: String,
    pub alias: Option<String>,
}

impl TableRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            alias: None,
        }
    }

    pub fn aliased(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }
}

impl From<&str> for TableRef {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for TableRef {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FromItem {
    Table(TableRef),
    Subquery { query: Box<QueryAst>, alias: String },
}

impl FromItem {
    pub fn subquery(query: impl Into<QueryAst>, alias: impl Into<String>) -> Self {
        Self::Subquery {
            query: Box::new(query.into()),
            alias: alias.into(),
        }
    }
}

impl From<TableRef> for FromItem {
    fn from(value: TableRef) -> Self {
        Self::Table(value)
    }
}

impl From<&str> for FromItem {
    fn from(value: &str) -> Self {
        Self::Table(TableRef::from(value))
    }
}

impl From<String> for FromItem {
    fn from(value: String) -> Self {
        Self::Table(TableRef::from(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnRef {
    pub table: Option<String>,
    pub name: String,
    pub alias: Option<String>,
    pub db_type: Option<DbType>,
}

impl ColumnRef {
    pub fn new(table: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            table: Some(table.into()),
            name: name.into(),
            alias: None,
            db_type: None,
        }
    }

    pub fn bare(name: impl Into<String>) -> Self {
        Self {
            table: None,
            name: name.into(),
            alias: None,
            db_type: None,
        }
    }

    pub fn typed(mut self, db_type: DbType) -> Self {
        self.db_type = Some(db_type);
        self
    }

    pub fn aliased(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }
}

impl From<&str> for ColumnRef {
    fn from(value: &str) -> Self {
        Self::bare(value)
    }
}

impl From<String> for ColumnRef {
    fn from(value: String) -> Self {
        Self::bare(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateFn {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateExpr {
    pub function: AggregateFn,
    pub expr: Option<Box<Expr>>,
    pub distinct: bool,
}

impl AggregateExpr {
    pub fn count_all() -> Self {
        Self {
            function: AggregateFn::Count,
            expr: None,
            distinct: false,
        }
    }

    pub fn count(expr: Expr) -> Self {
        Self {
            function: AggregateFn::Count,
            expr: Some(Box::new(expr)),
            distinct: false,
        }
    }

    pub fn count_distinct(expr: Expr) -> Self {
        Self {
            function: AggregateFn::Count,
            expr: Some(Box::new(expr)),
            distinct: true,
        }
    }

    pub fn sum(expr: Expr) -> Self {
        Self {
            function: AggregateFn::Sum,
            expr: Some(Box::new(expr)),
            distinct: false,
        }
    }

    pub fn avg(expr: Expr) -> Self {
        Self {
            function: AggregateFn::Avg,
            expr: Some(Box::new(expr)),
            distinct: false,
        }
    }

    pub fn min(expr: Expr) -> Self {
        Self {
            function: AggregateFn::Min,
            expr: Some(Box::new(expr)),
            distinct: false,
        }
    }

    pub fn max(expr: Expr) -> Self {
        Self {
            function: AggregateFn::Max,
            expr: Some(Box::new(expr)),
            distinct: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateNode {
    pub aggregate: AggregateExpr,
    pub alias: String,
}

impl AggregateNode {
    pub fn count_all(alias: impl Into<String>) -> Self {
        Self {
            aggregate: AggregateExpr::count_all(),
            alias: alias.into(),
        }
    }

    pub fn count(expr: Expr, alias: impl Into<String>) -> Self {
        Self {
            aggregate: AggregateExpr::count(expr),
            alias: alias.into(),
        }
    }

    pub fn count_distinct(expr: Expr, alias: impl Into<String>) -> Self {
        Self {
            aggregate: AggregateExpr::count_distinct(expr),
            alias: alias.into(),
        }
    }

    pub fn sum(expr: Expr, alias: impl Into<String>) -> Self {
        Self {
            aggregate: AggregateExpr::sum(expr),
            alias: alias.into(),
        }
    }

    pub fn avg(expr: Expr, alias: impl Into<String>) -> Self {
        Self {
            aggregate: AggregateExpr::avg(expr),
            alias: alias.into(),
        }
    }

    pub fn min(expr: Expr, alias: impl Into<String>) -> Self {
        Self {
            aggregate: AggregateExpr::min(expr),
            alias: alias.into(),
        }
    }

    pub fn max(expr: Expr, alias: impl Into<String>) -> Self {
        Self {
            aggregate: AggregateExpr::max(expr),
            alias: alias.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseWhen {
    pub condition: Condition,
    pub result: Box<Expr>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CaseExpr {
    pub whens: Vec<CaseWhen>,
    pub else_expr: Option<Box<Expr>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonPathSegment {
    Key(String),
    Index(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonPathMode {
    Json,
    Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonPathExpr {
    pub expr: Box<Expr>,
    pub path: Vec<JsonPathSegment>,
    pub mode: JsonPathMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonPredicateOp {
    Contains,
    ContainedBy,
    HasKey,
    HasAnyKeys,
    HasAllKeys,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum JsonPredicateValue {
    Json(serde_json::Value),
    Key(String),
    Keys(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UnaryOperator {
    Not,
    Negate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnaryExpr {
    pub operator: UnaryOperator,
    pub expr: Box<Expr>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Concat,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BinaryExpr {
    pub left: Box<Expr>,
    pub operator: BinaryOperator,
    pub right: Box<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowFrameUnits {
    Rows,
    Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WindowFrameBound {
    UnboundedPreceding,
    Preceding(u64),
    CurrentRow,
    Following(u64),
    UnboundedFollowing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowFrame {
    pub units: WindowFrameUnits,
    pub start: WindowFrameBound,
    pub end: Option<WindowFrameBound>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowSpec {
    pub partition_by: Vec<Expr>,
    pub order_by: Vec<OrderBy>,
    pub frame: Option<WindowFrame>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowExpr {
    pub function: Box<Expr>,
    pub window: WindowSpec,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Column(ColumnRef),
    Excluded(ColumnRef),
    Value(DbValue),
    Cast { expr: Box<Expr>, db_type: DbType },
    Aggregate(AggregateExpr),
    Function(Box<FunctionCall>),
    Unary(Box<UnaryExpr>),
    Binary(Box<BinaryExpr>),
    Subquery(Box<QueryAst>),
    Window(Box<WindowExpr>),
    Case(Box<CaseExpr>),
    JsonPath(Box<JsonPathExpr>),
    Raw(String),
}

impl Expr {
    pub fn column(column: impl Into<ColumnRef>) -> Self {
        Self::Column(column.into())
    }

    pub fn excluded(column: impl Into<ColumnRef>) -> Self {
        Self::Excluded(column.into())
    }

    pub fn value(value: impl Into<DbValue>) -> Self {
        Self::Value(value.into())
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self::Value(DbValue::Text(value.into()))
    }

    pub fn bool(value: bool) -> Self {
        Self::Value(DbValue::Bool(value))
    }

    pub fn false_() -> Self {
        Self::bool(false)
    }

    pub fn true_() -> Self {
        Self::bool(true)
    }

    pub fn cast(expr: impl Into<Expr>, db_type: DbType) -> Self {
        Self::Cast {
            expr: Box::new(expr.into()),
            db_type,
        }
    }

    pub fn cast_text(expr: impl Into<Expr>) -> Self {
        Self::cast(expr, DbType::Text)
    }

    pub fn function(name: impl Into<String>, args: impl IntoIterator<Item = Expr>) -> Self {
        Self::Function(Box::new(FunctionCall {
            name: name.into(),
            args: args.into_iter().collect(),
        }))
    }

    pub fn unary(operator: UnaryOperator, expr: impl Into<Expr>) -> Self {
        Self::Unary(Box::new(UnaryExpr {
            operator,
            expr: Box::new(expr.into()),
        }))
    }

    pub fn binary(left: impl Into<Expr>, operator: BinaryOperator, right: impl Into<Expr>) -> Self {
        Self::Binary(Box::new(BinaryExpr {
            left: Box::new(left.into()),
            operator,
            right: Box::new(right.into()),
        }))
    }

    pub fn subquery(query: impl Into<QueryAst>) -> Self {
        Self::Subquery(Box::new(query.into()))
    }

    pub fn window(function: impl Into<Expr>, window: WindowSpec) -> Self {
        Self::Window(Box::new(WindowExpr {
            function: Box::new(function.into()),
            window,
        }))
    }

    pub fn raw(sql: impl Into<String>) -> Self {
        Self::Raw(sql.into())
    }
}

impl From<ColumnRef> for Expr {
    fn from(value: ColumnRef) -> Self {
        Self::Column(value)
    }
}

impl From<&str> for Expr {
    fn from(value: &str) -> Self {
        Self::Column(ColumnRef::from(value))
    }
}

impl From<String> for Expr {
    fn from(value: String) -> Self {
        Self::Column(ColumnRef::from(value))
    }
}

impl From<DbValue> for Expr {
    fn from(value: DbValue) -> Self {
        Self::Value(value)
    }
}

impl From<AggregateExpr> for Expr {
    fn from(value: AggregateExpr) -> Self {
        Self::Aggregate(value)
    }
}

impl From<FunctionCall> for Expr {
    fn from(value: FunctionCall) -> Self {
        Self::Function(Box::new(value))
    }
}

impl From<UnaryExpr> for Expr {
    fn from(value: UnaryExpr) -> Self {
        Self::Unary(Box::new(value))
    }
}

impl From<BinaryExpr> for Expr {
    fn from(value: BinaryExpr) -> Self {
        Self::Binary(Box::new(value))
    }
}

impl From<WindowExpr> for Expr {
    fn from(value: WindowExpr) -> Self {
        Self::Window(Box::new(value))
    }
}

impl From<CaseExpr> for Expr {
    fn from(value: CaseExpr) -> Self {
        Self::Case(Box::new(value))
    }
}

impl From<JsonPathExpr> for Expr {
    fn from(value: JsonPathExpr) -> Self {
        Self::JsonPath(Box::new(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOp {
    Eq,
    IEq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    NotLike,
    ILike,
}

impl fmt::Display for ComparisonOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Eq => "=",
            Self::IEq => "IEQ",
            Self::NotEq => "<>",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Like => "LIKE",
            Self::NotLike => "NOT LIKE",
            Self::ILike => "ILIKE",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    Comparison {
        left: Expr,
        op: ComparisonOp,
        right: Expr,
    },
    InList {
        expr: Expr,
        values: Vec<DbValue>,
    },
    JsonPredicate {
        expr: Expr,
        op: JsonPredicateOp,
        value: JsonPredicateValue,
    },
    FullText {
        columns: Vec<ColumnRef>,
        query: String,
    },
    And(Vec<Condition>),
    Or(Vec<Condition>),
    Not(Box<Condition>),
    IsNull(Expr),
    IsNotNull(Expr),
    Exists(Box<QueryAst>),
    Raw {
        sql: String,
        bindings: Vec<DbValue>,
    },
}

impl Condition {
    pub fn compare(left: Expr, op: ComparisonOp, right: Expr) -> Self {
        Self::Comparison { left, op, right }
    }

    pub fn json(expr: Expr, op: JsonPredicateOp, value: JsonPredicateValue) -> Self {
        Self::JsonPredicate { expr, op, value }
    }

    pub fn full_text(
        columns: impl IntoIterator<Item = ColumnRef>,
        query: impl Into<String>,
    ) -> Self {
        Self::FullText {
            columns: columns.into_iter().collect(),
            query: query.into(),
        }
    }

    pub fn and(conditions: impl IntoIterator<Item = Condition>) -> Self {
        Self::And(conditions.into_iter().collect())
    }

    pub fn or(conditions: impl IntoIterator<Item = Condition>) -> Self {
        Self::Or(conditions.into_iter().collect())
    }

    pub fn negate(condition: Condition) -> Self {
        Self::Not(Box::new(condition))
    }

    pub fn exists(query: impl Into<QueryAst>) -> Self {
        Self::Exists(Box::new(query.into()))
    }

    pub fn is_null(column: impl Into<ColumnRef>) -> Self {
        Self::expr_is_null(Expr::column(column.into()))
    }

    pub fn is_not_null(column: impl Into<ColumnRef>) -> Self {
        Self::expr_is_not_null(Expr::column(column.into()))
    }

    pub fn expr_is_null(expr: impl Into<Expr>) -> Self {
        Self::IsNull(expr.into())
    }

    pub fn expr_is_not_null(expr: impl Into<Expr>) -> Self {
        Self::IsNotNull(expr.into())
    }

    pub fn false_() -> Self {
        Self::compare(Expr::false_(), ComparisonOp::Eq, Expr::true_())
    }

    pub fn true_() -> Self {
        Self::compare(Expr::true_(), ComparisonOp::Eq, Expr::true_())
    }

    pub fn raw(sql: impl Into<String>, bindings: Vec<DbValue>) -> Self {
        Self::Raw {
            sql: sql.into(),
            bindings,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderBy {
    pub expr: Expr,
    pub direction: OrderDirection,
}

impl OrderBy {
    pub fn asc(expr: impl Into<Expr>) -> Self {
        Self {
            expr: expr.into(),
            direction: OrderDirection::Asc,
        }
    }

    pub fn desc(expr: impl Into<Expr>) -> Self {
        Self {
            expr: expr.into(),
            direction: OrderDirection::Desc,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectItem {
    pub expr: Expr,
    pub alias: Option<String>,
}

impl SelectItem {
    pub fn new(expr: impl Into<Expr>) -> Self {
        Self {
            expr: expr.into(),
            alias: None,
        }
    }

    pub fn aliased(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JoinNode {
    pub kind: JoinKind,
    pub table: FromItem,
    pub lateral: bool,
    pub on: Option<Condition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockStrength {
    Update,
    NoKeyUpdate,
    Share,
    KeyShare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockBehavior {
    Wait,
    NoWait,
    SkipLocked,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LockClause {
    pub strength: LockStrength,
    pub of: Vec<String>,
    pub behavior: LockBehavior,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    HasMany,
    HasOne,
    BelongsTo,
    ManyToMany,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PivotNode {
    pub table: TableRef,
    pub local_key: ColumnRef,
    pub foreign_key: ColumnRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelationNode {
    pub name: String,
    pub kind: RelationKind,
    pub target: TableRef,
    pub local_key: ColumnRef,
    pub foreign_key: ColumnRef,
    pub pivot: Option<PivotNode>,
    pub filters: Option<Condition>,
    pub children: Vec<RelationNode>,
    pub aggregates: Vec<AggregateNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectNode {
    pub from: FromItem,
    pub distinct: bool,
    pub columns: Vec<SelectItem>,
    pub joins: Vec<JoinNode>,
    pub condition: Option<Condition>,
    pub group_by: Vec<Expr>,
    pub having: Option<Condition>,
    pub order_by: Vec<OrderBy>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub lock: Option<LockClause>,
    pub relations: Vec<RelationNode>,
    pub aggregates: Vec<AggregateNode>,
}

impl SelectNode {
    pub fn from(source: impl Into<FromItem>) -> Self {
        Self {
            from: source.into(),
            distinct: false,
            columns: Vec::new(),
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
        }
    }

    pub fn select(mut self, expr: impl Into<Expr>) -> Self {
        self.columns.push(SelectItem::new(expr));
        self
    }

    pub fn select_as(mut self, expr: impl Into<Expr>, alias: impl Into<String>) -> Self {
        self.columns.push(SelectItem::new(expr).aliased(alias));
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        self.condition = Some(match self.condition {
            Some(existing) => Condition::and([existing, condition]),
            None => condition,
        });
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.order_by.push(order);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OnConflictTarget {
    Columns(Vec<ColumnRef>),
    Constraint(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OnConflictAction {
    DoNothing,
    DoUpdate(Box<OnConflictUpdate>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OnConflictUpdate {
    pub assignments: Vec<(ColumnRef, Expr)>,
    pub condition: Option<Condition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OnConflictNode {
    pub target: Option<OnConflictTarget>,
    pub action: OnConflictAction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InsertSource {
    Values(Vec<Vec<(ColumnRef, Expr)>>),
    Select(Box<QueryAst>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InsertNode {
    pub into: TableRef,
    pub source: InsertSource,
    pub on_conflict: Option<OnConflictNode>,
    pub returning: Vec<SelectItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateNode {
    pub table: TableRef,
    pub values: Vec<(ColumnRef, Expr)>,
    pub from: Vec<FromItem>,
    pub condition: Option<Condition>,
    pub returning: Vec<SelectItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeleteNode {
    pub from: TableRef,
    pub using: Vec<FromItem>,
    pub condition: Option<Condition>,
    pub returning: Vec<SelectItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CteMaterialization {
    Materialized,
    NotMaterialized,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CteNode {
    pub name: String,
    pub query: Box<QueryAst>,
    pub recursive: bool,
    pub materialization: Option<CteMaterialization>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetOperator {
    Union,
    UnionAll,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetOperationNode {
    pub left: Box<QueryAst>,
    pub operator: SetOperator,
    pub right: Box<QueryAst>,
    pub order_by: Vec<OrderBy>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum QueryBody {
    Select(Box<SelectNode>),
    Insert(Box<InsertNode>),
    Update(Box<UpdateNode>),
    Delete(Box<DeleteNode>),
    SetOperation(Box<SetOperationNode>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryAst {
    pub with: Vec<CteNode>,
    pub body: QueryBody,
}

impl QueryAst {
    pub fn select(select: SelectNode) -> Self {
        Self {
            with: Vec::new(),
            body: QueryBody::Select(Box::new(select)),
        }
    }

    pub fn insert(insert: InsertNode) -> Self {
        Self {
            with: Vec::new(),
            body: QueryBody::Insert(Box::new(insert)),
        }
    }

    pub fn update(update: UpdateNode) -> Self {
        Self {
            with: Vec::new(),
            body: QueryBody::Update(Box::new(update)),
        }
    }

    pub fn delete(delete: DeleteNode) -> Self {
        Self {
            with: Vec::new(),
            body: QueryBody::Delete(Box::new(delete)),
        }
    }

    pub fn set_operation(set: SetOperationNode) -> Self {
        Self {
            with: Vec::new(),
            body: QueryBody::SetOperation(Box::new(set)),
        }
    }
}

impl From<SelectNode> for QueryAst {
    fn from(value: SelectNode) -> Self {
        Self::select(value)
    }
}

impl From<InsertNode> for QueryAst {
    fn from(value: InsertNode) -> Self {
        Self::insert(value)
    }
}

impl From<UpdateNode> for QueryAst {
    fn from(value: UpdateNode) -> Self {
        Self::update(value)
    }
}

impl From<DeleteNode> for QueryAst {
    fn from(value: DeleteNode) -> Self {
        Self::delete(value)
    }
}
