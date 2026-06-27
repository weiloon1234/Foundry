#[derive(foundry::Validate)]
#[validate(messages(email(required = "  ")))]
struct BlankStructMessage {
    #[validate(required)]
    email: String,
}

fn main() {}
