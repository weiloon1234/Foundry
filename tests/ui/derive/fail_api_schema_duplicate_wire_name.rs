use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct DuplicateWireNameApiSchema {
    #[serde(rename = "email")]
    email_address: String,
    email: String,
}

fn main() {}
