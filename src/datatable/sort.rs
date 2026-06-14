use std::marker::PhantomData;

use crate::database::{Expr, OrderDirection};

use super::column::DatatableFieldRef;

/// A default sort declaration for a datatable column.
pub struct DatatableSort<Row> {
    pub field_name: String,
    pub direction: OrderDirection,
    pub(crate) expr: Expr,
    _marker: PhantomData<Row>,
}

impl<Row> DatatableSort<Row> {
    pub fn asc(field: impl Into<DatatableFieldRef<Row>>) -> Self
    where
        Row: 'static,
    {
        Self::from_field(field.into(), OrderDirection::Asc)
    }

    pub fn desc(field: impl Into<DatatableFieldRef<Row>>) -> Self
    where
        Row: 'static,
    {
        Self::from_field(field.into(), OrderDirection::Desc)
    }

    fn from_field(field: DatatableFieldRef<Row>, direction: OrderDirection) -> Self {
        let expr = field.sort_expr.unwrap_or_else(|| {
            panic!("datatable sort for `{}` requires a sort target", field.name)
        });
        Self {
            field_name: field.name,
            direction,
            expr,
            _marker: PhantomData,
        }
    }
}
