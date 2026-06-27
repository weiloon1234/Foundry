#[derive(foundry::Validate)]
struct DuplicateRuleMessage {
    #[validate(required(message = "First", message = "Second"))]
    email: String,
}

fn main() {}
