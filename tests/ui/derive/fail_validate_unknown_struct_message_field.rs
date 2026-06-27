#[derive(foundry::Validate)]
#[validate(messages(emali(required = "Email is required.")))]
struct UnknownStructMessageField {
    #[validate(required)]
    email: String,
}

fn main() {}
