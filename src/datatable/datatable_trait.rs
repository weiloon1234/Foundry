use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::Serialize;

use crate::auth::Actor;
use crate::database::{
    Condition, Model, ModelQuery, OrderBy, Paginated, Pagination, ProjectionQuery, QueryExecutor,
};
use crate::foundation::{AppContext, Result};
use crate::support::Collection;

use super::column::DatatableColumn;
use super::context::DatatableContext;
use super::filter_meta::DatatableFilterRow;
use super::mapping::DatatableMapping;
use super::relation_filter::DatatableRelationFilter;
use super::request::DatatableRequest;
use super::response::{DatatableExportAccepted, DatatableJsonResponse};
use super::sort::DatatableSort;

mod private {
    pub trait Sealed {}
}

#[async_trait]
pub trait DatatableQuery<Row>: private::Sealed + Clone + Send + Sync + 'static {
    fn apply_where(self, condition: Condition) -> Self;

    fn apply_having(self, condition: Condition) -> Self;

    fn apply_order(self, order: OrderBy) -> Self;

    fn apply_limit(self, limit: u64) -> Self;

    fn stream<'a, E>(&'a self, executor: &'a E) -> Result<BoxStream<'a, Result<Row>>>
    where
        E: QueryExecutor;

    async fn get<E>(&self, executor: &E) -> Result<Collection<Row>>
    where
        E: QueryExecutor;

    async fn paginate<E>(&self, executor: &E, pagination: Pagination) -> Result<Paginated<Row>>
    where
        E: QueryExecutor;
}

impl<M> private::Sealed for ModelQuery<M> where M: Model + Serialize + Send + Sync + 'static {}

#[async_trait]
impl<M> DatatableQuery<M> for ModelQuery<M>
where
    M: Model + Serialize + Send + Sync + 'static,
{
    fn apply_where(self, condition: Condition) -> Self {
        self.where_(condition)
    }

    fn apply_having(self, condition: Condition) -> Self {
        self.having(condition)
    }

    fn apply_order(self, order: OrderBy) -> Self {
        self.order_by(order)
    }

    fn apply_limit(self, limit: u64) -> Self {
        self.limit(limit)
    }

    fn stream<'a, E>(&'a self, executor: &'a E) -> Result<BoxStream<'a, Result<M>>>
    where
        E: QueryExecutor,
    {
        ModelQuery::stream(self, executor)
    }

    async fn get<E>(&self, executor: &E) -> Result<Collection<M>>
    where
        E: QueryExecutor,
    {
        ModelQuery::get(self, executor).await
    }

    async fn paginate<E>(&self, executor: &E, pagination: Pagination) -> Result<Paginated<M>>
    where
        E: QueryExecutor,
    {
        ModelQuery::paginate(self, executor, pagination).await
    }
}

impl<P> private::Sealed for ProjectionQuery<P> where P: Clone + Serialize + Send + Sync + 'static {}

#[async_trait]
impl<P> DatatableQuery<P> for ProjectionQuery<P>
where
    P: Clone + Serialize + Send + Sync + 'static,
{
    fn apply_where(self, condition: Condition) -> Self {
        self.where_(condition)
    }

    fn apply_having(self, condition: Condition) -> Self {
        self.having(condition)
    }

    fn apply_order(self, order: OrderBy) -> Self {
        self.order_by(order)
    }

    fn apply_limit(self, limit: u64) -> Self {
        self.limit(limit)
    }

    fn stream<'a, E>(&'a self, executor: &'a E) -> Result<BoxStream<'a, Result<P>>>
    where
        E: QueryExecutor,
    {
        ProjectionQuery::stream(self, executor)
    }

    async fn get<E>(&self, executor: &E) -> Result<Collection<P>>
    where
        E: QueryExecutor,
    {
        ProjectionQuery::get(self, executor).await
    }

    async fn paginate<E>(&self, executor: &E, pagination: Pagination) -> Result<Paginated<P>>
    where
        E: QueryExecutor,
    {
        ProjectionQuery::paginate(self, executor, pagination).await
    }
}

#[async_trait]
pub trait Datatable: Send + Sync + 'static {
    type Row: Serialize + Send + Sync + 'static;
    type Query: DatatableQuery<Self::Row>;

    const ID: &'static str;

    /// Base scoped query. Receives context so the implementor can scope
    /// by actor, tenant, or any other contextual constraint.
    fn query(ctx: &DatatableContext) -> Self::Query;

    /// Declared columns that participate in rendering, filtering, sorting, export.
    fn columns() -> Vec<DatatableColumn<Self::Row>>;

    /// Output-only computed fields. Mappings override columns with the same name.
    fn mappings() -> Vec<DatatableMapping<Self::Row>> {
        Vec::new()
    }

    /// Custom filter hook. Receives the query after auto-filters are applied
    /// so the implementor can add further refinements.
    async fn filters(_ctx: &DatatableContext, query: Self::Query) -> Result<Self::Query> {
        Ok(query)
    }

    /// Frontend filter metadata (controls, labels, options).
    async fn available_filters(_ctx: &DatatableContext) -> Result<Vec<DatatableFilterRow>> {
        Ok(Vec::new())
    }

    /// Relation-backed auto filters. These are opt-in and typed so relation
    /// filter inputs never resolve from arbitrary string paths.
    fn relation_filters() -> Vec<DatatableRelationFilter<Self::Row, Self::Query>> {
        Vec::new()
    }

    /// Default sort when no sort is specified in the request.
    fn default_sort() -> Vec<DatatableSort<Self::Row>> {
        Vec::new()
    }

    // -- provided output methods --------------------------------------------

    async fn json(
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<DatatableJsonResponse> {
        super::json::build_json_response::<Self>(app, actor, request).await
    }

    async fn download(
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<axum::response::Response> {
        super::download::build_download_response::<Self>(app, actor, request).await
    }

    async fn queue_email(
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
        recipient: &str,
    ) -> Result<DatatableExportAccepted> {
        super::export_job::dispatch_export::<Self>(app, actor, request, recipient).await
    }
}
