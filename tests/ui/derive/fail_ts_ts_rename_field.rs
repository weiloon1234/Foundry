use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::TS)]
struct TsRenameTs {
    #[ts(rename = "emailAddress")]
    email: String,
}

fn main() {}
