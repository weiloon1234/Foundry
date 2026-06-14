use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::database::OrderDirection;
use crate::support::{Collection, Date, DateTime};

// ---------------------------------------------------------------------------
// Filter operation/value/input
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableFilterInput {
    pub field: String,
    pub op: DatatableFilterOp,
    pub value: DatatableFilterValue,
}

// ---------------------------------------------------------------------------
// Sort input
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
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

#[derive(Serialize, Deserialize, Clone, Debug, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableRequest {
    #[serde(default = "default_page")]
    #[ts(type = "number")]
    pub page: u64,
    #[serde(default = "default_per_page")]
    #[ts(type = "number")]
    pub per_page: u64,
    #[serde(default)]
    pub sort: Vec<DatatableSortInput>,
    #[serde(default)]
    pub filters: Vec<DatatableFilterInput>,
    pub search: Option<String>,
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

        let page: u64 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
        let per_page: u64 = params
            .get("per_page")
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let search = params.get("search").cloned();

        Self {
            page,
            per_page,
            sort: Vec::new(),
            filters,
            search,
        }
    }

    fn parse_legacy_filter(key: &str, value: &str) -> Option<DatatableFilterInput> {
        let stripped = key.strip_prefix("f-")?;

        // Order matters: longest prefixes first.
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
        if let Some(field) = stripped.strip_prefix("gte-") {
            return Some(DatatableFilterInput {
                field: field.to_string(),
                op: DatatableFilterOp::Gte,
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
    use super::{DatatableFilterInput, DatatableFilterOp, DatatableFilterValue, DatatableRequest};
    use std::collections::HashMap;

    fn query_params(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn from_query_params_uses_defaults_and_ignores_non_filter_params() {
        let params = query_params(&[("sort", "name"), ("direction", "asc")]);

        let request = DatatableRequest::from_query_params(&params);

        assert_eq!(request.page, 1);
        assert_eq!(request.per_page, 20);
        assert!(request.filters.is_empty());
        assert_eq!(request.search, None);
    }

    #[test]
    fn from_query_params_normalizes_legacy_filter_prefixes() {
        let params = query_params(&[
            ("page", "3"),
            ("per_page", "50"),
            ("search", "orders"),
            ("f-status", "active"),
            ("f-like-name", "ali"),
            ("f-like-any-name|email", "bob"),
            ("f-gte-total", "100"),
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
        assert_eq!(request.filters.len(), 10);
        assert!(request.filters.contains(&filter(
            "status",
            DatatableFilterOp::Eq,
            DatatableFilterValue::Text("active".to_string())
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
            "total",
            DatatableFilterOp::Gte,
            DatatableFilterValue::Text("100".to_string())
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
