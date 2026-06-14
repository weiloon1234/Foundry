use async_trait::async_trait;
use foundry::prelude::*;
use serde::Serialize;

const FIXTURE_USERS_TABLE: &str = "fixture_users";
const FIXTURE_REPORTS_TABLE: &str = "fixture_reports";

#[derive(Debug, Serialize, foundry::Model)]
#[foundry(table = FIXTURE_USERS_TABLE, primary_key_strategy = "manual")]
pub struct FixtureUser {
    id: i64,
    email: String,
}

pub struct FixtureUsersDatatable;

#[async_trait]
impl Datatable for FixtureUsersDatatable {
    type Row = FixtureUser;
    type Query = ModelQuery<FixtureUser>;

    const ID: &'static str = "fixture-users";

    fn query(_ctx: &DatatableContext) -> Self::Query {
        FixtureUser::query()
    }

    fn columns() -> Vec<DatatableColumn<Self::Row>> {
        vec![
            DatatableColumn::field(FixtureUser::ID)
                .label("ID")
                .sortable()
                .exportable(),
            DatatableColumn::field(FixtureUser::EMAIL)
                .label("Email")
                .sortable()
                .filterable()
                .exportable(),
        ]
    }

    fn default_sort() -> Vec<DatatableSort<Self::Row>> {
        vec![DatatableSort::asc(FixtureUser::ID)]
    }
}

#[derive(Clone, Debug, Serialize, foundry::Projection)]
pub struct FixtureReportRow {
    category: String,
    total: Option<i64>,
}

pub struct FixtureReportDatatable;

#[async_trait]
impl Datatable for FixtureReportDatatable {
    type Row = FixtureReportRow;
    type Query = ProjectionQuery<FixtureReportRow>;

    const ID: &'static str = "fixture-report";

    fn query(_ctx: &DatatableContext) -> Self::Query {
        FixtureReportRow::source(FIXTURE_REPORTS_TABLE)
            .select_source(FixtureReportRow::CATEGORY, FIXTURE_REPORTS_TABLE)
            .select_aggregate(AggregateProjection::<Option<i64>>::sum(
                FixtureReportRow::TOTAL.column_ref_from(FIXTURE_REPORTS_TABLE),
                FixtureReportRow::TOTAL.alias(),
            ))
            .group_by(FixtureReportRow::CATEGORY.column_ref_from(FIXTURE_REPORTS_TABLE))
    }

    fn columns() -> Vec<DatatableColumn<Self::Row>> {
        vec![
            DatatableColumn::field(FixtureReportRow::CATEGORY)
                .label("Category")
                .sortable()
                .filter_by(FixtureReportRow::CATEGORY.column_ref_from(FIXTURE_REPORTS_TABLE)),
            DatatableColumn::field(FixtureReportRow::TOTAL)
                .label("Total")
                .sortable()
                .filter_having(Expr::function(
                    "SUM",
                    [Expr::column(FixtureReportRow::TOTAL.column_ref_from(
                        FIXTURE_REPORTS_TABLE,
                    ))],
                ))
                .exportable(),
        ]
    }

    fn default_sort() -> Vec<DatatableSort<Self::Row>> {
        vec![DatatableSort::desc(FixtureReportRow::TOTAL)]
    }

    async fn available_filters(_ctx: &DatatableContext) -> Result<Vec<DatatableFilterRow>> {
        Ok(vec![DatatableFilterRow::pair(
            DatatableFilterField::text_search_fields(
                "category_query",
                "Category",
                [FixtureReportRow::CATEGORY],
            ),
            DatatableFilterField::number("minimum_total", "Minimum Total").bind(
                "total",
                DatatableFilterOp::Gte,
                DatatableFilterValueKind::Integer,
            ),
        )])
    }
}

pub fn register(registrar: &ServiceRegistrar) -> Result<()> {
    registrar.register_datatable::<FixtureUsersDatatable>()?;
    registrar.register_datatable::<FixtureReportDatatable>()?;
    Ok(())
}
