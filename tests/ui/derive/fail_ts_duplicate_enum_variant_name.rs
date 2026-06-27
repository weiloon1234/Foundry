use serde::Serialize;

#[derive(Serialize, ts_rs::TS, foundry::TS)]
#[serde(rename_all = "snake_case")]
enum DuplicateEnumVariantTs {
    PendingReview,
    #[serde(rename = "pending_review")]
    Pending,
}

fn main() {}
