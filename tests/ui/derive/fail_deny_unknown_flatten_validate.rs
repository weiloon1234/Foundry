#[derive(foundry::Validate)]
struct FlattenedChild {
    value: String,
}

#[derive(foundry::Validate)]
#[serde(deny_unknown_fields)]
struct StrictFlatten {
    #[serde(flatten)]
    child: FlattenedChild,
}

fn main() {}
