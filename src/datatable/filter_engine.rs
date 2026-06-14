use crate::database::{ComparisonOp, Condition, DbType, DbValue, Expr, Numeric, OrderBy};
use crate::foundation::{Error, Result};
use crate::support::{Date, DateTime, LocalDateTime};

use super::column::{DatatableColumn, DatatableFilterScope};
use super::datatable_trait::DatatableQuery;
use super::relation_filter::DatatableRelationFilter;
use super::request::{
    DatatableFilterInput, DatatableFilterOp, DatatableFilterValue, DatatableSortInput,
};
use super::sort::DatatableSort;

// ---------------------------------------------------------------------------
// Auto-filter application
// ---------------------------------------------------------------------------

/// Apply structured filter inputs to a datatable query.
pub fn apply_auto_filters<Row: 'static, Q>(
    query: Q,
    filters: &[DatatableFilterInput],
    columns: &[DatatableColumn<Row>],
) -> Result<Q>
where
    Q: DatatableQuery<Row>,
{
    apply_auto_filters_with_relation_filters(query, filters, columns, &[])
}

pub fn apply_auto_filters_with_relation_filters<Row: 'static, Q>(
    mut query: Q,
    filters: &[DatatableFilterInput],
    columns: &[DatatableColumn<Row>],
    relation_filters: &[DatatableRelationFilter<Row, Q>],
) -> Result<Q>
where
    Q: DatatableQuery<Row>,
{
    for filter in filters {
        if filter.op == DatatableFilterOp::LikeAny {
            if let Some(relation_filter) = relation_filters
                .iter()
                .find(|relation_filter| relation_filter.matches(&filter.field))
            {
                query = relation_filter.apply(query, filter)?;
                continue;
            }

            query = apply_like_any(query, filter, columns)?;
            continue;
        }

        let col = match columns.iter().find(|c| c.name == filter.field) {
            Some(c) if c.filterable => c,
            Some(_) => {
                return Err(Error::message(format!(
                    "column '{}' is not filterable",
                    filter.field
                )));
            }
            None => {
                if let Some(relation_filter) = relation_filters
                    .iter()
                    .find(|relation_filter| relation_filter.matches(&filter.field))
                {
                    query = relation_filter.apply(query, filter)?;
                    continue;
                }

                return Err(Error::message(format!(
                    "unknown filter field '{}'",
                    filter.field
                )));
            }
        };

        let target = col.filter_target().ok_or_else(|| {
            Error::message(format!(
                "column '{}' has no filter target; use filter_by(...) or filter_having(...)",
                col.name
            ))
        })?;

        let condition = build_filter_condition(
            &filter.op,
            target.expr.clone(),
            &filter.value,
            col.db_type(),
        )?;
        query = apply_filter(query, target.scope, condition);
    }

    Ok(query)
}

fn apply_like_any<Row: 'static, Q>(
    query: Q,
    filter: &DatatableFilterInput,
    columns: &[DatatableColumn<Row>],
) -> Result<Q>
where
    Q: DatatableQuery<Row>,
{
    let text = match &filter.value {
        DatatableFilterValue::Text(s) => s.clone(),
        _ => return Err(Error::message("LikeAny requires a text value")),
    };

    let pattern = format!("%{text}%");
    let field_names: Vec<&str> = filter.field.split('|').collect();

    let mut scope = None;
    let mut conditions = Vec::new();
    for name in &field_names {
        let Some(col) = columns.iter().find(|c| c.name == *name) else {
            continue;
        };
        if !col.filterable {
            continue;
        }
        let Some(target) = col.filter_target() else {
            continue;
        };

        if let Some(existing_scope) = scope {
            if existing_scope != target.scope {
                return Err(Error::message(
                    "LikeAny cannot mix WHERE and HAVING filter targets",
                ));
            }
        } else {
            scope = Some(target.scope);
        }

        conditions.push(Condition::compare(
            target.expr.clone(),
            ComparisonOp::ILike,
            Expr::value(DbValue::Text(pattern.clone())),
        ));
    }

    if conditions.is_empty() {
        return Ok(query);
    }

    let Some(scope) = scope else {
        return Err(Error::message(
            "LikeAny requires at least one filter target",
        ));
    };

    Ok(apply_filter(query, scope, Condition::or(conditions)))
}

pub(crate) fn build_filter_condition(
    op: &DatatableFilterOp,
    target_expr: Expr,
    value: &DatatableFilterValue,
    db_type: DbType,
) -> Result<Condition> {
    match op {
        DatatableFilterOp::Eq => {
            let db_val = filter_value_to_db(value, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Eq,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::NotEq => {
            let db_val = filter_value_to_db(value, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::NotEq,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::Like => {
            let text = expect_text(value)?;
            let pattern = format!("%{text}%");
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::ILike,
                Expr::value(DbValue::Text(pattern)),
            ))
        }
        DatatableFilterOp::Gt => {
            let db_val = filter_value_to_db(value, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Gt,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::Gte => {
            let db_val = filter_value_to_db(value, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Gte,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::Lt => {
            let db_val = filter_value_to_db(value, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Lt,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::Lte => {
            let db_val = filter_value_to_db(value, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Lte,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::In => {
            let values = expect_values(value)?;
            let db_values: Vec<DbValue> = values
                .iter()
                .map(|v| text_to_db_value(v, db_type))
                .collect::<Result<Vec<_>>>()?;
            Ok(Condition::InList {
                expr: target_expr,
                values: db_values,
            })
        }
        DatatableFilterOp::Date => {
            let text = expect_text(value)?;
            let db_val = text_to_db_value(&text, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Eq,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::DateFrom => {
            let text = expect_text(value)?;
            let db_val = text_to_db_value(&text, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Gte,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::DateTo => {
            let text = expect_text(value)?;
            let db_val = text_to_db_value(&text, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Lte,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::Datetime => {
            let text = expect_text(value)?;
            let db_val = text_to_db_value(&text, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Eq,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::DatetimeFrom => {
            let text = expect_text(value)?;
            let db_val = text_to_db_value(&text, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Gte,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::DatetimeTo => {
            let text = expect_text(value)?;
            let db_val = text_to_db_value(&text, db_type)?;
            Ok(Condition::compare(
                target_expr,
                ComparisonOp::Lte,
                Expr::value(db_val),
            ))
        }
        DatatableFilterOp::Has => match target_expr {
            Expr::Column(col_ref) => Ok(Condition::is_not_null(col_ref)),
            _ => Err(Error::message(
                "Has filters require a column expression target",
            )),
        },
        DatatableFilterOp::HasLike => match target_expr {
            Expr::Column(col_ref) => {
                let text = expect_text(value)?;
                let pattern = format!("%{text}%");
                let not_null = Condition::is_not_null(col_ref.clone());
                let like = Condition::compare(
                    Expr::column(col_ref),
                    ComparisonOp::ILike,
                    Expr::value(DbValue::Text(pattern)),
                );
                Ok(Condition::and(vec![not_null, like]))
            }
            _ => Err(Error::message(
                "HasLike filters require a column expression target",
            )),
        },
        DatatableFilterOp::LikeAny => Err(Error::message("LikeAny should be handled separately")),
    }
}

// ---------------------------------------------------------------------------
// Sort application
// ---------------------------------------------------------------------------

/// Apply sort inputs to a datatable query.
pub fn apply_sorts<Row: 'static, Q>(
    mut query: Q,
    sorts: &[DatatableSortInput],
    columns: &[DatatableColumn<Row>],
) -> Result<Q>
where
    Q: DatatableQuery<Row>,
{
    for sort in sorts {
        let col = match columns.iter().find(|c| c.name == sort.field) {
            Some(c) if c.sortable => c,
            Some(_) => {
                return Err(Error::message(format!(
                    "column '{}' is not sortable",
                    sort.field
                )));
            }
            None => {
                return Err(Error::message(format!(
                    "unknown sort field '{}'",
                    sort.field
                )));
            }
        };

        let expr = col.sort_expr().cloned().ok_or_else(|| {
            Error::message(format!(
                "column '{}' has no sort target; use sort_by(...) to define one",
                col.name
            ))
        })?;
        let order_by = OrderBy {
            expr,
            direction: sort.direction,
        };
        query = query.apply_order(order_by);
    }

    Ok(query)
}

/// Apply default sort declarations (used when request has no sort).
pub fn apply_default_sorts<Row: 'static, Q>(mut query: Q, sorts: &[DatatableSort<Row>]) -> Result<Q>
where
    Q: DatatableQuery<Row>,
{
    for sort in sorts {
        let order_by = OrderBy {
            expr: sort.expr.clone(),
            direction: sort.direction,
        };
        query = query.apply_order(order_by);
    }
    Ok(query)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn apply_filter<Row: 'static, Q>(query: Q, scope: DatatableFilterScope, condition: Condition) -> Q
where
    Q: DatatableQuery<Row>,
{
    match scope {
        DatatableFilterScope::Where => query.apply_where(condition),
        DatatableFilterScope::Having => query.apply_having(condition),
    }
}

fn expect_text(value: &DatatableFilterValue) -> Result<String> {
    match value {
        DatatableFilterValue::Text(s) => Ok(s.clone()),
        _ => Err(Error::message(
            "expected text value for this filter operation",
        )),
    }
}

fn expect_values(value: &DatatableFilterValue) -> Result<Vec<String>> {
    match value {
        DatatableFilterValue::Values(vs) => Ok(vs.clone()),
        DatatableFilterValue::Text(s) => Ok(vec![s.clone()]),
        _ => Err(Error::message(
            "expected list of values for In filter operation",
        )),
    }
}

fn filter_value_to_db(value: &DatatableFilterValue, db_type: DbType) -> Result<DbValue> {
    match value {
        DatatableFilterValue::Text(s) => text_to_db_value(s, db_type),
        DatatableFilterValue::Bool(b) => Ok(DbValue::Bool(*b)),
        DatatableFilterValue::Number(n) => number_to_db_value(*n, db_type),
        DatatableFilterValue::Values(vs) if vs.len() == 1 => text_to_db_value(&vs[0], db_type),
        _ => Err(Error::message(
            "cannot convert this filter value to a database value",
        )),
    }
}

fn text_to_db_value(text: &str, db_type: DbType) -> Result<DbValue> {
    match db_type {
        DbType::Bool => text
            .parse::<bool>()
            .map(DbValue::Bool)
            .map_err(|e| Error::message(format!("invalid boolean '{}': {e}", text))),
        DbType::Int16 => text
            .parse::<i16>()
            .map(DbValue::Int16)
            .map_err(|e| Error::message(format!("invalid integer '{}': {e}", text))),
        DbType::Int32 => text
            .parse::<i32>()
            .map(DbValue::Int32)
            .map_err(|e| Error::message(format!("invalid integer '{}': {e}", text))),
        DbType::Int64 => text
            .parse::<i64>()
            .map(DbValue::Int64)
            .map_err(|e| Error::message(format!("invalid integer '{}': {e}", text))),
        DbType::Float32 => text
            .parse::<f32>()
            .map(DbValue::Float32)
            .map_err(|e| Error::message(format!("invalid float '{}': {e}", text))),
        DbType::Float64 => text
            .parse::<f64>()
            .map(DbValue::Float64)
            .map_err(|e| Error::message(format!("invalid float '{}': {e}", text))),
        DbType::Numeric => Numeric::new(text.to_string())
            .map(DbValue::Numeric)
            .map_err(|e| Error::message(format!("invalid numeric '{}': {e}", text))),
        DbType::Date => text
            .parse::<Date>()
            .map(DbValue::Date)
            .map_err(|e| Error::message(format!("invalid date '{}': {e}", text))),
        DbType::Timestamp => text
            .parse::<LocalDateTime>()
            .map(DbValue::Timestamp)
            .map_err(|e| Error::message(format!("invalid timestamp '{}': {e}", text))),
        DbType::TimestampTz => text
            .parse::<DateTime>()
            .map(DbValue::TimestampTz)
            .map_err(|e| Error::message(format!("invalid timestamptz '{}': {e}", text))),
        DbType::Uuid => uuid::Uuid::parse_str(text)
            .map(DbValue::Uuid)
            .map_err(|e| Error::message(format!("invalid uuid '{}': {e}", text))),
        _ => Ok(DbValue::Text(text.to_string())),
    }
}

fn number_to_db_value(n: i64, db_type: DbType) -> Result<DbValue> {
    match db_type {
        DbType::Int16 => Ok(DbValue::Int16(n as i16)),
        DbType::Int32 => Ok(DbValue::Int32(n as i32)),
        DbType::Int64 => Ok(DbValue::Int64(n)),
        DbType::Float32 => Ok(DbValue::Float32(n as f32)),
        DbType::Float64 => Ok(DbValue::Float64(n as f64)),
        DbType::Numeric => Ok(DbValue::Numeric(Numeric::from(n))),
        _ => Ok(DbValue::Int64(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::apply_auto_filters;
    use super::{number_to_db_value, text_to_db_value};
    use crate::database::{DbType, DbValue, Expr, Numeric, ProjectionQuery};
    use crate::datatable::{
        DatatableColumn, DatatableFilterInput, DatatableFilterOp, DatatableFilterValue,
    };

    #[derive(Clone, serde::Serialize, foundry_macros::Projection)]
    struct ReportRow {
        total: i64,
        merchant_id: i64,
    }

    #[test]
    fn like_any_rejects_mixed_where_and_having_targets() {
        let columns = vec![
            DatatableColumn::field(ReportRow::MERCHANT_ID)
                .filter_by(ReportRow::MERCHANT_ID.column_ref()),
            DatatableColumn::field(ReportRow::TOTAL).filter_having(Expr::function(
                "SUM",
                [Expr::column(ReportRow::TOTAL.column_ref())],
            )),
        ];
        let filters = vec![DatatableFilterInput {
            field: "merchant_id|total".to_string(),
            op: DatatableFilterOp::LikeAny,
            value: DatatableFilterValue::Text("10".to_string()),
        }];

        let result = apply_auto_filters(
            ProjectionQuery::<ReportRow>::table("orders"),
            &filters,
            &columns,
        );
        let error = match result {
            Ok(_) => panic!("mixed scopes should fail"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("LikeAny cannot mix WHERE and HAVING"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn coerces_numeric_text_filters_into_numeric_db_values() {
        let value = text_to_db_value("12.50", DbType::Numeric).unwrap();
        assert_eq!(value, DbValue::Numeric(Numeric::new("12.50").unwrap()));

        let value = number_to_db_value(12, DbType::Numeric).unwrap();
        assert_eq!(value, DbValue::Numeric(Numeric::new("12").unwrap()));
    }

    #[test]
    fn rejects_invalid_numeric_text_filters() {
        let error = text_to_db_value("12.5.0", DbType::Numeric).unwrap_err();
        assert!(
            error.to_string().contains("invalid numeric '12.5.0'"),
            "unexpected error: {error}"
        );
    }
}
