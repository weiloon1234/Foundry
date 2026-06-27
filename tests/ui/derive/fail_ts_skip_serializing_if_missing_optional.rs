use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::TS)]
struct SparseResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn main() {}
