#[derive(foundry::Validate)]
struct FlattenedChild {
    #[validate(required)]
    value: String,
}

#[derive(foundry::Validate)]
struct FlattenedValidation {
    #[serde(flatten)]
    #[validate(nested)]
    child: FlattenedChild,
}

fn main() {}
