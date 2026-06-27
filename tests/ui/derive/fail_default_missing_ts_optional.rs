use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, ts_rs::TS, foundry::ApiSchema)]
struct DefaultedRequest {
    #[serde(default)]
    page: u64,
}

fn main() {}
