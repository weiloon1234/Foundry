#[derive(ts_rs::TS, foundry::ApiSchema)]
struct FlattenedChild {
    value: String,
}

#[derive(ts_rs::TS, foundry::ApiSchema)]
#[serde(deny_unknown_fields)]
struct StrictFlatten {
    #[serde(flatten)]
    child: FlattenedChild,
}

fn main() {}
