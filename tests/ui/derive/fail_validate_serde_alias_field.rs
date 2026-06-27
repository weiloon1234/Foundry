#[derive(foundry::Validate)]
struct AliasFieldValidate {
    #[serde(alias = "emailAddress")]
    email: String,
}

fn main() {}
