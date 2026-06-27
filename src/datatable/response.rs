use serde::{Deserialize, Serialize};

use crate::auth::Actor;
use crate::support::{GuardId, PermissionId, RoleId};

use super::column::DatatableColumn;
use super::filter_meta::DatatableFilterRow;
use super::request::{DatatableFilterInput, DatatableSortInput};

// ---------------------------------------------------------------------------
// JSON response
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableJsonResponse {
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
    pub columns: Vec<DatatableColumnMeta>,
    pub filters: Vec<DatatableFilterRow>,
    pub pagination: DatatablePaginationMeta,
    pub applied_filters: Vec<DatatableFilterInput>,
    pub sorts: Vec<DatatableSortInput>,
}

// ---------------------------------------------------------------------------
// Column metadata (sent to frontend)
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableColumnMeta {
    pub name: String,
    pub label: String,
    pub sortable: bool,
    pub filterable: bool,
    pub exportable: bool,
    pub relation: Option<String>,
}

impl DatatableColumnMeta {
    pub(crate) fn from_column<Row>(column: &DatatableColumn<Row>) -> Self {
        Self {
            name: column.name.clone(),
            label: column.label.clone(),
            sortable: column.sortable,
            filterable: column.filterable,
            exportable: column.exportable,
            relation: column.relation.clone(),
        }
    }
}

impl crate::openapi::ApiSchema for DatatableColumnMeta {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "label": { "type": "string" },
                "sortable": { "type": "boolean" },
                "filterable": { "type": "boolean" },
                "exportable": { "type": "boolean" },
                "relation": { "type": "string", "nullable": true },
            },
            "required": ["name", "label", "sortable", "filterable", "exportable", "relation"],
        })
    }

    fn schema_name() -> &'static str {
        "DatatableColumnMeta"
    }
}

// ---------------------------------------------------------------------------
// Pagination metadata
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
pub struct DatatablePaginationMeta {
    #[ts(type = "number")]
    pub page: u64,
    #[ts(type = "number")]
    pub per_page: u64,
    #[ts(type = "number")]
    pub total: u64,
    #[ts(type = "number")]
    pub total_pages: u64,
}

impl DatatablePaginationMeta {
    pub fn new(page: u64, per_page: u64, total: u64) -> Self {
        let total_pages = if per_page == 0 {
            0
        } else {
            total.div_ceil(per_page)
        };
        Self {
            page,
            per_page,
            total,
            total_pages,
        }
    }
}

impl crate::openapi::ApiSchema for DatatableJsonResponse {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "rows": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": true,
                    },
                },
                "columns": {
                    "type": "array",
                    "items": <DatatableColumnMeta as crate::openapi::ApiSchema>::schema(),
                },
                "filters": {
                    "type": "array",
                    "items": <DatatableFilterRow as crate::openapi::ApiSchema>::schema(),
                },
                "pagination": <DatatablePaginationMeta as crate::openapi::ApiSchema>::schema(),
                "applied_filters": {
                    "type": "array",
                    "items": <DatatableFilterInput as crate::openapi::ApiSchema>::schema(),
                },
                "sorts": {
                    "type": "array",
                    "items": <DatatableSortInput as crate::openapi::ApiSchema>::schema(),
                },
            },
            "required": [
                "rows",
                "columns",
                "filters",
                "pagination",
                "applied_filters",
                "sorts",
            ],
        })
    }

    fn schema_name() -> &'static str {
        "DatatableJsonResponse"
    }
}

// ---------------------------------------------------------------------------
// Export accepted response
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, foundry_macros::AppEnum)]
pub enum DatatableExportStatus {
    Queued,
}

impl DatatableExportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
        }
    }
}

impl std::fmt::Display for DatatableExportStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Serialize, Debug, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
pub struct DatatableExportAccepted {
    pub datatable_id: String,
    pub recipient: String,
    pub status: DatatableExportStatus,
}

// ---------------------------------------------------------------------------
// Actor snapshot (serializable for queued jobs)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableActorSnapshot {
    pub id: String,
    pub guard: String,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
}

impl From<&Actor> for DatatableActorSnapshot {
    fn from(actor: &Actor) -> Self {
        Self {
            id: actor.id.clone(),
            guard: actor.guard.as_str().to_string(),
            roles: actor.roles.iter().map(|r| r.as_str().to_string()).collect(),
            permissions: actor
                .permissions
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
        }
    }
}

impl From<&DatatableActorSnapshot> for Actor {
    fn from(snap: &DatatableActorSnapshot) -> Self {
        Actor::new(&snap.id, GuardId::owned(&snap.guard))
            .with_roles(snap.roles.iter().map(RoleId::owned))
            .with_permissions(snap.permissions.iter().map(PermissionId::owned))
    }
}

#[cfg(test)]
mod tests {
    use crate::openapi::ApiSchema;

    use super::{DatatableExportAccepted, DatatableExportStatus, DatatableJsonResponse};

    #[test]
    fn datatable_json_response_has_api_schema() {
        let schema = DatatableJsonResponse::schema();

        assert_eq!(
            DatatableJsonResponse::schema_name(),
            "DatatableJsonResponse"
        );
        assert_eq!(
            schema.get("type").and_then(serde_json::Value::as_str),
            Some("object")
        );
        assert!(schema
            .get("properties")
            .and_then(|properties| properties.get("rows"))
            .is_some());
        let column_schema = &schema["properties"]["columns"]["items"];
        assert_eq!(
            column_schema["properties"]["exportable"],
            serde_json::json!({ "type": "boolean" })
        );
        assert!(column_schema["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field.as_str() == Some("exportable"))));
        assert!(column_schema["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field.as_str() == Some("relation"))));
        assert_eq!(
            column_schema["properties"]["relation"],
            serde_json::json!({ "type": "string", "nullable": true })
        );
        assert!(schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|required| required
                .iter()
                .any(|field| field.as_str() == Some("pagination"))));
        assert!(schema["properties"]["pagination"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field.as_str() == Some("total_pages"))));

        let filter_field_schema =
            &schema["properties"]["filters"]["items"]["properties"]["fields"]["items"];
        assert_eq!(
            filter_field_schema["properties"]["kind"]["enum"],
            serde_json::json!(["text", "number", "select", "checkbox", "date", "date_time"])
        );
        assert_eq!(
            filter_field_schema["properties"]["binding"]["properties"]["op"]["enum"],
            serde_json::json!([
                "eq",
                "not_eq",
                "like",
                "gt",
                "gte",
                "lt",
                "lte",
                "in",
                "date",
                "date_from",
                "date_to",
                "datetime",
                "datetime_from",
                "datetime_to",
                "has",
                "has_like",
                "like_any"
            ])
        );
        assert_eq!(
            filter_field_schema["properties"]["options"]["properties"]["items"]["items"]
                ["properties"]["value"],
            serde_json::json!({ "type": "string" })
        );

        let applied_filter_schema = &schema["properties"]["applied_filters"]["items"];
        assert_eq!(
            applied_filter_schema["properties"]["op"]["enum"],
            filter_field_schema["properties"]["binding"]["properties"]["op"]["enum"]
        );
        assert!(applied_filter_schema["properties"]["value"]["oneOf"]
            .as_array()
            .is_some_and(|variants| variants.iter().any(|variant| variant
                .get("required")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|required| required
                    .iter()
                    .any(|field| field.as_str() == Some("values"))))));

        assert_eq!(
            schema["properties"]["sorts"]["items"]["properties"]["direction"]["enum"],
            serde_json::json!(["asc", "desc"])
        );
    }

    #[test]
    fn datatable_export_accepted_has_typed_status_schema() {
        let response = DatatableExportAccepted {
            datatable_id: "orders".to_string(),
            recipient: "ops@example.com".to_string(),
            status: DatatableExportStatus::Queued,
        };

        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["status"], serde_json::json!("queued"));

        let schema = DatatableExportAccepted::schema();
        assert_eq!(
            DatatableExportAccepted::schema_name(),
            "DatatableExportAccepted"
        );
        assert_eq!(schema["properties"]["status"]["type"], "string");
        assert_eq!(
            schema["properties"]["status"]["enum"],
            serde_json::json!(["queued"])
        );
    }
}
