use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct NoArgValidateRulePositionalArg {
    #[validate(required("email is required"))]
    email: String,
}

fn main() {}
