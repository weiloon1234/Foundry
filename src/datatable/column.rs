use std::marker::PhantomData;

use crate::database::{Column, DbType, Expr, ProjectionField};

#[derive(Clone)]
pub(crate) struct DatatableFilterTarget {
    pub scope: DatatableFilterScope,
    pub expr: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DatatableFilterScope {
    Where,
    Having,
}

pub struct DatatableFieldRef<Row> {
    pub(crate) name: String,
    pub(crate) db_type: DbType,
    pub(crate) sort_expr: Option<Expr>,
    pub(crate) filter_target: Option<DatatableFilterTarget>,
    _marker: PhantomData<Row>,
}

impl<Row, T> From<Column<Row, T>> for DatatableFieldRef<Row>
where
    Row: 'static,
{
    fn from(column: Column<Row, T>) -> Self {
        let column_ref = column.column_ref();
        Self {
            name: column.name().to_string(),
            db_type: column.db_type(),
            sort_expr: Some(Expr::column(column_ref.clone())),
            filter_target: Some(DatatableFilterTarget {
                scope: DatatableFilterScope::Where,
                expr: Expr::column(column_ref),
            }),
            _marker: PhantomData,
        }
    }
}

impl<Row, T> From<ProjectionField<Row, T>> for DatatableFieldRef<Row> {
    fn from(field: ProjectionField<Row, T>) -> Self {
        Self {
            name: field.alias().to_string(),
            db_type: field.db_type(),
            sort_expr: Some(Expr::column(field.column_ref())),
            filter_target: None,
            _marker: PhantomData,
        }
    }
}

/// A datatable column descriptor.
///
/// Stores column metadata for rendering, filtering, sorting, and export.
pub struct DatatableColumn<Row> {
    pub name: String,
    pub label: String,
    pub sortable: bool,
    pub filterable: bool,
    pub exportable: bool,
    pub relation: Option<String>,
    db_type: DbType,
    sort_expr: Option<Expr>,
    filter_target: Option<DatatableFilterTarget>,
    _marker: PhantomData<Row>,
}

impl<Row> DatatableColumn<Row>
where
    Row: 'static,
{
    pub fn field(field: impl Into<DatatableFieldRef<Row>>) -> Self {
        let field = field.into();
        Self {
            name: field.name.clone(),
            label: field.name,
            sortable: false,
            filterable: false,
            exportable: false,
            relation: None,
            db_type: field.db_type,
            sort_expr: field.sort_expr,
            filter_target: field.filter_target,
            _marker: PhantomData,
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn sortable(mut self) -> Self {
        assert!(
            self.sort_expr.is_some(),
            "datatable column `{}` has no sort target; use sort_by(...) to define one",
            self.name
        );
        self.sortable = true;
        self
    }

    pub fn sort_by(mut self, expr: impl Into<Expr>) -> Self {
        self.sort_expr = Some(expr.into());
        self.sortable = true;
        self
    }

    pub fn filterable(mut self) -> Self {
        assert!(
            self.filter_target.is_some(),
            "datatable column `{}` has no filter target; use filter_by(...) or filter_having(...)",
            self.name
        );
        self.filterable = true;
        self
    }

    pub fn filter_by(mut self, expr: impl Into<Expr>) -> Self {
        self.filter_target = Some(DatatableFilterTarget {
            scope: DatatableFilterScope::Where,
            expr: expr.into(),
        });
        self.filterable = true;
        self
    }

    pub fn filter_having(mut self, expr: impl Into<Expr>) -> Self {
        self.filter_target = Some(DatatableFilterTarget {
            scope: DatatableFilterScope::Having,
            expr: expr.into(),
        });
        self.filterable = true;
        self
    }

    pub fn exportable(mut self) -> Self {
        self.exportable = true;
        self
    }

    pub fn relation(mut self, relation: impl Into<String>) -> Self {
        self.relation = Some(relation.into());
        self
    }

    pub fn db_type(&self) -> DbType {
        self.db_type
    }

    pub(crate) fn sort_expr(&self) -> Option<&Expr> {
        self.sort_expr.as_ref()
    }

    pub(crate) fn filter_target(&self) -> Option<&DatatableFilterTarget> {
        self.filter_target.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::{DatatableColumn, DatatableFilterScope};
    use crate::database::{ColumnRef, DbType, ProjectionField};

    #[derive(Debug, serde::Serialize, crate::Model)]
    #[foundry(table = "datatable_column_models", primary_key_strategy = "manual")]
    struct ModelRow {
        id: i64,
    }

    #[derive(Clone, serde::Serialize)]
    struct RowProjection {
        total: i64,
    }

    impl RowProjection {
        const TOTAL: ProjectionField<Self, i64> = ProjectionField::new("total", DbType::Int64);
    }

    #[test]
    fn projection_fields_sort_by_alias_by_default() {
        let column = DatatableColumn::field(RowProjection::TOTAL).sortable();

        assert!(column.sortable);
        assert!(!column.filterable);
        assert_eq!(
            column.sort_expr().cloned(),
            Some(crate::database::Expr::column(
                ColumnRef::bare("total").typed(DbType::Int64)
            ))
        );
    }

    #[test]
    fn model_fields_keep_where_filter_targets() {
        let column = DatatableColumn::field(ModelRow::ID).sortable().filterable();

        let target = column.filter_target().expect("filter target should exist");
        assert!(column.sortable);
        assert!(column.filterable);
        assert_eq!(target.scope, DatatableFilterScope::Where);
    }

    #[test]
    fn filter_having_marks_the_column_as_filterable() {
        let column = DatatableColumn::field(RowProjection::TOTAL)
            .filter_having(crate::database::Expr::column(ColumnRef::bare("total")));

        let target = column.filter_target().expect("filter target should exist");
        assert!(column.filterable);
        assert_eq!(target.scope, DatatableFilterScope::Having);
    }
}
