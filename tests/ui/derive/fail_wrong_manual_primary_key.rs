use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users", primary_key_strategy = "manual")]
struct User {
    id: i64,
}

async fn invalid_lookup(database: &DatabaseManager) {
    let _ = User::query().find(database, "not-an-integer").await;
}

fn main() {}
