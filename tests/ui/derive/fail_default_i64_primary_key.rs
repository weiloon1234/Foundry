#[derive(foundry::Model)]
#[foundry(table = "users")]
struct User {
    id: i64,
    email: String,
}

fn main() {}
