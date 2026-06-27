use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct SingleArgValidateRuleExtraArg {
    #[validate(min(3, 5))]
    name: String,
}

fn main() {}
