use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<User>,
    active: bool,
}

fn main() {
    let _ = User::ACTIVE.ilike("true");
}
