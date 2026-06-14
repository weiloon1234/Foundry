use std::collections::HashMap;

use serde::Serialize;

use crate::database::Pagination;
use crate::foundation::{AppContext, Error, Result};

use super::callback::{datatable_available_filters, datatable_columns, datatable_mappings};
use super::column::DatatableColumn;
use super::context::DatatableContext;
use super::datatable_trait::{Datatable, DatatableQuery};
use super::mapping::DatatableMapping;
use super::response::{DatatableColumnMeta, DatatableJsonResponse, DatatablePaginationMeta};

/// Build a paginated JSON response for a datatable.
pub async fn build_json_response<D>(
    app: &AppContext,
    actor: Option<&crate::auth::Actor>,
    request: super::request::DatatableRequest,
) -> Result<DatatableJsonResponse>
where
    D: Datatable + ?Sized,
    D::Row: Serialize,
{
    let ctx = DatatableContext::new(app, actor, &request);

    let columns = datatable_columns::<D>()?;
    let query = super::query_pipeline::prepare_query::<D>(&ctx, &columns).await?;

    let config = app.config().datatable()?;
    let pagination = Pagination::new(
        request.page,
        effective_per_page(request.per_page, config.max_per_page),
    );
    let db = app.database()?;
    let paginated = query.paginate(db.as_ref(), pagination).await?;

    let mappings = datatable_mappings::<D>()?;
    let rows = build_rows(&paginated.data, &columns, &mappings, &ctx)?;

    let column_meta: Vec<DatatableColumnMeta> = columns
        .iter()
        .map(|c| DatatableColumnMeta {
            name: c.name.clone(),
            label: c.label.clone(),
            sortable: c.sortable,
            filterable: c.filterable,
        })
        .collect();

    let filters = datatable_available_filters::<D>(&ctx).await?;

    let pagination_meta = DatatablePaginationMeta::new(
        paginated.pagination.page,
        paginated.pagination.per_page,
        paginated.total,
    );

    Ok(DatatableJsonResponse {
        rows,
        columns: column_meta,
        filters,
        pagination: pagination_meta,
        applied_filters: request.filters,
        sorts: request.sort,
    })
}

fn build_rows<Row>(
    data: &crate::support::Collection<Row>,
    columns: &[DatatableColumn<Row>],
    mappings: &[DatatableMapping<Row>],
    ctx: &DatatableContext,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>>
where
    Row: Serialize,
{
    let mapping_index: HashMap<&str, &DatatableMapping<Row>> =
        mappings.iter().map(|m| (m.name.as_str(), m)).collect();

    let mut rows = Vec::with_capacity(data.len());

    for row in data.iter() {
        let mut map = serde_json::Map::new();

        let row_value = serde_json::to_value(row)
            .map_err(|e| Error::message(format!("failed to serialize row: {e}")))?;

        if let serde_json::Value::Object(obj) = row_value {
            for col in columns {
                if let Some(mapping) = mapping_index.get(col.name.as_str()) {
                    let value: serde_json::Value = mapping.try_compute(row, ctx)?.into();
                    map.insert(col.name.clone(), value);
                } else if let Some(val) = obj.get(&col.name) {
                    map.insert(col.name.clone(), val.clone());
                }
            }
        }

        for mapping in mappings {
            if !map.contains_key(&mapping.name) {
                let value: serde_json::Value = mapping.try_compute(row, ctx)?.into();
                map.insert(mapping.name.clone(), value);
            }
        }

        rows.push(map);
    }

    Ok(rows)
}

pub(crate) fn effective_per_page(requested: u64, max_per_page: u64) -> u64 {
    if max_per_page == 0 {
        requested
    } else {
        requested.min(max_per_page)
    }
}

#[cfg(test)]
mod tests {
    use super::effective_per_page;

    #[test]
    fn effective_per_page_clamps_to_configured_cap() {
        assert_eq!(effective_per_page(1_000, 500), 500);
        assert_eq!(effective_per_page(50, 500), 50);
        assert_eq!(effective_per_page(1_000, 0), 1_000);
    }
}
