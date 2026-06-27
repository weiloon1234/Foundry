use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::TS)]
enum AliasEnumVariantTs {
    #[serde(alias = "queued")]
    Pending,
}

fn main() {}
