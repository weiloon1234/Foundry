use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
#[validate(messages(email(unique "This email is already registered.")))]
struct MalformedStructValidationMetadata {
    email: String,
}

fn main() {}
