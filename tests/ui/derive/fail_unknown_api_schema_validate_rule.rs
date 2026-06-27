use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct TypoValidateRule {
    #[validate(emial)]
    email: String,
}

fn main() {}
