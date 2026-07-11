use foundry::Validate;
use serde::Deserialize;

#[derive(Deserialize, Validate)]
struct InvalidDistinctRequest {
    #[validate(distinct)]
    tag: String,
}

fn main() {}
