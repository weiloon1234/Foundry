#![allow(unused_imports)]

use foundry::prelude::*;

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct UserRow {
    email: String,
    #[foundry(alias = "email")]
    secondary_email: String,
}

fn main() {}
