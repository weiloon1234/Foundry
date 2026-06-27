use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, ts_rs::TS, foundry::TS)]
struct DirectionalSkipRequest {
    id: String,
    #[serde(skip_deserializing)]
    server_only: String,
}

fn main() {}
