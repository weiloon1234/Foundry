use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct DirectionalSkipResponse {
    id: String,
    #[serde(skip_serializing)]
    internal_token: String,
}

fn main() {}
