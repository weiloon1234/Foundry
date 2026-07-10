use foundry::prelude::*;
use foundry::Validate;

#[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
enum Status {
    Pending,
    Complete,
}

#[derive(Deserialize, Validate)]
struct TypedValidationRequest {
    #[validate(required, min_numeric(1))]
    count: Option<i64>,
    #[validate(required, each(integer, min_numeric(0)))]
    scores: Vec<i32>,
    #[validate(each(integer, min_numeric(0)))]
    adjustments: Option<Vec<i32>>,
    #[validate(each(app_enum(Status)))]
    statuses: Option<Vec<Status>>,
}

fn main() {}
