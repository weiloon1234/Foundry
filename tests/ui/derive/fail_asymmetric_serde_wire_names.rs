use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, foundry::ApiSchema)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
struct AsymmetricDto {
    display_name: String,
}

fn main() {}
