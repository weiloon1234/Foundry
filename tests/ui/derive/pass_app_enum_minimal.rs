use foundry::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
enum MinimalStatus {
    Pending,
    Completed,
}

fn main() {
    let _ = MinimalStatus::id();
    let _ = MinimalStatus::options();
    let _ = MinimalStatus::Pending.label_key();
}
