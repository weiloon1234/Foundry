use std::marker::PhantomData;

use crate::foundation::{Error, Result};

use super::ast::{
    AggregateNode, DbType, DbValue, Expr, FromItem, Numeric, QueryAst, QueryBody, SelectNode,
};
use super::compiler::PostgresCompiler;
use super::model::FromDbValue;
use super::runtime::{DbRecord, QueryExecutor};

const INTERNAL_COUNT_ALIAS: &str = "__foundry_count";
const INTERNAL_COUNT_DISTINCT_ALIAS: &str = "__foundry_count_distinct";
const INTERNAL_SUM_ALIAS: &str = "__foundry_sum";
const INTERNAL_AVG_ALIAS: &str = "__foundry_avg";
const INTERNAL_MIN_ALIAS: &str = "__foundry_min";
const INTERNAL_MAX_ALIAS: &str = "__foundry_max";
const INTERNAL_WRAPPED_SOURCE_ALIAS: &str = "__foundry_wrapped_source";

#[derive(Clone, Debug)]
pub struct AggregateProjection<T> {
    node: AggregateNode,
    _marker: PhantomData<fn() -> T>,
}

impl<T> AggregateProjection<T> {
    pub(crate) fn from_node(node: AggregateNode) -> Self {
        Self {
            node,
            _marker: PhantomData,
        }
    }

    pub(crate) fn node(&self) -> AggregateNode {
        self.node.clone()
    }

    pub fn alias(&self) -> &str {
        &self.node.alias
    }
}

impl<T> AggregateProjection<T>
where
    T: FromDbValue,
{
    pub fn decode(&self, record: &DbRecord) -> Result<T> {
        decode_aggregate_value(record, self.alias())
    }
}

pub(crate) fn decode_aggregate_value<T>(record: &DbRecord, alias: &str) -> Result<T>
where
    T: FromDbValue,
{
    let value = record
        .get(alias)
        .ok_or_else(|| Error::message(format!("missing column `{alias}` in record")))?;
    if let DbValue::Text(text) = value {
        if let Ok(numeric) = Numeric::new(text.clone()) {
            if let Ok(decoded) = T::from_db_value(&DbValue::Numeric(numeric)) {
                return Ok(decoded);
            }
        }
    }
    T::from_db_value(value)
}

impl AggregateProjection<i64> {
    pub fn count_all(alias: &'static str) -> Self {
        Self::from_node(AggregateNode::count_all(alias))
    }

    pub fn count(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::from_node(AggregateNode::count(expr.into(), alias))
    }

    pub fn count_distinct(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::from_node(AggregateNode::count_distinct(expr.into(), alias))
    }

    pub(crate) fn internal_count() -> Self {
        Self::count_all(INTERNAL_COUNT_ALIAS)
    }

    pub(crate) fn internal_count_distinct(expr: impl Into<Expr>) -> Self {
        Self::count_distinct(expr, INTERNAL_COUNT_DISTINCT_ALIAS)
    }
}

impl<T> AggregateProjection<Option<T>>
where
    T: FromDbValue,
{
    pub fn sum(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::from_node(AggregateNode::sum(expr.into(), alias))
    }

    pub fn avg(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::from_node(AggregateNode::avg(expr.into(), alias))
    }

    pub fn min(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::from_node(AggregateNode::min(expr.into(), alias))
    }

    pub fn max(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::from_node(AggregateNode::max(expr.into(), alias))
    }

    pub(crate) fn internal_sum(expr: impl Into<Expr>) -> Self {
        Self::sum(expr, INTERNAL_SUM_ALIAS)
    }

    pub(crate) fn internal_avg(expr: impl Into<Expr>) -> Self {
        Self::avg(expr, INTERNAL_AVG_ALIAS)
    }

    pub(crate) fn internal_min(expr: impl Into<Expr>) -> Self {
        Self::min(expr, INTERNAL_MIN_ALIAS)
    }

    pub(crate) fn internal_max(expr: impl Into<Expr>) -> Self {
        Self::max(expr, INTERNAL_MAX_ALIAS)
    }
}

pub(crate) async fn count_query_ast<E>(executor: &E, ast: &QueryAst) -> Result<u64>
where
    E: QueryExecutor + ?Sized,
{
    let aggregate_ast = if can_aggregate_directly(ast) {
        direct_aggregate_query(ast, AggregateProjection::<i64>::internal_count().node())?
    } else {
        QueryAst::select(SelectNode {
            from: FromItem::subquery(
                without_outer_order_limit_offset(ast),
                INTERNAL_WRAPPED_SOURCE_ALIAS,
            ),
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
            aggregates: vec![AggregateProjection::<i64>::internal_count().node()],
        })
    };

    let compiled = PostgresCompiler::compile(&aggregate_ast)?;
    let record = executor
        .query_records(&compiled)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::message("aggregate query returned no rows"))?;
    Ok(record.decode::<i64>(INTERNAL_COUNT_ALIAS)? as u64)
}

pub(crate) async fn execute_scalar_projection_on_ast<E, T>(
    executor: &E,
    ast: &QueryAst,
    projection: AggregateProjection<T>,
) -> Result<T>
where
    E: QueryExecutor + ?Sized,
    T: FromDbValue,
{
    let aggregate_ast = if can_aggregate_directly(ast) {
        direct_aggregate_query(ast, projection.node())?
    } else {
        return Err(Error::message(
            "complex aggregate helpers require a typed projection alias; use ProjectionQuery for grouped or set-operation aggregates",
        ));
    };

    let compiled = PostgresCompiler::compile(&aggregate_ast)?;
    let record = executor
        .query_records(&compiled)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::message("aggregate query returned no rows"))?;

    projection.decode(&record)
}

pub(crate) fn wrap_query_for_alias_aggregate(
    ast: &QueryAst,
    alias: &str,
    alias_type: DbType,
    projection: AggregateNode,
) -> QueryAst {
    let mut wrapped_projection = projection;
    if wrapped_projection.aggregate.expr.is_some() {
        wrapped_projection.aggregate.expr = Some(Box::new(Expr::column(
            super::ast::ColumnRef::bare(alias).typed(alias_type),
        )));
    }

    QueryAst::select(SelectNode {
        from: FromItem::subquery(ast.clone(), INTERNAL_WRAPPED_SOURCE_ALIAS),
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
        aggregates: vec![wrapped_projection],
    })
}

fn direct_aggregate_query(ast: &QueryAst, projection: AggregateNode) -> Result<QueryAst> {
    let QueryBody::Select(select) = &ast.body else {
        return Err(Error::message(
            "scalar aggregate helpers require a select query or a projection alias wrapper",
        ));
    };

    let mut aggregate_select = (**select).clone();
    aggregate_select.columns.clear();
    aggregate_select.aggregates = vec![projection];
    aggregate_select.order_by.clear();
    aggregate_select.limit = None;
    aggregate_select.offset = None;

    Ok(QueryAst {
        with: ast.with.clone(),
        body: QueryBody::Select(Box::new(aggregate_select)),
    })
}

fn can_aggregate_directly(ast: &QueryAst) -> bool {
    match &ast.body {
        QueryBody::Select(select) => {
            !select.distinct
                && select.group_by.is_empty()
                && select.having.is_none()
                && select.aggregates.is_empty()
        }
        QueryBody::Insert(_)
        | QueryBody::Update(_)
        | QueryBody::Delete(_)
        | QueryBody::SetOperation(_) => false,
    }
}

fn without_outer_order_limit_offset(ast: &QueryAst) -> QueryAst {
    let mut ast = ast.clone();
    match &mut ast.body {
        QueryBody::Select(select) => {
            select.order_by.clear();
            select.limit = None;
            select.offset = None;
        }
        QueryBody::SetOperation(set) => {
            set.order_by.clear();
            set.limit = None;
            set.offset = None;
        }
        QueryBody::Insert(_) | QueryBody::Update(_) | QueryBody::Delete(_) => {}
    }
    ast
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_decode_converts_compiler_numeric_text_before_typed_hydration() {
        let projection = AggregateProjection::<Option<i64>>::sum(
            super::super::ast::ColumnRef::bare("amount").typed(DbType::Int64),
            "total",
        );
        let mut record = DbRecord::new();
        record.insert("total", DbValue::Text("2500.0000".to_string()));

        assert_eq!(projection.decode(&record).unwrap(), Some(2500));
    }

    #[test]
    fn aggregate_decode_falls_back_to_real_text_values() {
        let projection = AggregateProjection::<Option<String>>::min(
            super::super::ast::ColumnRef::bare("label").typed(DbType::Text),
            "minimum",
        );
        let mut record = DbRecord::new();
        record.insert("minimum", DbValue::Text("123".to_string()));

        assert_eq!(projection.decode(&record).unwrap(), Some("123".to_string()));
    }
}
