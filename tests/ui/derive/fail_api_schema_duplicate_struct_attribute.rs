use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
#[validate(attributes(email = "email address", email = "login email"))]
struct DuplicateStructAttribute {
    #[validate(required)]
    email: String,
}

fn main() {}
