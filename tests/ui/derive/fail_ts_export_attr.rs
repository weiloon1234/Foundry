use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
#[ts(export)]
struct BadExport {
    value: String,
}

fn main() {}
