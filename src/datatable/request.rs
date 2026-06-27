use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::database::OrderDirection;
use crate::support::{Collection, Date, DateTime};

// ---------------------------------------------------------------------------
// Filter operation/value/input
// ---------------------------------------------------------------------------

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DatatableFilterOp {
    Eq,
    NotEq,
    Like,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    Date,
    DateFrom,
    DateTo,
    Datetime,
    DatetimeFrom,
    DatetimeTo,
    Has,
    HasLike,
    LikeAny,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, ts_rs::TS, foundry_macros::TS)]
#[serde(rename_all = "snake_case")]
pub enum DatatableFilterValue {
    Text(String),
    Bool(bool),
    Number(#[ts(type = "number")] i64),
    Values(Vec<String>),
}

impl crate::openapi::ApiSchema for DatatableFilterValue {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"],
                },
                {
                    "type": "object",
                    "properties": { "bool": { "type": "boolean" } },
                    "required": ["bool"],
                },
                {
                    "type": "object",
                    "properties": { "number": { "type": "integer", "format": "int64" } },
                    "required": ["number"],
                },
                {
                    "type": "object",
                    "properties": {
                        "values": {
                            "type": "array",
                            "items": { "type": "string" },
                        }
                    },
                    "required": ["values"],
                },
            ],
        })
    }

    fn schema_name() -> &'static str {
        "DatatableFilterValue"
    }
}

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Debug,
    PartialEq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct DatatableFilterInput {
    pub field: String,
    pub op: DatatableFilterOp,
    pub value: DatatableFilterValue,
}

// ---------------------------------------------------------------------------
// Sort input
// ---------------------------------------------------------------------------

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct DatatableSortInput {
    pub field: String,
    #[ts(type = "\"asc\" | \"desc\"")]
    pub direction: OrderDirection,
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

const fn default_page() -> u64 {
    1
}

const fn default_per_page() -> u64 {
    20
}

#[derive(Serialize, Clone, Debug, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema)]
pub struct DatatableRequest {
    #[serde(default = "default_page")]
    #[ts(optional, as = "Option<f64>")]
    pub page: u64,
    #[serde(default = "default_per_page")]
    #[ts(optional, as = "Option<f64>")]
    pub per_page: u64,
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub sort: Vec<DatatableSortInput>,
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub filters: Vec<DatatableFilterInput>,
    #[serde(default)]
    #[ts(optional)]
    pub search: Option<String>,
}

#[derive(Deserialize)]
struct DatatableRequestWire {
    #[serde(default = "default_page", deserialize_with = "deserialize_page")]
    page: u64,
    #[serde(
        default = "default_per_page",
        deserialize_with = "deserialize_per_page"
    )]
    per_page: u64,
    #[serde(default)]
    sort: Option<DatatableSortWire>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    filters: Vec<DatatableFilterInput>,
    #[serde(default)]
    search: Option<String>,
    #[serde(flatten)]
    legacy: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DatatableSortWire {
    Structured(Vec<DatatableSortInput>),
    Legacy(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum U64Wire {
    Number(u64),
    String(String),
}

impl<'de> Deserialize<'de> for DatatableRequest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DatatableRequestWire::deserialize(deserializer)?;
        let mut filters = wire.filters;

        for (key, value) in wire.legacy {
            let Some(value) = query_string_value(value) else {
                continue;
            };
            if let Some(filter) = Self::parse_legacy_filter(&key, &value) {
                filters.push(filter);
            }
        }

        let sort = match wire.sort {
            Some(DatatableSortWire::Structured(sort)) => sort,
            Some(DatatableSortWire::Legacy(sort)) => {
                Self::parse_legacy_sort_values(&sort, wire.direction.as_deref())
            }
            None => Vec::new(),
        };

        Ok(Self {
            page: wire.page,
            per_page: wire.per_page,
            sort,
            filters,
            search: wire.search,
        })
    }
}

fn deserialize_page<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_u64_or_default(deserializer, default_page())
}

fn deserialize_per_page<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_u64_or_default(deserializer, default_per_page())
}

fn deserialize_u64_or_default<'de, D>(
    deserializer: D,
    default: u64,
) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = U64Wire::deserialize(deserializer)?;
    Ok(match value {
        U64Wire::Number(value) => value,
        U64Wire::String(value) => value.trim().parse().unwrap_or(default),
    })
}

fn query_string_value(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Array(values) => Some(
            values
                .into_iter()
                .filter_map(query_string_value)
                .collect::<Vec<_>>()
                .join(","),
        ),
        serde_json::Value::Object(_) => None,
    }
}

impl DatatableRequest {
    // -- helpers that search through self.filters -------------------------

    pub fn text(&self, name: &str) -> Option<&str> {
        self.filters.iter().find_map(|f| {
            if f.field == name {
                match &f.value {
                    DatatableFilterValue::Text(s) => Some(s.as_str()),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        self.filters.iter().find_map(|f| {
            if f.field == name {
                match &f.value {
                    DatatableFilterValue::Bool(b) => Some(*b),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    pub fn date(&self, name: &str) -> Option<Date> {
        self.filters.iter().find_map(|f| {
            if f.field == name {
                match &f.value {
                    DatatableFilterValue::Text(s) => s.parse().ok(),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    pub fn datetime(&self, name: &str) -> Option<DateTime> {
        self.filters.iter().find_map(|f| {
            if f.field == name {
                match &f.value {
                    DatatableFilterValue::Text(s) => s.parse().ok(),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    pub fn values(&self, name: &str) -> Collection<String> {
        self.filters
            .iter()
            .filter(|f| f.field == name)
            .filter_map(|f| match &f.value {
                DatatableFilterValue::Values(vs) => Some(vs.clone()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>()
            .into()
    }

    // -- legacy param construction -----------------------------------------

    /// Build a `DatatableRequest` from raw query-string params, normalizing
    /// the `f-` prefixed legacy convention into structured `DatatableFilterInput`s.
    pub fn from_query_params(params: &HashMap<String, String>) -> Self {
        let mut filters = Vec::new();

        for (key, value) in params {
            if let Some(filter) = Self::parse_legacy_filter(key, value) {
                filters.push(filter);
            }
        }

        let sort = Self::parse_legacy_sort(params);

        let page: u64 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
        let per_page: u64 = params
            .get("per_page")
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let search = params.get("search").cloned();

        Self {
            page,
            per_page,
            sort,
            filters,
            search,
        }
    }

    fn parse_legacy_sort(params: &HashMap<String, String>) -> Vec<DatatableSortInput> {
        let Some(sort) = params.get("sort") else {
            return Vec::new();
        };

        Self::parse_legacy_sort_values(sort, params.get("direction").map(String::as_str))
    }

    fn parse_legacy_sort_values(sort: &str, direction: Option<&str>) -> Vec<DatatableSortInput> {
        let directions = direction
            .map(|direction| {
                direction
                    .split(',')
                    .map(Self::parse_legacy_direction)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        sort.split(',')
            .enumerate()
            .filter_map(|(index, field)| {
                let field = field.trim();
                if field.is_empty() {
                    return None;
                }

                let (field, embedded_direction) = field
                    .strip_prefix('-')
                    .map(|stripped| (stripped, OrderDirection::Desc))
                    .unwrap_or((field, OrderDirection::Asc));
                let field = field.trim();
                if field.is_empty() {
                    return None;
                }

                Some(DatatableSortInput {
                    field: field.to_string(),
                    direction: directions.get(index).copied().unwrap_or(embedded_direction),
                })
            })
            .collect()
    }

    fn parse_legacy_direction(value: &str) -> OrderDirection {
        match value.trim().to_ascii_lowercase().as_str() {
            "desc" | "descending" | "-1" => OrderDirection::Desc,
            _ => OrderDirection::Asc,
        }
    }

    fn parse_legacy_filter(key: &str, value: &str) -> Option<DatatableFilterInput> {
        let stripped = key.strip_prefix("f-")?;

        // Order matters: longest prefixes first.
        if let Some(field) = stripped.strip_prefix("eq-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Eq,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("datetime-from-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::DatetimeFrom,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("datetime-to-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::DatetimeTo,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("datetime-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Datetime,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(fields) = stripped.strip_prefix("like-any-") {
            return Some(DatatableFilterInput {
                field: fields.to_string(),
                op: DatatableFilterOp::LikeAny,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("has-like-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::HasLike,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("has-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Has,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("date-from-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::DateFrom,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("date-to-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::DateTo,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("date-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Date,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("like-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Like,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("not-eq-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::NotEq,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("in-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::In,
                value: DatatableFilterValue::Values(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToString::to_string)
                        .collect(),
                ),
            });
        }
        if let Some(field) = stripped.strip_prefix("gt-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Gt,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("gte-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Gte,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("lt-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Lt,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }
        if let Some(field) = stripped.strip_prefix("lte-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Lte,
                value: DatatableFilterValue::Text(value.to_string()),
            });
        }

        // Fallback: treat the entire stripped key as a field name for exact match.
        Some(DatatableFilterInput {
            field: stripped.to_string(),
            op: DatatableFilterOp::Eq,
            value: DatatableFilterValue::Text(value.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DatatableFilterInput, DatatableFilterOp, DatatableFilterValue, DatatableRequest,
        DatatableSortInput,
    };
    use crate::database::OrderDirection;
    use axum::extract::Query;
    use axum::http::Uri;
    use std::collections::HashMap;

    fn query_params(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn from_query_params_uses_defaults_and_ignores_non_filter_params() {
        let params = query_params(&[("unused", "name"), ("direction", "asc")]);

        let request = DatatableRequest::from_query_params(&params);

        assert_eq!(request.page, 1);
        assert_eq!(request.per_page, 20);
        assert!(request.sort.is_empty());
        assert!(request.filters.is_empty());
        assert_eq!(request.search, None);
    }

    #[test]
    fn from_query_params_normalizes_legacy_sort_params() {
        let params = query_params(&[
            ("sort", "name,-created_at,total"),
            ("direction", "asc,asc,desc"),
        ]);

        let request = DatatableRequest::from_query_params(&params);

        assert_eq!(
            request.sort,
            vec![
                DatatableSortInput {
                    field: "name".to_string(),
                    direction: OrderDirection::Asc,
                },
                DatatableSortInput {
                    field: "created_at".to_string(),
                    direction: OrderDirection::Asc,
                },
                DatatableSortInput {
                    field: "total".to_string(),
                    direction: OrderDirection::Desc,
                },
            ]
        );
    }

    #[test]
    fn deserialize_accepts_structured_json_shape() {
        let request: DatatableRequest = serde_json::from_value(serde_json::json!({
            "page": 4,
            "per_page": 15,
            "sort": [{ "field": "total", "direction": "desc" }],
            "filters": [{
                "field": "status",
                "op": "eq",
                "value": { "text": "paid" }
            }],
            "search": "invoice"
        }))
        .unwrap();

        assert_eq!(request.page, 4);
        assert_eq!(request.per_page, 15);
        assert_eq!(
            request.sort,
            vec![DatatableSortInput {
                field: "total".to_string(),
                direction: OrderDirection::Desc,
            }]
        );
        assert_eq!(
            request.filters,
            vec![filter(
                "status",
                DatatableFilterOp::Eq,
                DatatableFilterValue::Text("paid".to_string())
            )]
        );
        assert_eq!(request.search.as_deref(), Some("invoice"));
    }

    #[test]
    fn axum_query_deserialize_accepts_legacy_datatable_params() {
        let uri: Uri = "/datatable?page=2&per_page=25&search=orders&sort=total,-created_at&direction=desc,asc&f-gte-total=5000&f-in-status=paid,pending"
            .parse()
            .unwrap();

        let Query(request) = Query::<DatatableRequest>::try_from_uri(&uri).unwrap();

        assert_eq!(request.page, 2);
        assert_eq!(request.per_page, 25);
        assert_eq!(request.search.as_deref(), Some("orders"));
        assert_eq!(
            request.sort,
            vec![
                DatatableSortInput {
                    field: "total".to_string(),
                    direction: OrderDirection::Desc,
                },
                DatatableSortInput {
                    field: "created_at".to_string(),
                    direction: OrderDirection::Asc,
                },
            ]
        );
        assert!(request.filters.contains(&filter(
            "total",
            DatatableFilterOp::Gte,
            DatatableFilterValue::Text("5000".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "status",
            DatatableFilterOp::In,
            DatatableFilterValue::Values(vec!["paid".to_string(), "pending".to_string()])
        )));
    }

    #[test]
    fn axum_query_deserialize_accepts_explicit_eq_filter_prefix() {
        let uri: Uri =
            "/datatable?f-eq-in-stock=yes&f-eq-date-range=today&f-in-status=paid,pending"
                .parse()
                .unwrap();

        let Query(request) = Query::<DatatableRequest>::try_from_uri(&uri).unwrap();

        assert!(request.filters.contains(&filter(
            "in-stock",
            DatatableFilterOp::Eq,
            DatatableFilterValue::Text("yes".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "date-range",
            DatatableFilterOp::Eq,
            DatatableFilterValue::Text("today".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "status",
            DatatableFilterOp::In,
            DatatableFilterValue::Values(vec!["paid".to_string(), "pending".to_string()])
        )));
    }

    #[test]
    fn from_query_params_normalizes_legacy_filter_prefixes() {
        let params = query_params(&[
            ("page", "3"),
            ("per_page", "50"),
            ("search", "orders"),
            ("f-eq-category", "retail"),
            ("f-status", "active"),
            ("f-not-eq-status", "archived"),
            ("f-like-name", "ali"),
            ("f-like-any-name|email", "bob"),
            ("f-in-role", "admin, editor"),
            ("f-gt-age", "18"),
            ("f-gte-total", "100"),
            ("f-lt-items", "5"),
            ("f-lte-total", "200"),
            ("f-date-from-created_at", "2026-01-01"),
            ("f-date-to-created_at", "2026-01-31"),
            ("f-datetime-to-published_at", "2026-01-31T23:59:59Z"),
            ("f-has-profile_id", "1"),
            ("f-has-like-author_name", "sam"),
        ]);

        let request = DatatableRequest::from_query_params(&params);

        assert_eq!(request.page, 3);
        assert_eq!(request.per_page, 50);
        assert_eq!(request.search.as_deref(), Some("orders"));
        assert_eq!(request.filters.len(), 15);
        assert!(request.filters.contains(&filter(
            "category",
            DatatableFilterOp::Eq,
            DatatableFilterValue::Text("retail".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "status",
            DatatableFilterOp::Eq,
            DatatableFilterValue::Text("active".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "status",
            DatatableFilterOp::NotEq,
            DatatableFilterValue::Text("archived".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "name",
            DatatableFilterOp::Like,
            DatatableFilterValue::Text("ali".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "name|email",
            DatatableFilterOp::LikeAny,
            DatatableFilterValue::Text("bob".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "role",
            DatatableFilterOp::In,
            DatatableFilterValue::Values(vec!["admin".to_string(), "editor".to_string()])
        )));
        assert!(request.filters.contains(&filter(
            "age",
            DatatableFilterOp::Gt,
            DatatableFilterValue::Text("18".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "total",
            DatatableFilterOp::Gte,
            DatatableFilterValue::Text("100".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "items",
            DatatableFilterOp::Lt,
            DatatableFilterValue::Text("5".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "total",
            DatatableFilterOp::Lte,
            DatatableFilterValue::Text("200".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "created_at",
            DatatableFilterOp::DateFrom,
            DatatableFilterValue::Text("2026-01-01".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "created_at",
            DatatableFilterOp::DateTo,
            DatatableFilterValue::Text("2026-01-31".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "published_at",
            DatatableFilterOp::DatetimeTo,
            DatatableFilterValue::Text("2026-01-31T23:59:59Z".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "profile_id",
            DatatableFilterOp::Has,
            DatatableFilterValue::Text("1".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "author_name",
            DatatableFilterOp::HasLike,
            DatatableFilterValue::Text("sam".to_string())
        )));
    }

    #[test]
    fn explicit_eq_prefix_disambiguates_operator_prefixed_field_names() {
        let params = query_params(&[
            ("f-eq-in-stock", "yes"),
            ("f-eq-date-range", "today"),
            ("f-in-status", "paid,pending"),
        ]);

        let request = DatatableRequest::from_query_params(&params);

        assert!(request.filters.contains(&filter(
            "in-stock",
            DatatableFilterOp::Eq,
            DatatableFilterValue::Text("yes".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "date-range",
            DatatableFilterOp::Eq,
            DatatableFilterValue::Text("today".to_string())
        )));
        assert!(request.filters.contains(&filter(
            "status",
            DatatableFilterOp::In,
            DatatableFilterValue::Values(vec!["paid".to_string(), "pending".to_string()])
        )));
    }

    fn filter(
        field: &str,
        op: DatatableFilterOp,
        value: DatatableFilterValue,
    ) -> DatatableFilterInput {
        DatatableFilterInput {
            field: field.to_string(),
            op,
            value,
        }
    }
}
