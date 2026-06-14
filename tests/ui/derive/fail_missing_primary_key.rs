#![allow(unused_imports)]

use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users")]
struct User {
    sale_id: i64,
}

fn main() {}
