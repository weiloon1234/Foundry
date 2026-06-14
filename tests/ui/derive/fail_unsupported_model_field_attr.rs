#![allow(unused_imports)]

use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users", primary_key_strategy = "manual")]
struct User {
    id: i64,
    #[foundry(unique)]
    email: String,
}

fn main() {}
