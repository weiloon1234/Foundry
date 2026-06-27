use serde::Serialize;

#[derive(Serialize, foundry::ApiSchema, foundry::TS)]
struct SparseResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    message: Option<String>,
}

impl ts_rs::TS for SparseResponse {
    type WithoutGenerics = Self;

    fn name() -> String {
        "SparseResponse".to_string()
    }

    fn decl() -> String {
        "type SparseResponse = { message?: string | null };".to_string()
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn inline() -> String {
        "{ message?: string | null }".to_string()
    }

    fn inline_flattened() -> String {
        Self::inline()
    }
}

fn main() {}
