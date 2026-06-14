use serde_json::Value;

use crate::support::{Date, DateTime};

/// Output value type for datatable row mappings.
///
/// Converts to `serde_json::Value` for JSON responses.
/// Dates and datetimes serialize to ISO 8601 strings.
#[derive(Clone, Debug, PartialEq)]
pub enum DatatableValue {
    Null,
    String(String),
    Number(serde_json::Number),
    Bool(bool),
    Date(Date),
    DateTime(DateTime),
}

impl DatatableValue {
    pub fn null() -> Self {
        Self::Null
    }

    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    pub fn number(value: impl Into<serde_json::Number>) -> Self {
        Self::Number(value.into())
    }

    pub fn bool(value: bool) -> Self {
        Self::Bool(value)
    }

    pub fn date(value: Date) -> Self {
        Self::Date(value)
    }

    pub fn datetime(value: DateTime) -> Self {
        Self::DateTime(value)
    }
}

impl From<DatatableValue> for Value {
    fn from(value: DatatableValue) -> Self {
        match value {
            DatatableValue::Null => Value::Null,
            DatatableValue::String(s) => Value::String(s),
            DatatableValue::Number(n) => Value::Number(n),
            DatatableValue::Bool(b) => Value::Bool(b),
            DatatableValue::Date(d) => Value::String(d.to_string()),
            DatatableValue::DateTime(dt) => Value::String(dt.to_string()),
        }
    }
}
