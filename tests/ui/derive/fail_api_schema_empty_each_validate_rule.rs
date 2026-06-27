use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct EmptyEachValidateRule {
    #[validate(each())]
    tags: Vec<String>,
}

fn main() {}
