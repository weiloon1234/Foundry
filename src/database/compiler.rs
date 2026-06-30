use crate::foundation::{Error, Result};

use super::ast::{
    AggregateExpr, AggregateFn, AggregateNode, BinaryExpr, BinaryOperator, CaseExpr, ColumnRef,
    ComparisonOp, Condition, CteMaterialization, CteNode, DbType, DbValue, DeleteNode, Expr,
    FromItem, InsertNode, InsertSource, JoinKind, JsonPathExpr, JsonPathMode, JsonPathSegment,
    JsonPredicateOp, JsonPredicateValue, LockBehavior, LockClause, LockStrength, OnConflictAction,
    OnConflictNode, OnConflictTarget, OrderBy, OrderDirection, QueryAst, QueryBody, SelectItem,
    SelectNode, SetOperationNode, SetOperator, TableRef, UnaryExpr, UnaryOperator, UpdateNode,
    WindowFrameBound, WindowFrameUnits, WindowSpec,
};

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledSql {
    pub sql: String,
    pub bindings: Vec<DbValue>,
}

pub struct PostgresCompiler;

impl PostgresCompiler {
    pub fn compile(ast: &QueryAst) -> Result<CompiledSql> {
        let mut compiler = CompilerState::default();
        let sql = compiler.compile_query(ast)?;
        Ok(CompiledSql {
            sql,
            bindings: compiler.bindings,
        })
    }
}

#[derive(Default)]
struct CompilerState {
    bindings: Vec<DbValue>,
}

impl CompilerState {
    fn compile_query(&mut self, ast: &QueryAst) -> Result<String> {
        let mut sql = String::new();
        if !ast.with.is_empty() {
            sql.push_str("WITH ");
            if ast.with.iter().any(|cte| cte.recursive) {
                sql.push_str("RECURSIVE ");
            }
            sql.push_str(
                &ast.with
                    .iter()
                    .map(|cte| self.compile_cte(cte))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
            sql.push(' ');
        }
        sql.push_str(&self.compile_query_body(&ast.body)?);
        Ok(sql)
    }

    fn compile_cte(&mut self, cte: &CteNode) -> Result<String> {
        let mut sql = quote_identifier(&cte.name);
        sql.push_str(" AS ");
        if let Some(materialization) = cte.materialization {
            sql.push_str(match materialization {
                CteMaterialization::Materialized => "MATERIALIZED ",
                CteMaterialization::NotMaterialized => "NOT MATERIALIZED ",
            });
        }
        sql.push('(');
        sql.push_str(&self.compile_query(&cte.query)?);
        sql.push(')');
        Ok(sql)
    }

    fn compile_query_body(&mut self, body: &QueryBody) -> Result<String> {
        match body {
            QueryBody::Select(select) => self.compile_select(select),
            QueryBody::Insert(insert) => self.compile_insert(insert),
            QueryBody::Update(update) => self.compile_update(update),
            QueryBody::Delete(delete) => self.compile_delete(delete),
            QueryBody::SetOperation(set) => self.compile_set_operation(set),
        }
    }

    fn compile_select(&mut self, select: &SelectNode) -> Result<String> {
        let mut projection = Vec::new();
        if select.columns.is_empty() && select.aggregates.is_empty() {
            projection.push("*".to_string());
        } else {
            projection.extend(
                select
                    .columns
                    .iter()
                    .map(|item| self.compile_select_item(item))
                    .collect::<Result<Vec<_>>>()?,
            );
            projection.extend(
                select
                    .aggregates
                    .iter()
                    .map(|aggregate| self.compile_aggregate_projection(aggregate))
                    .collect::<Result<Vec<_>>>()?,
            );
        }

        let mut sql = String::from("SELECT ");
        if select.distinct {
            sql.push_str("DISTINCT ");
        }
        sql.push_str(&projection.join(", "));
        sql.push_str(" FROM ");
        sql.push_str(&self.compile_from_item(&select.from)?);

        if !select.joins.is_empty() {
            for join in &select.joins {
                let join_sql = match join.kind {
                    JoinKind::Inner => "INNER JOIN",
                    JoinKind::Left => "LEFT JOIN",
                    JoinKind::Right => "RIGHT JOIN",
                    JoinKind::Full => "FULL OUTER JOIN",
                    JoinKind::Cross => "CROSS JOIN",
                };
                sql.push(' ');
                sql.push_str(join_sql);
                if join.lateral {
                    sql.push_str(" LATERAL");
                }
                sql.push(' ');
                sql.push_str(&self.compile_from_item(&join.table)?);
                match (&join.kind, &join.on) {
                    (JoinKind::Cross, None) => {}
                    (JoinKind::Cross, Some(_)) => {
                        return Err(Error::message("cross join does not support ON conditions"));
                    }
                    (_, Some(on)) => {
                        sql.push_str(" ON ");
                        sql.push_str(&self.compile_condition(on)?);
                    }
                    (_, None) => {
                        return Err(Error::message(
                            "joined queries require an ON condition unless using CROSS JOIN",
                        ));
                    }
                }
            }
        }

        if let Some(condition) = &select.condition {
            sql.push_str(" WHERE ");
            sql.push_str(&self.compile_condition(condition)?);
        }

        if !select.group_by.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(
                &select
                    .group_by
                    .iter()
                    .map(|expr| self.compile_expr(expr))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
        }

        if let Some(condition) = &select.having {
            sql.push_str(" HAVING ");
            sql.push_str(&self.compile_condition(condition)?);
        }

        self.push_order_limit_offset(&mut sql, &select.order_by, select.limit, select.offset)?;
        if let Some(lock) = &select.lock {
            sql.push(' ');
            sql.push_str(&self.compile_lock(lock)?);
        }

        Ok(sql)
    }

    fn compile_set_operation(&mut self, set: &SetOperationNode) -> Result<String> {
        let mut sql = format!(
            "({}) {} ({})",
            self.compile_query(&set.left)?,
            match set.operator {
                SetOperator::Union => "UNION",
                SetOperator::UnionAll => "UNION ALL",
            },
            self.compile_query(&set.right)?,
        );

        self.push_order_limit_offset(&mut sql, &set.order_by, set.limit, set.offset)?;
        Ok(sql)
    }

    fn push_order_limit_offset(
        &mut self,
        sql: &mut String,
        order_by: &[OrderBy],
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<()> {
        if !order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(
                &order_by
                    .iter()
                    .map(|order| self.compile_order(order))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
        }

        if let Some(limit) = limit {
            sql.push_str(" LIMIT ");
            sql.push_str(&self.bind_value(DbValue::Int64(limit as i64)));
        }

        if let Some(offset) = offset {
            sql.push_str(" OFFSET ");
            sql.push_str(&self.bind_value(DbValue::Int64(offset as i64)));
        }

        Ok(())
    }

    fn compile_insert(&mut self, insert: &InsertNode) -> Result<String> {
        let (columns, source_sql) = match &insert.source {
            InsertSource::Values(rows) => {
                let (columns, rows) = self.normalize_insert_rows(rows)?;
                if rows.is_empty() {
                    return Err(Error::message("insert query requires at least one value"));
                }

                let row_sql = rows
                    .iter()
                    .map(|row| {
                        Ok(format!(
                            "({})",
                            row.iter()
                                .map(|expr| self.compile_expr(expr))
                                .collect::<Result<Vec<_>>>()?
                                .join(", ")
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                (columns, format!("VALUES {}", row_sql.join(", ")))
            }
            InsertSource::Select(query) => {
                let select = match &query.body {
                    QueryBody::Select(select) => select,
                    QueryBody::SetOperation(_) => {
                        return Err(Error::message(
                            "insert select source requires explicit target columns; use the insert target columns as the source projection order",
                        ));
                    }
                    QueryBody::Insert(_) | QueryBody::Update(_) | QueryBody::Delete(_) => {
                        return Err(Error::message(
                            "insert select source must be a select or set-operation query",
                        ));
                    }
                };
                if select.columns.is_empty() {
                    return Err(Error::message(
                        "insert select queries require explicit selected columns",
                    ));
                }
                let columns = select
                    .columns
                    .iter()
                    .map(|item| match &item.expr {
                        Expr::Column(column) => Ok(column.clone()),
                        _ => Err(Error::message(
                            "insert select queries require direct column expressions",
                        )),
                    })
                    .collect::<Result<Vec<_>>>()?;
                (columns, self.compile_query(query)?)
            }
        };

        let mut sql = format!(
            "INSERT INTO {} ({}) {}",
            self.compile_table(&insert.into),
            columns
                .iter()
                .map(|column| quote_identifier(&column.name))
                .collect::<Vec<_>>()
                .join(", "),
            source_sql
        );

        if let Some(on_conflict) = &insert.on_conflict {
            sql.push(' ');
            sql.push_str(&self.compile_on_conflict(on_conflict)?);
        }

        if !insert.returning.is_empty() {
            sql.push_str(" RETURNING ");
            sql.push_str(
                &insert
                    .returning
                    .iter()
                    .map(|item| self.compile_select_item(item))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
        }

        Ok(sql)
    }

    fn normalize_insert_rows(
        &mut self,
        rows: &[Vec<(ColumnRef, Expr)>],
    ) -> Result<(Vec<ColumnRef>, Vec<Vec<Expr>>)> {
        if rows.is_empty() {
            return Err(Error::message("insert query requires at least one row"));
        }

        let first_row = rows
            .first()
            .ok_or_else(|| Error::message("insert query requires at least one row"))?;
        if first_row.is_empty() {
            return Err(Error::message("insert query requires at least one value"));
        }

        let mut canonical_columns = Vec::with_capacity(first_row.len());
        let mut canonical_names = Vec::with_capacity(first_row.len());
        for (column, _) in first_row {
            if canonical_names.iter().any(|name| name == &column.name) {
                return Err(Error::message(format!(
                    "insert row contains duplicate column `{}`",
                    column.name
                )));
            }
            canonical_names.push(column.name.clone());
            canonical_columns.push(column.clone());
        }

        let mut normalized_rows = Vec::with_capacity(rows.len());
        for row in rows {
            if row.is_empty() {
                return Err(Error::message("insert query requires at least one value"));
            }

            let mut ordered = Vec::with_capacity(canonical_columns.len());
            for column_name in &canonical_names {
                let mut matched = row.iter().filter(|(column, _)| &column.name == column_name);
                let (_, expr) = matched.next().ok_or_else(|| {
                    Error::message(format!(
                        "insert row is missing required column `{column_name}`"
                    ))
                })?;
                if matched.next().is_some() {
                    return Err(Error::message(format!(
                        "insert row contains duplicate column `{column_name}`"
                    )));
                }
                ordered.push(expr.clone());
            }

            if row.len() != canonical_columns.len() {
                return Err(Error::message(
                    "insert rows must use the same set of columns",
                ));
            }

            normalized_rows.push(ordered);
        }

        Ok((canonical_columns, normalized_rows))
    }

    fn compile_on_conflict(&mut self, on_conflict: &OnConflictNode) -> Result<String> {
        let target = match &on_conflict.target {
            Some(OnConflictTarget::Columns(columns)) => format!(
                "({})",
                columns
                    .iter()
                    .map(|column| quote_identifier(&column.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some(OnConflictTarget::Constraint(constraint)) => {
                format!("ON CONSTRAINT {}", quote_identifier(constraint))
            }
            None => String::new(),
        };

        let action = match &on_conflict.action {
            OnConflictAction::DoNothing => "DO NOTHING".to_string(),
            OnConflictAction::DoUpdate(update) => {
                if on_conflict.target.is_none() {
                    return Err(Error::message(
                        "on conflict do update requires conflict columns or a named constraint",
                    ));
                }
                if update.assignments.is_empty() {
                    return Err(Error::message(
                        "on conflict do update requires at least one assignment",
                    ));
                }

                let mut sql = format!(
                    "DO UPDATE SET {}",
                    update
                        .assignments
                        .iter()
                        .map(|(column, expr)| {
                            Ok(format!(
                                "{} = {}",
                                quote_identifier(&column.name),
                                self.compile_expr(expr)?
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?
                        .join(", ")
                );
                if let Some(condition) = &update.condition {
                    sql.push_str(" WHERE ");
                    sql.push_str(&self.compile_condition(condition)?);
                }
                sql
            }
        };

        Ok(if target.is_empty() {
            format!("ON CONFLICT {action}")
        } else {
            format!("ON CONFLICT {target} {action}")
        })
    }

    fn compile_update(&mut self, update: &UpdateNode) -> Result<String> {
        if update.values.is_empty() {
            return Err(Error::message("update query requires at least one value"));
        }

        let assignments = update
            .values
            .iter()
            .map(|(column, expr)| {
                Ok(format!(
                    "{} = {}",
                    quote_identifier(&column.name),
                    self.compile_expr(expr)?
                ))
            })
            .collect::<Result<Vec<_>>>()?;

        let mut sql = format!(
            "UPDATE {} SET {}",
            self.compile_table(&update.table),
            assignments.join(", ")
        );

        if !update.from.is_empty() {
            sql.push_str(" FROM ");
            sql.push_str(
                &update
                    .from
                    .iter()
                    .map(|item| self.compile_from_item(item))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
        }

        if let Some(condition) = &update.condition {
            sql.push_str(" WHERE ");
            sql.push_str(&self.compile_condition(condition)?);
        }

        if !update.returning.is_empty() {
            sql.push_str(" RETURNING ");
            sql.push_str(
                &update
                    .returning
                    .iter()
                    .map(|item| self.compile_select_item(item))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
        }

        Ok(sql)
    }

    fn compile_delete(&mut self, delete: &DeleteNode) -> Result<String> {
        let mut sql = format!("DELETE FROM {}", self.compile_table(&delete.from));

        if !delete.using.is_empty() {
            sql.push_str(" USING ");
            sql.push_str(
                &delete
                    .using
                    .iter()
                    .map(|item| self.compile_from_item(item))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
        }

        if let Some(condition) = &delete.condition {
            sql.push_str(" WHERE ");
            sql.push_str(&self.compile_condition(condition)?);
        }

        if !delete.returning.is_empty() {
            sql.push_str(" RETURNING ");
            sql.push_str(
                &delete
                    .returning
                    .iter()
                    .map(|item| self.compile_select_item(item))
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
            );
        }

        Ok(sql)
    }

    fn compile_from_item(&mut self, item: &FromItem) -> Result<String> {
        match item {
            FromItem::Table(table) => Ok(self.compile_table(table)),
            FromItem::Subquery { query, alias } => Ok(format!(
                "({}) AS {}",
                self.compile_query(query)?,
                quote_identifier(alias),
            )),
        }
    }

    fn compile_select_item(&mut self, item: &SelectItem) -> Result<String> {
        let (mut expression, expr_alias) = self.compile_projection_expr(&item.expr)?;
        if matches!(self.expr_db_type(&item.expr), Some(DbType::Numeric)) {
            expression = format!("({expression})::text");
        }

        if let Some(alias) = item.alias.as_deref().or(expr_alias.as_deref()) {
            Ok(format!("{expression} AS {}", quote_identifier(alias)))
        } else {
            Ok(expression)
        }
    }

    fn compile_aggregate_projection(&mut self, aggregate: &AggregateNode) -> Result<String> {
        let mut expression = self.compile_aggregate_expr(&aggregate.aggregate)?;
        if matches!(
            self.expr_db_type(&Expr::Aggregate(aggregate.aggregate.clone())),
            Some(DbType::Numeric)
        ) {
            expression = format!("({expression})::text");
        }
        Ok(format!(
            "{} AS {}",
            expression,
            quote_identifier(&aggregate.alias),
        ))
    }

    fn compile_projection_expr(&mut self, expr: &Expr) -> Result<(String, Option<String>)> {
        match expr {
            Expr::Column(column) => Ok((self.compile_column_name(column), column.alias.clone())),
            _ => Ok((self.compile_expr(expr)?, None)),
        }
    }

    fn compile_aggregate_expr(&mut self, aggregate: &AggregateExpr) -> Result<String> {
        let target = match (&aggregate.function, &aggregate.expr) {
            (AggregateFn::Count, None) if !aggregate.distinct => "*".to_string(),
            (AggregateFn::Count, None) if aggregate.distinct => {
                return Err(Error::message("count distinct requires an expression"));
            }
            (_, Some(expr)) => {
                let compiled = self.compile_expr(expr)?;
                if aggregate.distinct {
                    format!("DISTINCT {compiled}")
                } else {
                    compiled
                }
            }
            _ => {
                return Err(Error::message(
                    "aggregate function requires an expression unless using count(*)",
                ));
            }
        };

        Ok(format!(
            "{}({target})",
            match aggregate.function {
                AggregateFn::Count => "COUNT",
                AggregateFn::Sum => "SUM",
                AggregateFn::Avg => "AVG",
                AggregateFn::Min => "MIN",
                AggregateFn::Max => "MAX",
            },
        ))
    }

    fn compile_case_expr(&mut self, case: &CaseExpr) -> Result<String> {
        if case.whens.is_empty() {
            return Err(Error::message(
                "case expression requires at least one when branch",
            ));
        }

        let mut sql = String::from("CASE");
        for branch in &case.whens {
            sql.push_str(" WHEN ");
            sql.push_str(&self.compile_condition(&branch.condition)?);
            sql.push_str(" THEN ");
            sql.push_str(&self.compile_expr(&branch.result)?);
        }

        if let Some(else_expr) = &case.else_expr {
            sql.push_str(" ELSE ");
            sql.push_str(&self.compile_expr(else_expr)?);
        }

        sql.push_str(" END");
        Ok(sql)
    }

    fn compile_function(&mut self, name: &str, args: &[Expr]) -> Result<String> {
        if name.eq_ignore_ascii_case("EXTRACT") {
            if args.len() != 2 {
                return Err(Error::message(
                    "EXTRACT requires a field name and expression argument",
                ));
            }

            let field = match &args[0] {
                Expr::Value(DbValue::Text(field)) => field.as_str(),
                _ => {
                    return Err(Error::message(
                        "EXTRACT requires the first argument to be a text field name",
                    ));
                }
            };
            validate_extract_field(field)?;

            return Ok(format!(
                "EXTRACT({} FROM {})",
                field,
                self.compile_expr(&args[1])?
            ));
        }

        if name.eq_ignore_ascii_case("JSONB_TEXT_OR_FIRST") {
            if args.len() != 2 {
                return Err(Error::message(
                    "JSONB_TEXT_OR_FIRST requires a JSONB expression and preferred key",
                ));
            }

            let preferred_key = match &args[1] {
                Expr::Value(DbValue::Text(key)) => key.as_str(),
                _ => {
                    return Err(Error::message(
                        "JSONB_TEXT_OR_FIRST requires the second argument to be a text key",
                    ));
                }
            };
            let expr = self.compile_expr(&args[0])?;
            return Ok(format!(
                "COALESCE(({expr})->>{}, (SELECT value FROM jsonb_each_text({expr}) LIMIT 1))",
                self.bind_text(preferred_key),
            ));
        }

        Ok(format!(
            "{}({})",
            name,
            args.iter()
                .map(|expr| self.compile_expr(expr))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        ))
    }

    fn compile_unary_expr(&mut self, expr: &UnaryExpr) -> Result<String> {
        Ok(match expr.operator {
            UnaryOperator::Not => format!("NOT ({})", self.compile_expr(&expr.expr)?),
            UnaryOperator::Negate => format!("-({})", self.compile_expr(&expr.expr)?),
        })
    }

    fn compile_binary_expr(&mut self, expr: &BinaryExpr) -> Result<String> {
        Ok(format!(
            "({} {} {})",
            self.compile_expr(&expr.left)?,
            match &expr.operator {
                BinaryOperator::Add => "+",
                BinaryOperator::Subtract => "-",
                BinaryOperator::Multiply => "*",
                BinaryOperator::Divide => "/",
                BinaryOperator::Concat => "||",
                BinaryOperator::Custom(operator) => {
                    validate_custom_operator(operator)?;
                    operator.as_str()
                }
            },
            self.compile_expr(&expr.right)?,
        ))
    }

    fn compile_window_spec(&mut self, window: &WindowSpec) -> Result<String> {
        let mut segments = Vec::new();
        if !window.partition_by.is_empty() {
            segments.push(format!(
                "PARTITION BY {}",
                window
                    .partition_by
                    .iter()
                    .map(|expr| self.compile_expr(expr))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ")
            ));
        }
        if !window.order_by.is_empty() {
            segments.push(format!(
                "ORDER BY {}",
                window
                    .order_by
                    .iter()
                    .map(|order| self.compile_order(order))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ")
            ));
        }
        if let Some(frame) = &window.frame {
            let start = self.compile_window_frame_bound(&frame.start);
            let frame_sql = if let Some(end) = &frame.end {
                format!(
                    "{} BETWEEN {} AND {}",
                    match frame.units {
                        WindowFrameUnits::Rows => "ROWS",
                        WindowFrameUnits::Range => "RANGE",
                    },
                    start,
                    self.compile_window_frame_bound(end)
                )
            } else {
                format!(
                    "{} {}",
                    match frame.units {
                        WindowFrameUnits::Rows => "ROWS",
                        WindowFrameUnits::Range => "RANGE",
                    },
                    start
                )
            };
            segments.push(frame_sql);
        }
        Ok(segments.join(" "))
    }

    fn compile_window_frame_bound(&self, bound: &WindowFrameBound) -> String {
        match bound {
            WindowFrameBound::UnboundedPreceding => "UNBOUNDED PRECEDING".to_string(),
            WindowFrameBound::Preceding(value) => format!("{value} PRECEDING"),
            WindowFrameBound::CurrentRow => "CURRENT ROW".to_string(),
            WindowFrameBound::Following(value) => format!("{value} FOLLOWING"),
            WindowFrameBound::UnboundedFollowing => "UNBOUNDED FOLLOWING".to_string(),
        }
    }

    fn compile_lock(&self, lock: &LockClause) -> Result<String> {
        let mut sql = match lock.strength {
            LockStrength::Update => "FOR UPDATE".to_string(),
            LockStrength::NoKeyUpdate => "FOR NO KEY UPDATE".to_string(),
            LockStrength::Share => "FOR SHARE".to_string(),
            LockStrength::KeyShare => "FOR KEY SHARE".to_string(),
        };

        if !lock.of.is_empty() {
            sql.push_str(" OF ");
            sql.push_str(
                &lock
                    .of
                    .iter()
                    .map(|alias| quote_identifier(alias))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }

        match lock.behavior {
            LockBehavior::Wait => {}
            LockBehavior::NoWait => sql.push_str(" NOWAIT"),
            LockBehavior::SkipLocked => sql.push_str(" SKIP LOCKED"),
        }

        Ok(sql)
    }

    fn compile_json_path_expr(&mut self, path: &JsonPathExpr) -> Result<String> {
        let mut sql = self.compile_expr(&path.expr)?;
        if path.path.is_empty() {
            return Ok(match path.mode {
                JsonPathMode::Json => sql,
                JsonPathMode::Text => format!("({sql})::text"),
            });
        }

        for (index, segment) in path.path.iter().enumerate() {
            let is_last = index == path.path.len() - 1;
            let operator = if is_last && path.mode == JsonPathMode::Text {
                "->>"
            } else {
                "->"
            };
            sql = match segment {
                JsonPathSegment::Key(key) => format!("({sql}) {operator} {}", self.bind_text(key)),
                JsonPathSegment::Index(index) => format!("({sql}) {operator} {index}"),
            };
        }

        Ok(sql)
    }

    fn compile_order(&mut self, order: &OrderBy) -> Result<String> {
        Ok(format!(
            "{} {}",
            self.compile_expr(&order.expr)?,
            match order.direction {
                OrderDirection::Asc => "ASC",
                OrderDirection::Desc => "DESC",
            }
        ))
    }

    fn compile_condition(&mut self, condition: &Condition) -> Result<String> {
        match condition {
            Condition::Comparison { left, op, right } => match op {
                ComparisonOp::IEq => Ok(format!(
                    "LOWER({}) = LOWER({})",
                    self.compile_expr(left)?,
                    self.compile_expr(right)?,
                )),
                _ => Ok(format!(
                    "{} {} {}",
                    self.compile_expr(left)?,
                    self.compile_comparison_op(*op),
                    self.compile_expr(right)?,
                )),
            },
            Condition::InList { expr, values } => {
                if values.is_empty() {
                    return Ok("FALSE".to_string());
                }
                Ok(format!(
                    "{} IN ({})",
                    self.compile_expr(expr)?,
                    values
                        .iter()
                        .cloned()
                        .map(|value| self.bind_value(value))
                        .collect::<Vec<_>>()
                        .join(", "),
                ))
            }
            Condition::JsonPredicate { expr, op, value } => {
                let expr = self.compile_expr(expr)?;
                Ok(match (op, value) {
                    (JsonPredicateOp::Contains, JsonPredicateValue::Json(value)) => {
                        format!(
                            "{expr} @> {}",
                            self.bind_value(DbValue::Json(value.clone()))
                        )
                    }
                    (JsonPredicateOp::ContainedBy, JsonPredicateValue::Json(value)) => {
                        format!(
                            "{expr} <@ {}",
                            self.bind_value(DbValue::Json(value.clone()))
                        )
                    }
                    (JsonPredicateOp::HasKey, JsonPredicateValue::Key(key)) => {
                        format!("{expr} ? {}", self.bind_text(key))
                    }
                    (JsonPredicateOp::HasAnyKeys, JsonPredicateValue::Keys(keys)) => {
                        format!("{expr} ?| {}", self.bind_text_array(keys))
                    }
                    (JsonPredicateOp::HasAllKeys, JsonPredicateValue::Keys(keys)) => {
                        format!("{expr} ?& {}", self.bind_text_array(keys))
                    }
                    _ => {
                        return Err(Error::message(
                            "json predicate received an incompatible value shape",
                        ));
                    }
                })
            }
            Condition::FullText { columns, query } => {
                if columns.is_empty() {
                    return Ok("FALSE".to_string());
                }

                let document = columns
                    .iter()
                    .map(|column| {
                        format!("COALESCE({}::text, '')", self.compile_column_name(column))
                    })
                    .collect::<Vec<_>>()
                    .join(" || ' ' || ");

                Ok(format!(
                    "to_tsvector('english'::regconfig, {document}) @@ plainto_tsquery('english'::regconfig, {})",
                    self.bind_text(query),
                ))
            }
            Condition::And(conditions) => {
                if conditions.is_empty() {
                    return Ok("TRUE".to_string());
                }

                Ok(format!(
                    "({})",
                    conditions
                        .iter()
                        .map(|condition| self.compile_condition(condition))
                        .collect::<Result<Vec<_>>>()?
                        .join(" AND "),
                ))
            }
            Condition::Or(conditions) => {
                if conditions.is_empty() {
                    return Ok("FALSE".to_string());
                }

                Ok(format!(
                    "({})",
                    conditions
                        .iter()
                        .map(|condition| self.compile_condition(condition))
                        .collect::<Result<Vec<_>>>()?
                        .join(" OR "),
                ))
            }
            Condition::Not(condition) => {
                Ok(format!("NOT ({})", self.compile_condition(condition)?))
            }
            Condition::IsNull(expr) => Ok(format!("{} IS NULL", self.compile_expr(expr)?)),
            Condition::IsNotNull(expr) => Ok(format!("{} IS NOT NULL", self.compile_expr(expr)?)),
            Condition::IsTrue(expr) => Ok(format!("{} IS TRUE", self.compile_expr(expr)?)),
            Condition::IsFalse(expr) => Ok(format!("{} IS FALSE", self.compile_expr(expr)?)),
            Condition::Exists(query) => Ok(format!("EXISTS ({})", self.compile_query(query)?)),
            Condition::Raw { sql, bindings } => {
                let placeholder_count = sql.matches('?').count();
                if placeholder_count != bindings.len() {
                    return Err(Error::message(format!(
                        "raw condition placeholder count mismatch: expected {placeholder_count} bindings, got {}",
                        bindings.len()
                    )));
                }

                let mut compiled = String::new();
                let mut binding_iter = bindings.iter();
                for (index, segment) in sql.split('?').enumerate() {
                    compiled.push_str(segment);
                    if index < placeholder_count {
                        if let Some(value) = binding_iter.next() {
                            compiled.push_str(&self.bind_value(value.clone()));
                        }
                    }
                }
                Ok(compiled)
            }
        }
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<String> {
        match expr {
            Expr::Column(column) => Ok(self.compile_column(column)),
            Expr::Excluded(column) => Ok(format!("EXCLUDED.{}", quote_identifier(&column.name))),
            Expr::Value(value) => Ok(self.bind_value(value.clone())),
            Expr::Cast { expr, db_type } => Ok(format!(
                "({})::{}",
                self.compile_expr(expr)?,
                db_type.postgres_cast()
            )),
            Expr::Aggregate(aggregate) => self.compile_aggregate_expr(aggregate),
            Expr::Function(function) => self.compile_function(&function.name, &function.args),
            Expr::Unary(expr) => self.compile_unary_expr(expr),
            Expr::Binary(expr) => self.compile_binary_expr(expr),
            Expr::Subquery(query) => Ok(format!("({})", self.compile_query(query)?)),
            Expr::Window(window) => Ok(format!(
                "{} OVER ({})",
                self.compile_expr(&window.function)?,
                self.compile_window_spec(&window.window)?
            )),
            Expr::Case(case) => self.compile_case_expr(case),
            Expr::JsonPath(path) => self.compile_json_path_expr(path),
            Expr::Raw(sql) => Ok(sql.clone()),
        }
    }

    fn expr_db_type(&self, expr: &Expr) -> Option<DbType> {
        match expr {
            Expr::Column(column) => column.db_type,
            Expr::Excluded(column) => column.db_type,
            Expr::Value(value) => Some(value.db_type()),
            Expr::Cast { db_type, .. } => Some(*db_type),
            Expr::Aggregate(aggregate) => match aggregate.function {
                AggregateFn::Count => Some(DbType::Int64),
                AggregateFn::Sum | AggregateFn::Avg => aggregate
                    .expr
                    .as_ref()
                    .and_then(|expr| self.expr_db_type(expr))
                    .map(|db_type| match db_type {
                        DbType::Int16 | DbType::Int32 | DbType::Int64 | DbType::Numeric => {
                            DbType::Numeric
                        }
                        other => other,
                    }),
                AggregateFn::Min | AggregateFn::Max => aggregate
                    .expr
                    .as_ref()
                    .and_then(|expr| self.expr_db_type(expr)),
            },
            Expr::Function(function)
                if function.name.eq_ignore_ascii_case("JSONB_TEXT_OR_FIRST") =>
            {
                Some(DbType::Text)
            }
            Expr::Function(_) => None,
            Expr::Unary(expr) => self.expr_db_type(&expr.expr),
            Expr::Binary(expr) => self
                .expr_db_type(&expr.left)
                .or_else(|| self.expr_db_type(&expr.right)),
            Expr::Subquery(_) => None,
            Expr::Window(_) => None,
            Expr::Case(case) => case
                .else_expr
                .as_ref()
                .and_then(|expr| self.expr_db_type(expr))
                .or_else(|| {
                    case.whens
                        .first()
                        .and_then(|when| self.expr_db_type(&when.result))
                }),
            Expr::JsonPath(path) => Some(match path.mode {
                JsonPathMode::Json => DbType::Json,
                JsonPathMode::Text => DbType::Text,
            }),
            Expr::Raw(_) => None,
        }
    }

    fn compile_column(&self, column: &ColumnRef) -> String {
        let mut sql = self.compile_column_name(column);
        if let Some(alias) = &column.alias {
            sql.push_str(" AS ");
            sql.push_str(&quote_identifier(alias));
        }
        sql
    }

    fn compile_column_name(&self, column: &ColumnRef) -> String {
        match &column.table {
            Some(table) => format!(
                "{}.{}",
                quote_identifier(table),
                quote_identifier(&column.name)
            ),
            None => quote_identifier(&column.name),
        }
    }

    fn compile_table(&self, table: &TableRef) -> String {
        match &table.alias {
            Some(alias) => format!(
                "{} AS {}",
                quote_identifier(&table.name),
                quote_identifier(alias)
            ),
            None => quote_identifier(&table.name),
        }
    }

    fn compile_comparison_op(&self, op: ComparisonOp) -> &'static str {
        match op {
            ComparisonOp::Eq => "=",
            ComparisonOp::IEq => unreachable!("IEq is compiled explicitly in compile_condition"),
            ComparisonOp::NotEq => "<>",
            ComparisonOp::Gt => ">",
            ComparisonOp::Gte => ">=",
            ComparisonOp::Lt => "<",
            ComparisonOp::Lte => "<=",
            ComparisonOp::Like => "LIKE",
            ComparisonOp::NotLike => "NOT LIKE",
            ComparisonOp::ILike => "ILIKE",
        }
    }

    fn bind_text(&mut self, value: &str) -> String {
        self.bind_value(DbValue::Text(value.to_string()))
    }

    fn bind_text_array(&mut self, values: &[String]) -> String {
        format!(
            "ARRAY[{}]",
            values
                .iter()
                .map(|value| self.bind_text(value))
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    fn bind_value(&mut self, value: DbValue) -> String {
        self.bindings.push(value.clone());
        format!(
            "${}::{}",
            self.bindings.len(),
            value.db_type().postgres_cast()
        )
    }
}

fn quote_identifier(identifier: &str) -> String {
    identifier
        .split('.')
        .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}

/// Fields accepted by Postgres `EXTRACT(<field> FROM ...)`. The field is a
/// keyword token, not a bindable value or quotable identifier, so it must be
/// validated against this list before being written into the SQL text.
const EXTRACT_FIELDS: &[&str] = &[
    "century",
    "day",
    "decade",
    "dow",
    "doy",
    "epoch",
    "hour",
    "isodow",
    "isoyear",
    "julian",
    "microseconds",
    "millennium",
    "milliseconds",
    "minute",
    "month",
    "quarter",
    "second",
    "timezone",
    "timezone_hour",
    "timezone_minute",
    "week",
    "year",
];

fn validate_extract_field(field: &str) -> Result<()> {
    if EXTRACT_FIELDS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(field))
    {
        Ok(())
    } else {
        Err(Error::message(format!(
            "EXTRACT field `{}` is not a recognized date/time field",
            field
        )))
    }
}

/// Custom operators are written into the SQL text verbatim, so restrict them
/// to characters that can form Postgres operators or keyword operators
/// (e.g. `->>`, `@>`, `ILIKE`, `IS DISTINCT FROM`) and reject anything that
/// could open a comment or break out of the expression.
fn validate_custom_operator(operator: &str) -> Result<()> {
    const OPERATOR_SYMBOLS: &str = "+-*/<>=~!@#%^&|`?";
    let valid_chars = !operator.is_empty()
        && operator.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || ch == '_' || ch == ' ' || OPERATOR_SYMBOLS.contains(ch)
        });
    if !valid_chars || operator.contains("--") || operator.contains("/*") {
        return Err(Error::message(format!(
            "custom SQL operator `{}` contains unsupported characters",
            operator
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CompiledSql, PostgresCompiler};
    use crate::database::ast::{
        AggregateExpr, AggregateNode, CaseExpr, CaseWhen, ColumnRef, ComparisonOp, Condition,
        CteMaterialization, CteNode, DbType, DbValue, DeleteNode, Expr, FromItem, InsertNode,
        InsertSource, JoinKind, JoinNode, JsonPathExpr, JsonPathMode, JsonPathSegment,
        JsonPredicateOp, JsonPredicateValue, LockBehavior, LockClause, LockStrength, Numeric,
        OnConflictAction, OnConflictNode, OnConflictTarget, OnConflictUpdate, OrderBy, QueryAst,
        QueryBody, SelectItem, SelectNode, SetOperationNode, SetOperator, TableRef, UpdateNode,
        WindowFrame, WindowFrameBound, WindowFrameUnits, WindowSpec,
    };

    fn compile(ast: QueryAst) -> CompiledSql {
        PostgresCompiler::compile(&ast).unwrap()
    }

    #[test]
    fn extract_field_is_validated_against_known_fields() {
        assert!(super::validate_extract_field("year").is_ok());
        assert!(super::validate_extract_field("EPOCH").is_ok());
        assert!(super::validate_extract_field("timezone_hour").is_ok());
        assert!(super::validate_extract_field("year FROM now()); DROP TABLE users; --").is_err());
        assert!(super::validate_extract_field("").is_err());
    }

    #[test]
    fn custom_operator_rejects_injection_shaped_input() {
        assert!(super::validate_custom_operator("->>").is_ok());
        assert!(super::validate_custom_operator("@>").is_ok());
        assert!(super::validate_custom_operator("ILIKE").is_ok());
        assert!(super::validate_custom_operator("IS DISTINCT FROM").is_ok());
        assert!(super::validate_custom_operator("").is_err());
        assert!(super::validate_custom_operator("= 1; DROP TABLE users; --").is_err());
        assert!(super::validate_custom_operator("--").is_err());
        assert!(super::validate_custom_operator("/*").is_err());
        assert!(super::validate_custom_operator("= (SELECT 1)").is_err());
    }

    #[test]
    fn compiles_select_with_join_and_nested_conditions() {
        let ast = QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![
                SelectItem::new(Expr::column(ColumnRef::new("users", "id"))),
                SelectItem::new(Expr::column(ColumnRef::new("users", "email"))),
            ],
            joins: vec![JoinNode {
                kind: JoinKind::Left,
                table: FromItem::Table(TableRef::new("profiles")),
                lateral: false,
                on: Some(Condition::compare(
                    Expr::column(ColumnRef::new("profiles", "user_id")),
                    ComparisonOp::Eq,
                    Expr::column(ColumnRef::new("users", "id")),
                )),
            }],
            condition: Some(Condition::and([
                Condition::compare(
                    Expr::column(ColumnRef::new("users", "active")),
                    ComparisonOp::Eq,
                    Expr::value(true),
                ),
                Condition::or([
                    Condition::compare(
                        Expr::column(ColumnRef::new("users", "role")),
                        ComparisonOp::Eq,
                        Expr::value("admin"),
                    ),
                    Condition::compare(
                        Expr::column(ColumnRef::new("users", "role")),
                        ComparisonOp::Eq,
                        Expr::value("editor"),
                    ),
                ]),
            ])),
            group_by: Vec::new(),
            having: None,
            order_by: vec![OrderBy::desc(Expr::column(ColumnRef::new("users", "id")))],
            limit: Some(20),
            offset: Some(40),
            lock: None,
            relations: Vec::new(),
            aggregates: Vec::new(),
        });

        let compiled = compile(ast);
        assert_eq!(
            compiled.sql,
            "SELECT \"users\".\"id\", \"users\".\"email\" FROM \"users\" LEFT JOIN \"profiles\" ON \"profiles\".\"user_id\" = \"users\".\"id\" WHERE (\"users\".\"active\" = $1::boolean AND (\"users\".\"role\" = $2::text OR \"users\".\"role\" = $3::text)) ORDER BY \"users\".\"id\" DESC LIMIT $4::bigint OFFSET $5::bigint"
        );
    }

    #[test]
    fn raw_condition_requires_exact_binding_count() {
        let ast = QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(ColumnRef::new("users", "id")))],
            joins: Vec::new(),
            condition: Some(Condition::raw(
                "email = ? AND status = ?",
                vec![DbValue::Text("a@example.com".to_string())],
            )),
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
            relations: Vec::new(),
            aggregates: Vec::new(),
        });

        let error = PostgresCompiler::compile(&ast).unwrap_err();

        assert!(error
            .to_string()
            .contains("raw condition placeholder count mismatch: expected 2 bindings, got 1"));
    }

    #[test]
    fn raw_condition_rejects_extra_bindings() {
        let ast = QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(ColumnRef::new("users", "id")))],
            joins: Vec::new(),
            condition: Some(Condition::raw(
                "email = ?",
                vec![
                    DbValue::Text("a@example.com".to_string()),
                    DbValue::Text("active".to_string()),
                ],
            )),
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
            relations: Vec::new(),
            aggregates: Vec::new(),
        });

        let error = PostgresCompiler::compile(&ast).unwrap_err();

        assert!(error
            .to_string()
            .contains("raw condition placeholder count mismatch: expected 1 bindings, got 2"));
    }

    #[test]
    fn compiles_full_text_condition() {
        let compiled = compile(QueryAst::select(
            SelectNode::from(TableRef::new("users"))
                .select(Expr::column(ColumnRef::new("users", "id")))
                .where_(Condition::full_text(
                    [
                        ColumnRef::new("users", "name"),
                        ColumnRef::new("users", "email"),
                    ],
                    "alice",
                )),
        ));

        assert_eq!(
            compiled.sql,
            "SELECT \"users\".\"id\" FROM \"users\" WHERE to_tsvector('english'::regconfig, COALESCE(\"users\".\"name\"::text, '') || ' ' || COALESCE(\"users\".\"email\"::text, '')) @@ plainto_tsquery('english'::regconfig, $1::text)"
        );
    }

    #[test]
    fn full_text_condition_with_no_columns_compiles_false() {
        let compiled = compile(QueryAst::select(
            SelectNode::from(TableRef::new("users"))
                .select(Expr::column(ColumnRef::new("users", "id")))
                .where_(Condition::full_text(Vec::<ColumnRef>::new(), "alice")),
        ));

        assert_eq!(
            compiled.sql,
            "SELECT \"users\".\"id\" FROM \"users\" WHERE FALSE"
        );
    }

    #[test]
    fn empty_boolean_condition_groups_compile_to_identity_values() {
        let and_compiled = compile(QueryAst::select(
            SelectNode::from(TableRef::new("users"))
                .select(Expr::column(ColumnRef::new("users", "id")))
                .where_(Condition::and(Vec::<Condition>::new())),
        ));
        let or_compiled = compile(QueryAst::select(
            SelectNode::from(TableRef::new("users"))
                .select(Expr::column(ColumnRef::new("users", "id")))
                .where_(Condition::or(Vec::<Condition>::new())),
        ));

        assert_eq!(
            and_compiled.sql,
            "SELECT \"users\".\"id\" FROM \"users\" WHERE TRUE"
        );
        assert_eq!(
            or_compiled.sql,
            "SELECT \"users\".\"id\" FROM \"users\" WHERE FALSE"
        );
    }

    #[test]
    fn compiles_insert_update_and_delete() {
        let insert = compile(QueryAst::insert(InsertNode {
            into: TableRef::new("users"),
            source: InsertSource::Values(vec![vec![
                (
                    ColumnRef::new("users", "email"),
                    Expr::value("foundry@example.com"),
                ),
                (ColumnRef::new("users", "active"), Expr::value(true)),
            ]]),
            on_conflict: None,
            returning: vec![SelectItem::new(Expr::column(ColumnRef::new("users", "id")))],
        }));
        assert_eq!(
            insert.sql,
            "INSERT INTO \"users\" (\"email\", \"active\") VALUES ($1::text, $2::boolean) RETURNING \"users\".\"id\""
        );

        let update = compile(QueryAst::update(UpdateNode {
            table: TableRef::new("users"),
            values: vec![(
                ColumnRef::new("users", "email"),
                Expr::value("updated@example.com"),
            )],
            from: Vec::new(),
            condition: Some(Condition::compare(
                Expr::column(ColumnRef::new("users", "id")),
                ComparisonOp::Eq,
                Expr::value(1_i64),
            )),
            returning: vec![],
        }));
        assert_eq!(
            update.sql,
            "UPDATE \"users\" SET \"email\" = $1::text WHERE \"users\".\"id\" = $2::bigint"
        );

        let delete = compile(QueryAst::delete(crate::database::ast::DeleteNode {
            from: TableRef::new("users"),
            using: Vec::new(),
            condition: Some(Condition::compare(
                Expr::column(ColumnRef::new("users", "id")),
                ComparisonOp::Eq,
                Expr::value(7_i64),
            )),
            returning: vec![],
        }));
        assert_eq!(
            delete.sql,
            "DELETE FROM \"users\" WHERE \"users\".\"id\" = $1::bigint"
        );
    }

    #[test]
    fn compiles_right_full_and_cross_joins() {
        let ast = QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(ColumnRef::new("users", "id")))],
            joins: vec![
                JoinNode {
                    kind: JoinKind::Right,
                    table: FromItem::Table(TableRef::new("profiles")),
                    lateral: false,
                    on: Some(Condition::compare(
                        Expr::column(ColumnRef::new("profiles", "user_id")),
                        ComparisonOp::Eq,
                        Expr::column(ColumnRef::new("users", "id")),
                    )),
                },
                JoinNode {
                    kind: JoinKind::Full,
                    table: FromItem::Table(TableRef::new("teams")),
                    lateral: false,
                    on: Some(Condition::compare(
                        Expr::column(ColumnRef::new("teams", "owner_id")),
                        ComparisonOp::Eq,
                        Expr::column(ColumnRef::new("users", "id")),
                    )),
                },
                JoinNode {
                    kind: JoinKind::Cross,
                    table: FromItem::Table(TableRef::new("regions")),
                    lateral: false,
                    on: None,
                },
            ],
            condition: None,
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
            relations: Vec::new(),
            aggregates: Vec::new(),
        });

        let compiled = compile(ast);
        assert_eq!(
            compiled.sql,
            "SELECT \"users\".\"id\" FROM \"users\" RIGHT JOIN \"profiles\" ON \"profiles\".\"user_id\" = \"users\".\"id\" FULL OUTER JOIN \"teams\" ON \"teams\".\"owner_id\" = \"users\".\"id\" CROSS JOIN \"regions\""
        );
    }

    #[test]
    fn compiles_multi_row_insert_and_on_conflict() {
        let compiled = compile(QueryAst::insert(InsertNode {
            into: TableRef::new("users"),
            source: InsertSource::Values(vec![
                vec![
                    (ColumnRef::new("users", "id"), Expr::value(1_i64)),
                    (
                        ColumnRef::new("users", "email"),
                        Expr::value("first@example.com"),
                    ),
                ],
                vec![
                    (
                        ColumnRef::new("users", "email"),
                        Expr::value("second@example.com"),
                    ),
                    (ColumnRef::new("users", "id"), Expr::value(2_i64)),
                ],
            ]),
            on_conflict: Some(OnConflictNode {
                target: Some(OnConflictTarget::Columns(vec![ColumnRef::new(
                    "users", "email",
                )])),
                action: OnConflictAction::DoUpdate(Box::new(OnConflictUpdate {
                    assignments: vec![(
                        ColumnRef::new("users", "email"),
                        Expr::excluded(ColumnRef::new("users", "email")),
                    )],
                    condition: Some(Condition::compare(
                        Expr::column(ColumnRef::new("users", "active")),
                        ComparisonOp::Eq,
                        Expr::value(true),
                    )),
                })),
            }),
            returning: vec![SelectItem::new(Expr::column(ColumnRef::new("users", "id")))],
        }));

        assert_eq!(
            compiled.sql,
            "INSERT INTO \"users\" (\"id\", \"email\") VALUES ($1::bigint, $2::text), ($3::bigint, $4::text) ON CONFLICT (\"email\") DO UPDATE SET \"email\" = EXCLUDED.\"email\" WHERE \"users\".\"active\" = $5::boolean RETURNING \"users\".\"id\""
        );
    }

    #[test]
    fn compiles_group_by_having_and_aggregate_projections() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("orders")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(ColumnRef::new(
                "orders",
                "merchant_id",
            )))],
            joins: Vec::new(),
            condition: None,
            group_by: vec![Expr::column(ColumnRef::new("orders", "merchant_id"))],
            having: Some(Condition::compare(
                Expr::Aggregate(AggregateExpr::sum(Expr::column(ColumnRef::new(
                    "orders", "total",
                )))),
                ComparisonOp::Gt,
                Expr::value(100_i64),
            )),
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
            relations: Vec::new(),
            aggregates: vec![AggregateNode::sum(
                Expr::column(ColumnRef::new("orders", "total").typed(DbType::Numeric)),
                "total_amount",
            )],
        }));

        assert_eq!(
            compiled.sql,
            "SELECT \"orders\".\"merchant_id\", (SUM(\"orders\".\"total\"))::text AS \"total_amount\" FROM \"orders\" GROUP BY \"orders\".\"merchant_id\" HAVING SUM(\"orders\".\"total\") > $1::bigint"
        );
    }

    #[test]
    fn compiles_case_json_cte_and_union() {
        let left = QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![
                SelectItem::new(CaseExpr {
                    whens: vec![CaseWhen {
                        condition: Condition::compare(
                            Expr::column(ColumnRef::new("users", "active")),
                            ComparisonOp::Eq,
                            Expr::value(true),
                        ),
                        result: Box::new(Expr::value("active")),
                    }],
                    else_expr: Some(Box::new(Expr::value("inactive"))),
                })
                .aliased("status_label"),
                SelectItem::new(JsonPathExpr {
                    expr: Box::new(Expr::column(
                        ColumnRef::new("users", "metadata").typed(DbType::Json),
                    )),
                    path: vec![JsonPathSegment::Key("profile".to_string())],
                    mode: JsonPathMode::Json,
                })
                .aliased("profile"),
            ],
            joins: Vec::new(),
            condition: Some(Condition::json(
                Expr::column(ColumnRef::new("users", "metadata").typed(DbType::Json)),
                JsonPredicateOp::HasKey,
                JsonPredicateValue::Key("profile".to_string()),
            )),
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
            relations: Vec::new(),
            aggregates: Vec::new(),
        });
        let right = QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("archived_users")),
            distinct: false,
            columns: vec![
                SelectItem::new(Expr::value("archived")).aliased("status_label"),
                SelectItem::new(Expr::column(
                    ColumnRef::new("archived_users", "profile").typed(DbType::Json),
                ))
                .aliased("profile"),
            ],
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
        });

        let compiled = compile(QueryAst {
            with: vec![CteNode {
                name: "recent_users".to_string(),
                query: Box::new(left.clone()),
                recursive: false,
                materialization: Some(CteMaterialization::Materialized),
            }],
            body: QueryBody::SetOperation(Box::new(SetOperationNode {
                left: Box::new(QueryAst::select(SelectNode {
                    from: FromItem::Table(TableRef::new("recent_users")),
                    distinct: false,
                    columns: vec![
                        SelectItem::new(Expr::column(ColumnRef::bare("status_label"))),
                        SelectItem::new(Expr::column(
                            ColumnRef::bare("profile").typed(DbType::Json),
                        )),
                    ],
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
                })),
                operator: SetOperator::UnionAll,
                right: Box::new(right),
                order_by: vec![OrderBy::asc("status_label")],
                limit: Some(10),
                offset: None,
            })),
        });

        assert_eq!(
            compiled.sql,
            "WITH \"recent_users\" AS MATERIALIZED (SELECT CASE WHEN \"users\".\"active\" = $1::boolean THEN $2::text ELSE $3::text END AS \"status_label\", (\"users\".\"metadata\") -> $4::text AS \"profile\" FROM \"users\" WHERE \"users\".\"metadata\" ? $5::text) (SELECT \"status_label\", \"profile\" FROM \"recent_users\") UNION ALL (SELECT $6::text AS \"status_label\", \"archived_users\".\"profile\" AS \"profile\" FROM \"archived_users\") ORDER BY \"status_label\" ASC LIMIT $7::bigint"
        );
    }

    #[test]
    fn compiles_numeric_and_distinct_aggregates() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("payments")),
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
            aggregates: vec![
                AggregateNode::count_distinct(
                    Expr::column(ColumnRef::new("payments", "merchant_id")),
                    "merchant_count",
                ),
                AggregateNode::avg(
                    Expr::column(ColumnRef::new("payments", "amount").typed(DbType::Numeric)),
                    "avg_amount",
                ),
                AggregateNode::min(
                    Expr::column(ColumnRef::new("payments", "amount").typed(DbType::Numeric)),
                    "min_amount",
                ),
                AggregateNode::max(
                    Expr::column(ColumnRef::new("payments", "amount").typed(DbType::Numeric)),
                    "max_amount",
                ),
            ],
        }));

        assert_eq!(
            compiled.sql,
            "SELECT COUNT(DISTINCT \"payments\".\"merchant_id\") AS \"merchant_count\", (AVG(\"payments\".\"amount\"))::text AS \"avg_amount\", (MIN(\"payments\".\"amount\"))::text AS \"min_amount\", (MAX(\"payments\".\"amount\"))::text AS \"max_amount\" FROM \"payments\""
        );
    }

    #[test]
    fn compiles_aliased_numeric_projection_casts_before_alias() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(
                ColumnRef::new("users", "credit_1")
                    .typed(DbType::Numeric)
                    .aliased("credit_1"),
            ))],
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
        }));

        assert_eq!(
            compiled.sql,
            "SELECT (\"users\".\"credit_1\")::text AS \"credit_1\" FROM \"users\""
        );
    }

    #[test]
    fn null_bindings_keep_their_type_information() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(ColumnRef::new("users", "id")))],
            joins: Vec::new(),
            condition: Some(Condition::compare(
                Expr::column(ColumnRef::new("users", "deleted_at")),
                ComparisonOp::Eq,
                Expr::value(DbValue::Null(DbType::Text)),
            )),
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
            relations: Vec::new(),
            aggregates: Vec::new(),
        }));

        assert_eq!(
            compiled.sql,
            "SELECT \"users\".\"id\" FROM \"users\" WHERE \"users\".\"deleted_at\" = $1::text"
        );
        assert_eq!(compiled.bindings, vec![DbValue::Null(DbType::Text)]);

        let numeric = Numeric::new("123.45").unwrap();
        assert_eq!(numeric.as_str(), "123.45");
    }

    #[test]
    fn compiles_case_insensitive_equality_operator() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(ColumnRef::new("users", "id")))],
            joins: Vec::new(),
            condition: Some(Condition::compare(
                Expr::column(ColumnRef::new("users", "email")),
                ComparisonOp::IEq,
                Expr::value("Test@Example.com"),
            )),
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: None,
            relations: Vec::new(),
            aggregates: Vec::new(),
        }));

        assert_eq!(
            compiled.sql,
            "SELECT \"users\".\"id\" FROM \"users\" WHERE LOWER(\"users\".\"email\") = LOWER($1::text)"
        );
        assert_eq!(
            compiled.bindings,
            vec![DbValue::Text("Test@Example.com".into())]
        );
    }

    #[test]
    fn compiles_typed_cast_expressions() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users")),
            distinct: false,
            columns: vec![
                SelectItem::new(Expr::cast_text(ColumnRef::new("users", "id"))).aliased("id"),
            ],
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
        }));

        assert_eq!(
            compiled.sql,
            r#"SELECT ("users"."id")::text AS "id" FROM "users""#
        );
        assert!(compiled.bindings.is_empty());
    }

    #[test]
    fn compiles_lock_clauses_and_lateral_joins() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("users").aliased("u")),
            distinct: false,
            columns: vec![SelectItem::new(Expr::column(ColumnRef::new("u", "id")))],
            joins: vec![JoinNode {
                kind: JoinKind::Left,
                table: FromItem::subquery(
                    QueryAst::select(SelectNode {
                        from: FromItem::Table(TableRef::new("orders").aliased("o")),
                        distinct: false,
                        columns: vec![SelectItem::new(Expr::column(ColumnRef::new(
                            "o", "user_id",
                        )))],
                        joins: Vec::new(),
                        condition: None,
                        group_by: Vec::new(),
                        having: None,
                        order_by: Vec::new(),
                        limit: Some(1),
                        offset: None,
                        lock: None,
                        relations: Vec::new(),
                        aggregates: Vec::new(),
                    }),
                    "recent_orders",
                ),
                lateral: true,
                on: Some(Condition::compare(
                    Expr::column(ColumnRef::new("recent_orders", "user_id")),
                    ComparisonOp::Eq,
                    Expr::column(ColumnRef::new("u", "id")),
                )),
            }],
            condition: None,
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
            lock: Some(LockClause {
                strength: LockStrength::Update,
                of: vec!["u".to_string()],
                behavior: LockBehavior::SkipLocked,
            }),
            relations: Vec::new(),
            aggregates: Vec::new(),
        }));

        assert_eq!(
            compiled.sql,
            "SELECT \"u\".\"id\" FROM \"users\" AS \"u\" LEFT JOIN LATERAL (SELECT \"o\".\"user_id\" FROM \"orders\" AS \"o\" LIMIT $1::bigint) AS \"recent_orders\" ON \"recent_orders\".\"user_id\" = \"u\".\"id\" FOR UPDATE OF \"u\" SKIP LOCKED"
        );
    }

    #[test]
    fn compiles_insert_select_update_from_and_delete_using() {
        let insert = compile(QueryAst::insert(InsertNode {
            into: TableRef::new("user_archive"),
            source: InsertSource::Select(Box::new(QueryAst::select(SelectNode {
                from: FromItem::Table(TableRef::new("users")),
                distinct: false,
                columns: vec![
                    SelectItem::new(Expr::column(ColumnRef::bare("id"))),
                    SelectItem::new(Expr::column(ColumnRef::bare("email"))),
                ],
                joins: Vec::new(),
                condition: Some(Condition::compare(
                    Expr::column(ColumnRef::bare("active")),
                    ComparisonOp::Eq,
                    Expr::value(false),
                )),
                group_by: Vec::new(),
                having: None,
                order_by: Vec::new(),
                limit: None,
                offset: None,
                lock: None,
                relations: Vec::new(),
                aggregates: Vec::new(),
            }))),
            on_conflict: None,
            returning: Vec::new(),
        }));
        assert_eq!(
            insert.sql,
            "INSERT INTO \"user_archive\" (\"id\", \"email\") SELECT \"id\", \"email\" FROM \"users\" WHERE \"active\" = $1::boolean"
        );

        let update = compile(QueryAst::update(UpdateNode {
            table: TableRef::new("merchants"),
            values: vec![(
                ColumnRef::new("merchants", "name"),
                Expr::column(ColumnRef::new("users", "email")),
            )],
            from: vec![FromItem::Table(TableRef::new("users"))],
            condition: Some(Condition::compare(
                Expr::column(ColumnRef::new("merchants", "user_id")),
                ComparisonOp::Eq,
                Expr::column(ColumnRef::new("users", "id")),
            )),
            returning: vec![],
        }));
        assert_eq!(
            update.sql,
            "UPDATE \"merchants\" SET \"name\" = \"users\".\"email\" FROM \"users\" WHERE \"merchants\".\"user_id\" = \"users\".\"id\""
        );

        let delete = compile(QueryAst::delete(DeleteNode {
            from: TableRef::new("merchants"),
            using: vec![FromItem::Table(TableRef::new("users"))],
            condition: Some(Condition::compare(
                Expr::column(ColumnRef::new("merchants", "user_id")),
                ComparisonOp::Eq,
                Expr::column(ColumnRef::new("users", "id")),
            )),
            returning: vec![],
        }));
        assert_eq!(
            delete.sql,
            "DELETE FROM \"merchants\" USING \"users\" WHERE \"merchants\".\"user_id\" = \"users\".\"id\""
        );
    }

    #[test]
    fn compiles_functions_subqueries_and_windows() {
        let compiled = compile(QueryAst::select(SelectNode {
            from: FromItem::Table(TableRef::new("payments")),
            distinct: false,
            columns: vec![
                SelectItem::new(Expr::function(
                    "COALESCE",
                    [
                        Expr::column(ColumnRef::new("payments", "nickname")),
                        Expr::value("guest"),
                    ],
                ))
                .aliased("display_name"),
                SelectItem::new(Expr::function(
                    "LOWER",
                    [Expr::column(ColumnRef::new("payments", "email"))],
                ))
                .aliased("email_lower"),
                SelectItem::new(Expr::function(
                    "DATE_TRUNC",
                    [
                        Expr::value("day"),
                        Expr::column(
                            ColumnRef::new("payments", "created_at").typed(DbType::TimestampTz),
                        ),
                    ],
                ))
                .aliased("created_day"),
                SelectItem::new(Expr::function(
                    "EXTRACT",
                    [
                        Expr::value("epoch"),
                        Expr::column(
                            ColumnRef::new("payments", "created_at").typed(DbType::TimestampTz),
                        ),
                    ],
                ))
                .aliased("created_epoch"),
                SelectItem::new(Expr::window(
                    Expr::function("ROW_NUMBER", std::iter::empty()),
                    WindowSpec {
                        partition_by: vec![Expr::column(ColumnRef::new("payments", "merchant_id"))],
                        order_by: vec![OrderBy::desc(Expr::column(ColumnRef::new(
                            "payments",
                            "created_at",
                        )))],
                        frame: Some(WindowFrame {
                            units: WindowFrameUnits::Rows,
                            start: WindowFrameBound::UnboundedPreceding,
                            end: Some(WindowFrameBound::CurrentRow),
                        }),
                    },
                ))
                .aliased("row_number"),
                SelectItem::new(Expr::subquery(QueryAst::select(SelectNode {
                    from: FromItem::Table(TableRef::new("refunds")),
                    distinct: false,
                    columns: vec![SelectItem::new(Expr::Aggregate(AggregateExpr::count_all()))],
                    joins: Vec::new(),
                    condition: Some(Condition::compare(
                        Expr::column(ColumnRef::new("refunds", "payment_id")),
                        ComparisonOp::Eq,
                        Expr::column(ColumnRef::new("payments", "id")),
                    )),
                    group_by: Vec::new(),
                    having: None,
                    order_by: Vec::new(),
                    limit: None,
                    offset: None,
                    lock: None,
                    relations: Vec::new(),
                    aggregates: Vec::new(),
                })))
                .aliased("refund_count"),
            ],
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
        }));

        assert_eq!(
            compiled.sql,
            "SELECT COALESCE(\"payments\".\"nickname\", $1::text) AS \"display_name\", LOWER(\"payments\".\"email\") AS \"email_lower\", DATE_TRUNC($2::text, \"payments\".\"created_at\") AS \"created_day\", EXTRACT(epoch FROM \"payments\".\"created_at\") AS \"created_epoch\", ROW_NUMBER() OVER (PARTITION BY \"payments\".\"merchant_id\" ORDER BY \"payments\".\"created_at\" DESC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS \"row_number\", (SELECT COUNT(*) FROM \"refunds\" WHERE \"refunds\".\"payment_id\" = \"payments\".\"id\") AS \"refund_count\" FROM \"payments\""
        );
    }

    #[test]
    fn select_node_builder_compiles_scalar_subqueries() {
        let compiled = compile(QueryAst::select(
            SelectNode::from(TableRef::new("tags")).select_as(
                Expr::subquery(
                    SelectNode::from(TableRef::new("contact_tags"))
                        .select(crate::database::query::Sql::count_all())
                        .where_(Condition::compare(
                            Expr::column(ColumnRef::new("contact_tags", "tag_id")),
                            ComparisonOp::Eq,
                            Expr::column(ColumnRef::new("tags", "id")),
                        )),
                ),
                "contact_count",
            ),
        ));

        assert_eq!(
            compiled.sql,
            "SELECT (SELECT COUNT(*) FROM \"contact_tags\" WHERE \"contact_tags\".\"tag_id\" = \"tags\".\"id\") AS \"contact_count\" FROM \"tags\""
        );
    }

    #[test]
    fn compiles_json_text_or_first_helper() {
        let compiled = compile(QueryAst::select(
            SelectNode::from(TableRef::new("voucher_claims")).select_as(
                crate::database::query::Sql::json_text_or_first(
                    Expr::column(ColumnRef::new("voucher_claims", "voucher_snapshot"))
                        .json()
                        .key("name")
                        .as_json(),
                    "en",
                ),
                "voucher_name",
            ),
        ));

        assert_eq!(
            compiled.sql,
            "SELECT COALESCE(((\"voucher_claims\".\"voucher_snapshot\") -> $1::text)->>$2::text, (SELECT value FROM jsonb_each_text((\"voucher_claims\".\"voucher_snapshot\") -> $1::text) LIMIT 1)) AS \"voucher_name\" FROM \"voucher_claims\""
        );
    }
}
