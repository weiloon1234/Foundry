use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::TS)]
#[ts(export)]
struct BadExport {
    value: String,
}

fn main() {}
