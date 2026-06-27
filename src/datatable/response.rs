use serde::{Deserialize, Serialize};

use crate::auth::Actor;
use crate::support::{GuardId, PermissionId, RoleId};

use super::filter_meta::DatatableFilterRow;
use super::request::{DatatableFilterInput, DatatableSortInput};

// ---------------------------------------------------------------------------
// JSON response
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableJsonResponse {
    #[ts(type = "Array<Record<string, unknown>>")]
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
    pub columns: Vec<DatatableColumnMeta>,
    pub filters: Vec<DatatableFilterRow>,
    pub pagination: DatatablePaginationMeta,
    pub applied_filters: Vec<DatatableFilterInput>,
    pub sorts: Vec<DatatableSortInput>,
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
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "label": {"type": "string"},
                            "sortable": {"type": "boolean"},
                            "filterable": {"type": "boolean"},
                        },
                        "required": ["name", "label", "sortable", "filterable"],
                    },
                },
                "filters": {"type": "array", "items": {"type": "object"}},
                "pagination": {
                    "type": "object",
                    "properties": {
                        "page": {"type": "integer", "format": "uint64"},
                        "per_page": {"type": "integer", "format": "uint64"},
                        "total": {"type": "integer", "format": "uint64"},
                        "total_pages": {"type": "integer", "format": "uint64"},
                    },
                    "required": ["page", "per_page", "total", "total_pages"],
                },
                "applied_filters": {"type": "array", "items": {"type": "object"}},
                "sorts": {"type": "array", "items": {"type": "object"}},
            },
            "required": ["rows", "columns", "filters", "pagination", "applied_filters", "sorts"],
        })
    }

    fn schema_name() -> &'static str {
        "DatatableJsonResponse"
    }
}

inventory::submit! {
    crate::openapi::ApiSchemaDefinition {
        name: "DatatableJsonResponse",
        schema_fn: <DatatableJsonResponse as crate::openapi::ApiSchema>::schema,
    }
}

// ---------------------------------------------------------------------------
// Column metadata (sent to frontend)
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableColumnMeta {
    pub name: String,
    pub label: String,
    pub sortable: bool,
    pub filterable: bool,
}

// ---------------------------------------------------------------------------
// Pagination metadata
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, ts_rs::TS, foundry_macros::TS)]
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

// ---------------------------------------------------------------------------
// Export accepted response
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableExportAccepted {
    pub datatable_id: String,
    pub recipient: String,
    pub status: String,
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
