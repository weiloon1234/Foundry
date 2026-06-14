use std::marker::PhantomData;

use crate::foundation::Result;

use super::callback::catch_datatable_callback;
use super::context::DatatableContext;
use super::value::DatatableValue;

type MappingCallback<M> = Box<dyn Fn(&M, &DatatableContext) -> DatatableValue + Send + Sync>;

/// A computed output-only field for datatable rows.
///
/// Mappings can override existing column values or add new computed fields.
/// They are not automatically sortable or filterable.
pub struct DatatableMapping<M> {
    pub name: String,
    callback: MappingCallback<M>,
    _marker: PhantomData<M>,
}

impl<M> DatatableMapping<M> {
    pub fn new<F>(name: impl Into<String>, callback: F) -> Self
    where
        F: Fn(&M, &DatatableContext) -> DatatableValue + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            callback: Box::new(callback),
            _marker: PhantomData,
        }
    }

    pub fn compute(&self, model: &M, ctx: &DatatableContext) -> DatatableValue {
        (self.callback)(model, ctx)
    }

    pub(crate) fn try_compute(&self, model: &M, ctx: &DatatableContext) -> Result<DatatableValue> {
        catch_datatable_callback(format!("mapping `{}`", self.name), || {
            (self.callback)(model, ctx)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DatatableMapping;
    use crate::config::ConfigRepository;
    use crate::datatable::{DatatableContext, DatatableRequest, DatatableValue};
    use crate::foundation::{AppContext, Container};
    use crate::validation::RuleRegistry;

    #[derive(serde::Serialize)]
    struct MappingRow {
        id: i64,
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn test_request() -> DatatableRequest {
        DatatableRequest {
            page: 1,
            per_page: 20,
            sort: Vec::new(),
            filters: Vec::new(),
            search: None,
        }
    }

    #[test]
    fn mapping_panic_becomes_datatable_error() {
        let app = test_app();
        let request = test_request();
        let ctx = DatatableContext::new(&app, None, &request);
        let row = MappingRow { id: 1 };
        let mapping =
            DatatableMapping::new("status", |_row: &MappingRow, _ctx| panic!("mapping boom"));

        let error = mapping.try_compute(&row, &ctx).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("datatable mapping `status` panicked: mapping boom"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn mapping_success_still_returns_value() {
        let app = test_app();
        let request = test_request();
        let ctx = DatatableContext::new(&app, None, &request);
        let row = MappingRow { id: 7 };
        let mapping = DatatableMapping::new("id_text", |row: &MappingRow, _ctx| {
            DatatableValue::String(row.id.to_string())
        });

        let value = mapping.try_compute(&row, &ctx).unwrap();

        assert_eq!(value, DatatableValue::String("7".to_string()));
    }
}
