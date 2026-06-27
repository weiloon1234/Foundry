#[derive(foundry::Validate)]
struct SkipDeserializingRule {
    #[serde(skip_deserializing)]
    #[validate(required)]
    server_only: String,
}

fn main() {}
