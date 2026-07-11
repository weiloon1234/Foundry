use foundry::Validate;
use serde::Deserialize;

#[derive(Deserialize, Validate)]
struct InvalidConditionalRequest {
    #[validate(required_if("missing", "yes"))]
    note: Option<String>,
}

fn main() {}
