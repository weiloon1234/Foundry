#[derive(foundry::Validate)]
#[validate(attributes(emali = "email address"))]
struct UnknownStructAttributeField {
    #[validate(required)]
    email: String,
}

fn main() {}
