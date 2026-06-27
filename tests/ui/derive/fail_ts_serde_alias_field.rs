use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::TS)]
struct AliasFieldTs {
    #[serde(alias = "emailAddress")]
    email: String,
}

fn main() {}
