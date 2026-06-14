#![allow(unused_imports)]

use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users", primary_key_strategy = "manual")]
struct User {
    id: i64,
    #[foundry(column = "email")]
    primary_email: String,
    email: String,
}

fn main() {}
