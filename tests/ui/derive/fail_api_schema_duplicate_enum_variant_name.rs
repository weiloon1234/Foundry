use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
#[serde(rename_all = "snake_case")]
enum DuplicateEnumVariantApiSchema {
    PendingReview,
    #[serde(rename = "pending_review")]
    Pending,
}

fn main() {}
