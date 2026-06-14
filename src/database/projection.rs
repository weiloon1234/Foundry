use std::{
    any::{type_name, Any},
    marker::PhantomData,
};

use crate::foundation::{Error, Result};
use crate::logging::{catch_sync_panic, panic_payload_message};

use super::ast::{ColumnRef, DbType, Expr, FromItem, SelectItem};
use super::model::FromDbValue;
use super::runtime::DbRecord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionFieldInfo {
    pub alias: &'static str,
    pub source_column: Option<&'static str>,
    pub db_type: DbType,
}

impl ProjectionFieldInfo {
    pub const fn new(alias: &'static str, db_type: DbType) -> Self {
        Self {
            alias,
            source_column: Some(alias),
            db_type,
        }
    }

    pub const fn from_source(
        alias: &'static str,
        source_column: &'static str,
        db_type: DbType,
    ) -> Self {
        Self {
            alias,
            source_column: Some(source_column),
            db_type,
        }
    }

    pub fn select_from(self, table_alias: &str) -> Result<SelectItem> {
        let source_column = self.source_column.unwrap_or(self.alias);
        Ok(SelectItem::new(
            ColumnRef::new(table_alias, source_column)
                .typed(self.db_type)
                .aliased(self.alias),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionField<P, T> {
    info: ProjectionFieldInfo,
    _marker: PhantomData<fn() -> (P, T)>,
}

impl<P, T> ProjectionField<P, T> {
    pub const fn new(alias: &'static str, db_type: DbType) -> Self {
        Self {
            info: ProjectionFieldInfo::new(alias, db_type),
            _marker: PhantomData,
        }
    }

    pub const fn from_source(
        alias: &'static str,
        source_column: &'static str,
        db_type: DbType,
    ) -> Self {
        Self {
            info: ProjectionFieldInfo::from_source(alias, source_column, db_type),
            _marker: PhantomData,
        }
    }

    pub const fn info(&self) -> ProjectionFieldInfo {
        self.info
    }

    pub const fn alias(&self) -> &'static str {
        self.info.alias
    }

    pub const fn db_type(&self) -> DbType {
        self.info.db_type
    }

    pub fn column_ref(&self) -> ColumnRef {
        ColumnRef::bare(self.alias()).typed(self.db_type())
    }

    pub fn column_ref_from(&self, table_alias: &str) -> ColumnRef {
        let source_column = self.info.source_column.unwrap_or(self.alias());
        ColumnRef::new(table_alias, source_column).typed(self.db_type())
    }

    pub fn decode(&self, record: &DbRecord) -> Result<T>
    where
        T: FromDbValue,
    {
        record.decode(self.alias())
    }

    pub fn select(&self, expr: impl Into<Expr>) -> SelectItem {
        SelectItem::new(expr).aliased(self.alias())
    }

    pub fn select_from(&self, table_alias: &str) -> Result<SelectItem> {
        self.info.select_from(table_alias)
    }
}

impl<P, T> From<ProjectionField<P, T>> for ColumnRef {
    fn from(value: ProjectionField<P, T>) -> Self {
        value.column_ref()
    }
}

impl<P, T> From<&ProjectionField<P, T>> for ColumnRef {
    fn from(value: &ProjectionField<P, T>) -> Self {
        value.column_ref()
    }
}

#[derive(Clone)]
pub struct ProjectionMeta<P> {
    fields: &'static [ProjectionFieldInfo],
    hydrate: fn(&DbRecord) -> Result<P>,
}

impl<P> ProjectionMeta<P> {
    pub const fn new(
        fields: &'static [ProjectionFieldInfo],
        hydrate: fn(&DbRecord) -> Result<P>,
    ) -> Self {
        Self { fields, hydrate }
    }

    pub const fn fields(&self) -> &'static [ProjectionFieldInfo] {
        self.fields
    }

    pub fn hydrate_record(&self, record: &DbRecord) -> Result<P> {
        match catch_sync_panic(|| (self.hydrate)(record)) {
            Ok(result) => result,
            Err(panic) => Err(projection_hydration_panic_error::<P>(panic)),
        }
    }

    pub fn source_select_items(&self, table_alias: &str) -> Result<Vec<SelectItem>> {
        self.fields
            .iter()
            .copied()
            .map(|field| field.select_from(table_alias))
            .collect()
    }
}

fn projection_hydration_panic_error<P>(panic: Box<dyn Any + Send>) -> Error {
    let projection = type_name::<P>();
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        projection = projection,
        panic = %message,
        "projection hydration panicked"
    );
    Error::message(format!(
        "projection `{projection}` hydration panicked: {message}"
    ))
}

pub trait Projection: Clone + Send + Sync + Sized + 'static {
    fn projection_meta() -> &'static ProjectionMeta<Self>;

    fn from_record(record: &DbRecord) -> Result<Self> {
        Self::projection_meta().hydrate_record(record)
    }

    fn source(source: impl Into<FromItem>) -> super::query::ProjectionQuery<Self> {
        super::query::ProjectionQuery::new(source, Self::projection_meta())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DbValue;
    use crate::foundation::Error;

    #[derive(Clone, Debug, PartialEq)]
    struct PanickingHydrationText(String);

    impl FromDbValue for PanickingHydrationText {
        fn from_db_value(value: &DbValue) -> Result<Self> {
            match value {
                DbValue::Text(value) if value == "panic" => panic!("projection hydrate boom"),
                DbValue::Text(value) => Ok(Self(value.clone())),
                _ => Err(Error::message("expected text value")),
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, crate::Projection)]
    struct PanicHydrationProjection {
        #[foundry(db_type = "text")]
        value: PanickingHydrationText,
    }

    #[test]
    fn projection_meta_hydration_panic_becomes_error() {
        let mut record = DbRecord::new();
        record.insert("value", DbValue::from("panic"));

        let error = PanicHydrationProjection::from_record(&record).unwrap_err();

        assert!(error.to_string().contains("projection `"));
        assert!(error
            .to_string()
            .contains("hydration panicked: projection hydrate boom"));
    }
}
