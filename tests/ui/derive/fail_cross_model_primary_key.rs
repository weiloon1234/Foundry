use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<User>,
}

#[derive(foundry::Model)]
#[foundry(table = "orders")]
struct Order {
    id: ModelId<Order>,
}

async fn invalid_lookup(database: &DatabaseManager) {
    let _ = User::query()
        .find(database, ModelId::<Order>::generate())
        .await;
}

fn main() {}
