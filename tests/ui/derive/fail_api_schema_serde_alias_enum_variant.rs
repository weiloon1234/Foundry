use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
enum AliasEnumVariantApiSchema {
    #[serde(alias = "queued")]
    Pending,
}

fn main() {}
