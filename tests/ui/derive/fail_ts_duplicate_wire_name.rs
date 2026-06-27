use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::TS)]
struct DuplicateWireNameTs {
    #[serde(rename = "email")]
    email_address: String,
    email: String,
}

fn main() {}
