use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct BlankRuleMessage {
    #[validate(required(message = ""))]
    email: String,
}

fn main() {}
