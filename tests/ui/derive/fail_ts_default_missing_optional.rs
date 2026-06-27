use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, ts_rs::TS, foundry::TS)]
struct DefaultedTypeScriptRequest {
    #[serde(default)]
    page: u64,
}

fn main() {}
