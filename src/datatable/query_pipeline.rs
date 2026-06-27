use crate::foundation::Result;

use super::callback::{
    catch_datatable_callback, catch_datatable_future, datatable_default_sort,
    datatable_relation_filters,
};
use super::column::DatatableColumn;
use super::context::DatatableContext;
use super::datatable_trait::Datatable;
use super::filter_engine::{
    apply_auto_filters_with_relation_filters, apply_default_sorts, apply_sorts,
};

/// Shared query-build pipeline used by both JSON and download modes.
///
/// Steps: scoped base query -> auto-filters -> custom filter hook -> sorting.
pub async fn prepare_query<D>(
    ctx: &DatatableContext<'_>,
    columns: &[DatatableColumn<D::Row>],
) -> Result<D::Query>
where
    D: Datatable + ?Sized,
{
    let query = catch_datatable_callback(format!("`{}` query callback", D::ID), || D::query(ctx))?;
    let relation_filters = datatable_relation_filters::<D>()?;
    let query = apply_auto_filters_with_relation_filters(
        query,
        &ctx.request.filters,
        columns,
        &relation_filters,
    )?;
    let query = catch_datatable_future(
        format!("`{}` filters callback", D::ID),
        catch_datatable_callback(format!("`{}` filters callback", D::ID), || {
            D::filters(ctx, query)
        })?,
    )
    .await?;

    if ctx.request.sort.is_empty() {
        let default_sort = datatable_default_sort::<D>()?;
        apply_default_sorts(query, &default_sort)
    } else {
        apply_sorts(query, &ctx.request.sort, columns)
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde::Serialize;

    use super::prepare_query;
    use crate::config::ConfigRepository;
    use crate::database::ProjectionQuery;
    use crate::datatable::{
        Datatable, DatatableColumn, DatatableContext, DatatableFilterInput, DatatableRequest,
        DatatableSort,
    };
    use crate::foundation::{AppContext, Container, Error};
    use crate::validation::RuleRegistry;

    #[derive(Clone, Serialize, crate::Projection)]
    struct PipelineRow {
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

    fn test_request(filters: Vec<DatatableFilterInput>) -> DatatableRequest {
        DatatableRequest {
            page: 1,
            per_page: 20,
            sort: Vec::new(),
            filters,
            search: None,
        }
    }

    fn test_context<'a>(
        app: &'a AppContext,
        request: &'a DatatableRequest,
    ) -> DatatableContext<'a> {
        DatatableContext::new(app, None, request)
    }

    fn expect_error<T>(result: crate::Result<T>) -> Error {
        match result {
            Ok(_) => panic!("expected datatable callback to fail"),
            Err(error) => error,
        }
    }

    struct QueryPanicDatatable;

    #[async_trait]
    impl Datatable for QueryPanicDatatable {
        type Row = PipelineRow;
        type Query = ProjectionQuery<PipelineRow>;

        const ID: &'static str = "panic.query";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            panic!("query boom")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            Vec::new()
        }
    }

    struct FiltersPanicDatatable;

    #[async_trait]
    impl Datatable for FiltersPanicDatatable {
        type Row = PipelineRow;
        type Query = ProjectionQuery<PipelineRow>;

        const ID: &'static str = "panic.filters";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            ProjectionQuery::table("pipeline_rows")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            Vec::new()
        }

        async fn filters(
            _ctx: &DatatableContext,
            _query: Self::Query,
        ) -> crate::Result<Self::Query> {
            panic!("filters boom")
        }
    }

    struct RelationFiltersPanicDatatable;

    #[async_trait]
    impl Datatable for RelationFiltersPanicDatatable {
        type Row = PipelineRow;
        type Query = ProjectionQuery<PipelineRow>;

        const ID: &'static str = "panic.relation_filters";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            ProjectionQuery::table("pipeline_rows")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            Vec::new()
        }

        fn relation_filters(
        ) -> Vec<crate::datatable::DatatableRelationFilter<Self::Row, Self::Query>> {
            panic!("relation filters boom")
        }
    }

    struct DefaultSortPanicDatatable;

    #[async_trait]
    impl Datatable for DefaultSortPanicDatatable {
        type Row = PipelineRow;
        type Query = ProjectionQuery<PipelineRow>;

        const ID: &'static str = "panic.default_sort";

        fn query(_ctx: &DatatableContext) -> Self::Query {
            ProjectionQuery::table("pipeline_rows")
        }

        fn columns() -> Vec<DatatableColumn<Self::Row>> {
            Vec::new()
        }

        fn default_sort() -> Vec<DatatableSort<Self::Row>> {
            panic!("default sort boom")
        }
    }

    #[tokio::test]
    async fn query_callback_panic_becomes_datatable_error() {
        let app = test_app();
        let request = test_request(Vec::new());
        let ctx = test_context(&app, &request);

        let error = expect_error(prepare_query::<QueryPanicDatatable>(&ctx, &[]).await);

        assert!(
            error
                .to_string()
                .contains("datatable `panic.query` query callback panicked: query boom"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn filters_callback_panic_becomes_datatable_error() {
        let app = test_app();
        let request = test_request(Vec::new());
        let ctx = test_context(&app, &request);

        let error = expect_error(prepare_query::<FiltersPanicDatatable>(&ctx, &[]).await);

        assert!(
            error
                .to_string()
                .contains("datatable `panic.filters` filters callback panicked: filters boom"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn relation_filters_callback_panic_becomes_datatable_error() {
        let app = test_app();
        let request = test_request(Vec::new());
        let ctx = test_context(&app, &request);

        let error = expect_error(prepare_query::<RelationFiltersPanicDatatable>(&ctx, &[]).await);

        assert!(
            error.to_string().contains(
                "datatable `panic.relation_filters` relation_filters callback panicked: relation filters boom"
            ),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn default_sort_callback_panic_becomes_datatable_error() {
        let app = test_app();
        let request = test_request(Vec::new());
        let ctx = test_context(&app, &request);

        let error = expect_error(prepare_query::<DefaultSortPanicDatatable>(&ctx, &[]).await);

        assert!(
            error.to_string().contains(
                "datatable `panic.default_sort` default_sort callback panicked: default sort boom"
            ),
            "unexpected error: {error}"
        );
    }
}
