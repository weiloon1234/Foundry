use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<User>,
    email: String,
}

fn main() {
    let _ = User::update().set_null(User::EMAIL);
}
