#[derive(foundry::Validate)]
struct SkipRule {
    #[serde(skip)]
    #[validate(required)]
    internal: String,
}

fn main() {}
