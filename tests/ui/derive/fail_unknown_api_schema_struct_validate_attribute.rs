use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
#[validate(afetr(validate_payload))]
struct TypoStructValidateAttribute {
    email: String,
}

fn main() {}
