#[derive(foundry::Validate)]
struct DuplicateWireNameValidate {
    #[serde(rename = "email")]
    email_address: String,
    email: String,
}

fn main() {}
