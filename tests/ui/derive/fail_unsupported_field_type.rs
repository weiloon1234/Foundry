#![allow(unused_imports)]

use foundry::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct CustomType;

#[derive(foundry::Model)]
#[foundry(table = "users", primary_key_strategy = "manual")]
struct User {
    id: i64,
    custom: CustomType,
}

fn main() {}
