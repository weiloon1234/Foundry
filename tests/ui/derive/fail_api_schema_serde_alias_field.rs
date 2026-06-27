use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct AliasFieldApiSchema {
    #[serde(alias = "emailAddress")]
    email: String,
}

fn main() {}
