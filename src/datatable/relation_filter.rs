use std::marker::PhantomData;
use std::sync::Arc;

use crate::database::{
    Column, ComparisonOp, Condition, DbType, DbValue, Expr, ManyToManyDef, Model, ModelQuery,
    RelationDef,
};
use crate::foundation::{Error, Result};

use super::callback::catch_datatable_callback;
use super::filter_engine::build_filter_condition;
use super::request::{DatatableFilterInput, DatatableFilterOp, DatatableFilterValue};

#[derive(Clone)]
pub struct DatatableRelationColumn<Row> {
    expr: Expr,
    db_type: DbType,
    _marker: PhantomData<Row>,
}

impl<Row> DatatableRelationColumn<Row> {
    pub fn field<T>(column: Column<Row, T>) -> Self {
        Self {
            expr: Expr::column(column.column_ref()),
            db_type: column.db_type(),
            _marker: PhantomData,
        }
    }
}

type RelationFilterApplier<Query> =
    Arc<dyn Fn(Query, &DatatableFilterInput) -> Result<Query> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct DatatableRelationFilter<Row, Query> {
    pub field: String,
    pub relation: String,
    aliases: Vec<String>,
    apply: RelationFilterApplier<Query>,
    _marker: PhantomData<fn() -> Row>,
}

impl<Row, Query> DatatableRelationFilter<Row, Query>
where
    Query: 'static,
{
    fn new(
        field: impl Into<String>,
        relation: impl Into<String>,
        apply: impl Fn(Query, &DatatableFilterInput) -> Result<Query> + Send + Sync + 'static,
    ) -> Self {
        let field = field.into();
        let aliases = default_aliases(&field);
        Self {
            field,
            relation: relation.into(),
            aliases,
            apply: Arc::new(apply),
            _marker: PhantomData,
        }
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        let alias = alias.into();
        if alias != self.field && !self.aliases.iter().any(|existing| existing == &alias) {
            self.aliases.push(alias);
        }
        self
    }

    pub(crate) fn matches(&self, field: &str) -> bool {
        self.field == field || self.aliases.iter().any(|alias| alias == field)
    }

    pub(crate) fn apply(&self, query: Query, filter: &DatatableFilterInput) -> Result<Query> {
        catch_datatable_callback(
            format!(
                "relation filter `{}` for relation `{}`",
                self.field, self.relation
            ),
            || (self.apply)(query, filter),
        )?
    }
}

impl<From> DatatableRelationFilter<From, ModelQuery<From>>
where
    From: Model,
{
    pub fn relation<To, T>(
        field: impl Into<String>,
        relation: RelationDef<From, To>,
        column: Column<To, T>,
    ) -> Self
    where
        To: Model,
        T: 'static,
    {
        Self::relation_columns(field, relation, [DatatableRelationColumn::field(column)])
    }

    pub fn relation_columns<To>(
        field: impl Into<String>,
        relation: RelationDef<From, To>,
        columns: impl IntoIterator<Item = DatatableRelationColumn<To>>,
    ) -> Self
    where
        To: Model,
    {
        let relation_name = relation.node().name;
        let columns = collect_relation_columns(columns);
        Self::new(field, relation_name, move |query, filter| {
            let condition = build_relation_condition(filter, &columns)?;
            Ok(query.where_has(relation.clone(), |child| child.where_(condition)))
        })
    }

    pub fn many_to_many<To, Pivot, T>(
        field: impl Into<String>,
        relation: ManyToManyDef<From, To, Pivot>,
        column: Column<To, T>,
    ) -> Self
    where
        To: Model,
        Pivot: Clone + Send + Sync + 'static,
        T: 'static,
    {
        Self::many_to_many_columns(field, relation, [DatatableRelationColumn::field(column)])
    }

    pub fn many_to_many_columns<To, Pivot>(
        field: impl Into<String>,
        relation: ManyToManyDef<From, To, Pivot>,
        columns: impl IntoIterator<Item = DatatableRelationColumn<To>>,
    ) -> Self
    where
        To: Model,
        Pivot: Clone + Send + Sync + 'static,
    {
        let relation_name = relation.node().name;
        let columns = collect_relation_columns(columns);
        Self::new(field, relation_name, move |query, filter| {
            let condition = build_relation_condition(filter, &columns)?;
            Ok(query.where_has_many_to_many(relation.clone(), |child| child.where_(condition)))
        })
    }
}

fn default_aliases(field: &str) -> Vec<String> {
    let legacy = field.replace('.', "-");
    if legacy == field {
        Vec::new()
    } else {
        vec![legacy]
    }
}

fn collect_relation_columns<Row>(
    columns: impl IntoIterator<Item = DatatableRelationColumn<Row>>,
) -> Vec<DatatableRelationColumn<Row>> {
    columns.into_iter().collect()
}

fn build_relation_condition<Row>(
    filter: &DatatableFilterInput,
    columns: &[DatatableRelationColumn<Row>],
) -> Result<Condition> {
    if columns.is_empty() {
        return Err(Error::message(format!(
            "relation filter '{}' has no target columns",
            filter.field
        )));
    }

    if filter.op == DatatableFilterOp::LikeAny {
        return build_relation_like_any_condition(filter, columns);
    }

    if columns.len() != 1 {
        return Err(Error::message(format!(
            "relation filter '{}' requires LikeAny when multiple target columns are declared",
            filter.field
        )));
    }

    let column = &columns[0];
    build_filter_condition(
        &filter.op,
        column.expr.clone(),
        &filter.value,
        column.db_type,
    )
}

fn build_relation_like_any_condition<Row>(
    filter: &DatatableFilterInput,
    columns: &[DatatableRelationColumn<Row>],
) -> Result<Condition> {
    let text = match &filter.value {
        DatatableFilterValue::Text(value) => value,
        _ => return Err(Error::message("LikeAny requires a text value")),
    };
    let pattern = format!("%{text}%");
    let conditions = columns
        .iter()
        .map(|column| {
            Condition::compare(
                column.expr.clone(),
                ComparisonOp::ILike,
                Expr::value(DbValue::Text(pattern.clone())),
            )
        })
        .collect::<Vec<_>>();
    Ok(Condition::or(conditions))
}

#[cfg(test)]
mod tests {
    use super::DatatableRelationFilter;
    use crate::datatable::{DatatableFilterInput, DatatableFilterOp, DatatableFilterValue};
    use crate::foundation::Error;

    fn text_filter(field: &str) -> DatatableFilterInput {
        DatatableFilterInput {
            field: field.to_string(),
            op: DatatableFilterOp::Like,
            value: DatatableFilterValue::Text("foundry".to_string()),
        }
    }

    #[test]
    fn relation_filter_apply_panic_becomes_datatable_error() {
        let filter = DatatableRelationFilter::<(), i64>::new(
            "merchant.name",
            "merchant",
            |_query, _filter| -> crate::Result<i64> { panic!("relation filter boom") },
        );

        let error = filter.apply(1, &text_filter("merchant.name")).unwrap_err();

        assert!(
            error.to_string().contains(
                "datatable relation filter `merchant.name` for relation `merchant` panicked: relation filter boom"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn relation_filter_apply_error_remains_unchanged() {
        let filter = DatatableRelationFilter::<(), i64>::new(
            "merchant.name",
            "merchant",
            |_query, _filter| Err(Error::message("bad relation filter")),
        );

        let error = filter.apply(1, &text_filter("merchant.name")).unwrap_err();

        assert_eq!(error.to_string(), "bad relation filter");
    }
}
