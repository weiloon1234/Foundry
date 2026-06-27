#[derive(foundry::Validate)]
#[validate(messages(email(min = "Email is too short.")))]
struct UnreachableStructMessageRule {
    #[validate(required)]
    email: String,
}

fn main() {}
