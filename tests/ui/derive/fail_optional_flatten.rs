use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct FlattenedChild {
    value: String,
}

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct OptionalFlatten {
    #[serde(flatten)]
    child: Option<FlattenedChild>,
}

fn main() {}
