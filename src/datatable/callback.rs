use std::any::Any;
use std::future::Future;

use crate::foundation::{Error, Result};
use crate::logging::{catch_future_panic, catch_sync_panic, panic_payload_message};

use super::column::DatatableColumn;
use super::context::DatatableContext;
use super::datatable_trait::Datatable;
use super::filter_meta::DatatableFilterRow;
use super::mapping::DatatableMapping;
use super::relation_filter::DatatableRelationFilter;
use super::sort::DatatableSort;

pub(crate) fn catch_datatable_callback<T>(
    subject: impl Into<String>,
    callback: impl FnOnce() -> T,
) -> Result<T> {
    let subject = subject.into();
    catch_sync_panic(callback).map_err(|panic| datatable_callback_panic_error(subject, panic))
}

pub(crate) async fn catch_datatable_future<T, Fut>(
    subject: impl Into<String>,
    future: Fut,
) -> Result<T>
where
    Fut: Future<Output = Result<T>>,
{
    let subject = subject.into();
    match catch_future_panic(future).await {
        Ok(result) => result,
        Err(panic) => Err(datatable_callback_panic_error(subject, panic)),
    }
}

pub(crate) fn datatable_columns<D>() -> Result<Vec<DatatableColumn<D::Row>>>
where
    D: Datatable + ?Sized,
{
    catch_datatable_callback(format!("`{}` columns callback", D::ID), D::columns)
}

pub(crate) fn datatable_mappings<D>() -> Result<Vec<DatatableMapping<D::Row>>>
where
    D: Datatable + ?Sized,
{
    catch_datatable_callback(format!("`{}` mappings callback", D::ID), D::mappings)
}

pub(crate) fn datatable_relation_filters<D>(
) -> Result<Vec<DatatableRelationFilter<D::Row, D::Query>>>
where
    D: Datatable + ?Sized,
{
    catch_datatable_callback(
        format!("`{}` relation_filters callback", D::ID),
        D::relation_filters,
    )
}

pub(crate) fn datatable_default_sort<D>() -> Result<Vec<DatatableSort<D::Row>>>
where
    D: Datatable + ?Sized,
{
    catch_datatable_callback(
        format!("`{}` default_sort callback", D::ID),
        D::default_sort,
    )
}

pub(crate) async fn datatable_available_filters<D>(
    ctx: &DatatableContext<'_>,
) -> Result<Vec<DatatableFilterRow>>
where
    D: Datatable + ?Sized,
{
    let subject = format!("`{}` available_filters callback", D::ID);
    let future = catch_datatable_callback(subject.clone(), || D::available_filters(ctx))?;
    catch_datatable_future(subject, future).await
}

fn datatable_callback_panic_error(subject: String, panic: Box<dyn Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.datatable",
        callback = %subject,
        panic = %message,
        "datatable callback panicked"
    );
    Error::message(format!("datatable {subject} panicked: {message}"))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde::Serialize;

    use super::{datatable_available_filters, datatable_columns, datatable_mappings};
    use crate::config::ConfigRepository;
    use crate::database::ProjectionQuery;
    use crate::datatable::{
        Datatable, DatatableColumn, DatatableContext, DatatableMapping, DatatableRequest,
    };
    use crate::foundation::{AppContext, Container, Error, Result};
    use crate::validation::RuleRegistry;

    #[derive(Clone, Serialize, crate::Projection)]
    struct MetadataRow {
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

    fn expect_error<T>(result: Result<T>) -> Error {
        match result {
            Ok(_) => panic!("expected datatable metadata callback to fail"),
            Err(error) => error,
        }
    }

    struct ColumnsPanicDatatable;

    #[async_trait]
    impl Datatable for ColumnsPanicDatatable {
        type Row = MetadataRow;
        type Query = ProjectionQuery<MetadataRow>;

        const ID: &'static str = "metadata.columns";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            ProjectionQuery::table("metadata_rows")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            panic!("columns boom")
        }
    }

    struct MappingsPanicDatatable;

    #[async_trait]
    impl Datatable for MappingsPanicDatatable {
        type Row = MetadataRow;
        type Query = ProjectionQuery<MetadataRow>;

        const ID: &'static str = "metadata.mappings";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            ProjectionQuery::table("metadata_rows")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            Vec::new()
        }

        fn mappings() -> Vec<DatatableMapping<Self::Row>> {
            panic!("mappings boom")
        }
    }

    struct AvailableFiltersErrorDatatable;

    #[async_trait]
    impl Datatable for AvailableFiltersErrorDatatable {
        type Row = MetadataRow;
        type Query = ProjectionQuery<MetadataRow>;

        const ID: &'static str = "metadata.available_filters_error";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            ProjectionQuery::table("metadata_rows")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            Vec::new()
        }

        async fn available_filters(
            _ctx: &DatatableContext,
        ) -> Result<Vec<crate::datatable::DatatableFilterRow>> {
            Err(Error::message("available filters failed"))
        }
    }

    struct AvailableFiltersPanicDatatable;

    #[async_trait]
    impl Datatable for AvailableFiltersPanicDatatable {
        type Row = MetadataRow;
        type Query = ProjectionQuery<MetadataRow>;

        const ID: &'static str = "metadata.available_filters";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            ProjectionQuery::table("metadata_rows")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            Vec::new()
        }

        async fn available_filters(
            _ctx: &DatatableContext,
        ) -> Result<Vec<crate::datatable::DatatableFilterRow>> {
            panic!("available filters boom")
        }
    }

    #[test]
    fn columns_callback_panic_becomes_datatable_error() {
        let error = expect_error(datatable_columns::<ColumnsPanicDatatable>());

        assert!(
            error
                .to_string()
                .contains("datatable `metadata.columns` columns callback panicked: columns boom"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn mappings_callback_panic_becomes_datatable_error() {
        let error = expect_error(datatable_mappings::<MappingsPanicDatatable>());

        assert!(
            error.to_string().contains(
                "datatable `metadata.mappings` mappings callback panicked: mappings boom"
            ),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn available_filters_error_remains_unchanged() {
        let app = test_app();
        let request = test_request();
        let ctx = DatatableContext::new(&app, None, &request);

        let error =
            expect_error(datatable_available_filters::<AvailableFiltersErrorDatatable>(&ctx).await);

        assert_eq!(error.to_string(), "available filters failed");
    }

    #[tokio::test]
    async fn available_filters_future_panic_becomes_datatable_error() {
        let app = test_app();
        let request = test_request();
        let ctx = DatatableContext::new(&app, None, &request);

        let error =
            expect_error(datatable_available_filters::<AvailableFiltersPanicDatatable>(&ctx).await);

        assert!(
            error.to_string().contains(
                "datatable `metadata.available_filters` available_filters callback panicked: available filters boom"
            ),
            "unexpected error: {error}"
        );
    }
}
