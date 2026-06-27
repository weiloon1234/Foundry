use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct TsRenameApiSchema {
    #[ts(rename = "emailAddress")]
    email: String,
}

fn main() {}
