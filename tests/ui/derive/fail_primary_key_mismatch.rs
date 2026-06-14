#![allow(unused_imports)]

use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users", primary_key = "user_id")]
struct User {
    id: i64,
    email: String,
}

fn main() {}
