use std::collections::HashMap;

use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use futures_util::StreamExt;
use serde::Serialize;
use tokio::io::AsyncWriteExt as _;

use crate::foundation::{AppContext, Error, Result};
use crate::http::download::attachment_content_disposition;

use super::callback::{datatable_columns, datatable_mappings};
use super::column::DatatableColumn;
use super::context::DatatableContext;
use super::datatable_trait::{Datatable, DatatableQuery};
use super::mapping::DatatableMapping;

const XLSX_ROW_BUFFER: usize = 256;

/// Build an XLSX download response for a datatable.
///
/// Executes the full scoped + filtered query (no pagination) and writes
/// results into an XLSX workbook via `rust_xlsxwriter`.
pub async fn build_download_response<D>(
    app: &AppContext,
    actor: Option<&crate::auth::Actor>,
    request: super::request::DatatableRequest,
) -> Result<axum::response::Response>
where
    D: Datatable + ?Sized,
    D::Row: Serialize,
{
    let bytes = build_xlsx_bytes::<D>(app, actor, request).await?;

    let filename = format!("{}.xlsx", D::ID);
    axum::response::Response::builder()
        .header(
            CONTENT_TYPE,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        )
        .header(
            CONTENT_DISPOSITION,
            attachment_content_disposition(&filename),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| Error::message(format!("failed to build download response: {e}")))
}

pub(super) async fn response_to_xlsx_file(
    datatable_id: &str,
    response: axum::response::Response,
) -> Result<super::export::GeneratedDatatableExportFile> {
    let datatable_id = datatable_id.to_string();
    let filename = format!("{datatable_id}.xlsx");
    let (artifact, file) = crate::support::run_blocking(
        format!("datatable `{datatable_id}` export temp file"),
        move || {
            super::export::GeneratedDatatableExportFile::create(datatable_id, filename, Vec::new())
        },
    )
    .await?;
    let mut file = tokio::fs::File::from_std(file);
    let (_, body) = response.into_parts();
    let mut chunks = body.into_data_stream();

    while let Some(chunk) = chunks.next().await {
        let chunk = chunk
            .map_err(|error| Error::message(format!("failed to read export body: {error}")))?;
        file.write_all(&chunk).await.map_err(|error| {
            Error::message(format!(
                "failed to write datatable export artifact: {error}"
            ))
        })?;
    }
    file.flush().await.map_err(|error| {
        Error::message(format!(
            "failed to flush datatable export artifact: {error}"
        ))
    })?;
    drop(file);

    crate::support::run_blocking("datatable export artifact metadata", move || {
        artifact.refresh_size()
    })
    .await
}

/// Generate XLSX bytes from a datatable query (no pagination).
///
/// Shared between the download endpoint and the export job.
pub(super) async fn build_xlsx_bytes<D>(
    app: &AppContext,
    actor: Option<&crate::auth::Actor>,
    request: super::request::DatatableRequest,
) -> Result<Vec<u8>>
where
    D: Datatable + ?Sized,
    D::Row: Serialize,
{
    build_xlsx::<D, Vec<u8>, _>(app, actor, request, |workbook, _columns| {
        workbook
            .save_to_buffer()
            .map_err(|error| Error::message(format!("xlsx save error: {error}")))
    })
    .await
}

pub(super) async fn build_xlsx_file<D>(
    app: &AppContext,
    actor: Option<&crate::auth::Actor>,
    request: super::request::DatatableRequest,
) -> Result<super::export::GeneratedDatatableExportFile>
where
    D: Datatable + ?Sized,
    D::Row: Serialize,
{
    let datatable_id = D::ID.to_string();
    let filename = format!("{}.xlsx", D::ID);
    build_xlsx::<D, super::export::GeneratedDatatableExportFile, _>(
        app,
        actor,
        request,
        move |workbook, columns| {
            let (artifact, file) = super::export::GeneratedDatatableExportFile::create(
                datatable_id,
                filename,
                columns,
            )?;
            workbook
                .save_to_writer(file)
                .map_err(|error| Error::message(format!("xlsx save error: {error}")))?;
            artifact.refresh_size()
        },
    )
    .await
}

async fn build_xlsx<D, T, F>(
    app: &AppContext,
    actor: Option<&crate::auth::Actor>,
    request: super::request::DatatableRequest,
    finish: F,
) -> Result<T>
where
    D: Datatable + ?Sized,
    D::Row: Serialize,
    T: Send + 'static,
    F: FnOnce(&mut rust_xlsxwriter::Workbook, Vec<String>) -> Result<T> + Send + 'static,
{
    let ctx = DatatableContext::new(app, actor, &request);

    let columns = datatable_columns::<D>()?;
    let query = super::query_pipeline::prepare_query::<D>(&ctx, &columns).await?;
    let config = app.config().datatable()?;
    let query = apply_export_limit(query, config.max_export_rows);

    let db = app.database()?;
    let mut rows = query.stream(db.as_ref())?;
    let exportable_columns: Vec<DatatableColumn<D::Row>> = columns
        .into_iter()
        .filter(|column| column.exportable)
        .collect();
    let mappings = datatable_mappings::<D>()?;
    let app = app.clone();
    let actor = actor.cloned();
    let request = request.clone();
    let locale = ctx.locale.clone();
    let timezone = ctx.timezone.clone();
    let (sender, receiver) = tokio::sync::mpsc::channel(XLSX_ROW_BUFFER);

    let producer = async move {
        let mut row_count = 0usize;
        while let Some(row) = rows.next().await {
            let row = row?;
            row_count = row_count.saturating_add(1);
            ensure_export_row_count(D::ID, row_count, config.max_export_rows)?;
            sender.send(row).await.map_err(|_| {
                Error::message(format!(
                    "datatable `{}` XLSX writer stopped before consuming all rows",
                    D::ID
                ))
            })?;
        }
        Ok(row_count)
    };
    let writer =
        crate::support::run_blocking(format!("datatable `{}` XLSX build", D::ID), move || {
            build_xlsx_stream(
                receiver,
                &exportable_columns,
                &mappings,
                &app,
                actor.as_ref(),
                &request,
                locale,
                timezone,
                finish,
            )
        });

    let (producer_result, writer_result) = tokio::join!(producer, writer);
    match (producer_result, writer_result) {
        (_, Err(error)) => Err(error),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Ok(bytes)) => Ok(bytes),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_xlsx_stream<Row, T, F>(
    mut rows: tokio::sync::mpsc::Receiver<Row>,
    columns: &[DatatableColumn<Row>],
    mappings: &[DatatableMapping<Row>],
    app: &AppContext,
    actor: Option<&crate::auth::Actor>,
    request: &super::request::DatatableRequest,
    locale: Option<String>,
    timezone: crate::support::Timezone,
    finish: F,
) -> Result<T>
where
    Row: Serialize,
    F: FnOnce(&mut rust_xlsxwriter::Workbook, Vec<String>) -> Result<T>,
{
    use rust_xlsxwriter::{Format, Workbook};

    let ctx = DatatableContext::with_locale_and_timezone(app, actor, request, locale, timezone);
    let mapping_index: HashMap<&str, &DatatableMapping<Row>> = mappings
        .iter()
        .map(|mapping| (mapping.name.as_str(), mapping))
        .collect();

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet_with_constant_memory();
    let header_format = Format::new().set_bold();

    for column_index in 0..columns.len() {
        worksheet
            .set_column_width(column_index as u16, 15)
            .map_err(|error| Error::message(format!("xlsx format error: {error}")))?;
    }
    for (column_index, column) in columns.iter().enumerate() {
        worksheet
            .write_string_with_format(0, column_index as u16, &column.label, &header_format)
            .map_err(|error| Error::message(format!("xlsx write error: {error}")))?;
    }

    let mut row_index = 1u32;
    while let Some(row) = rows.blocking_recv() {
        let row_value = serde_json::to_value(&row)
            .map_err(|error| Error::message(format!("failed to serialize row: {error}")))?;
        let Some(object) = row_value.as_object() else {
            continue;
        };

        for (column_index, column) in columns.iter().enumerate() {
            let value = if let Some(mapping) = mapping_index.get(column.name.as_str()) {
                mapping.try_compute(&row, &ctx)?.into()
            } else {
                object
                    .get(&column.name)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
            };

            write_cell(worksheet, row_index, column_index as u16, &value)
                .map_err(|error| Error::message(format!("xlsx write error: {error}")))?;
        }
        row_index = row_index.saturating_add(1);
    }

    let column_names = columns.iter().map(|column| column.name.clone()).collect();
    finish(&mut workbook, column_names)
}

fn apply_export_limit<Row, Query>(query: Query, max_export_rows: u64) -> Query
where
    Query: DatatableQuery<Row>,
{
    if max_export_rows == 0 {
        query
    } else {
        query.apply_limit(max_export_rows.saturating_add(1))
    }
}

fn ensure_export_row_count(datatable_id: &str, rows: usize, max_export_rows: u64) -> Result<()> {
    if max_export_rows > 0 && rows as u64 > max_export_rows {
        return Err(Error::message(format!(
            "datatable `{datatable_id}` export exceeded datatable.max_export_rows ({max_export_rows}); narrow filters or raise the configured cap"
        )));
    }
    Ok(())
}

fn write_cell(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    col: u16,
    value: &serde_json::Value,
) -> std::result::Result<(), rust_xlsxwriter::XlsxError> {
    match value {
        serde_json::Value::Null => worksheet.write_string(row, col, ""),
        serde_json::Value::Bool(b) => worksheet.write_boolean(row, col, *b),
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                worksheet.write_number(row, col, f)
            } else {
                worksheet.write_string(row, col, n.to_string())
            }
        }
        serde_json::Value::String(s) => worksheet.write_string(row, col, s),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            worksheet.write_string(row, col, value.to_string())
        }
    }?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::{build_xlsx_stream, ensure_export_row_count, response_to_xlsx_file};
    use crate::config::ConfigRepository;
    use crate::datatable::{DatatableColumn, DatatableRequest};
    use crate::foundation::{AppContext, Container};
    use crate::support::Timezone;
    use crate::validation::RuleRegistry;

    #[derive(Clone, Serialize, crate::Projection)]
    struct ExportRow {
        id: i64,
        name: String,
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    #[test]
    fn export_row_count_allows_zero_unlimited_cap() {
        ensure_export_row_count("orders", 1_000_000, 0).unwrap();
    }

    #[test]
    fn export_row_count_rejects_rows_above_cap() {
        let error = ensure_export_row_count("orders", 501, 500).unwrap_err();

        assert!(error
            .to_string()
            .contains("datatable `orders` export exceeded datatable.max_export_rows (500)"));
    }

    #[test]
    fn constant_memory_writer_consumes_bounded_row_channel() {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender
            .try_send(ExportRow {
                id: 1,
                name: "First".to_string(),
            })
            .unwrap();
        sender
            .try_send(ExportRow {
                id: 2,
                name: "Second".to_string(),
            })
            .unwrap();
        drop(sender);

        let app = test_app();
        let request = DatatableRequest {
            page: 1,
            per_page: 20,
            sort: Vec::new(),
            filters: Vec::new(),
            search: None,
        };
        let columns = vec![
            DatatableColumn::field(ExportRow::ID).label("ID"),
            DatatableColumn::field(ExportRow::NAME).label("Name"),
        ];

        let bytes = build_xlsx_stream(
            receiver,
            &columns,
            &[],
            &app,
            None,
            &request,
            Some("en".to_string()),
            Timezone::utc(),
            |workbook, _columns| {
                workbook
                    .save_to_buffer()
                    .map_err(|error| crate::foundation::Error::message(error.to_string()))
            },
        )
        .unwrap();

        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.len() > 1_000);
    }

    #[tokio::test]
    async fn dynamic_download_response_streams_to_cleaned_up_file_artifact() {
        let response = axum::response::Response::new(axum::body::Body::from("PK-response"));
        let artifact = response_to_xlsx_file("orders", response).await.unwrap();
        let path = artifact.path().to_path_buf();

        assert_eq!(artifact.size(), 11);
        assert_eq!(artifact.read_bounded(32).await.unwrap(), b"PK-response");
        drop(artifact);

        assert!(!path.exists());
    }
}
