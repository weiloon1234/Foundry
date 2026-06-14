#![allow(unused_imports)]

use foundry::prelude::*;

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct UserRow {
    tags: Loaded<Vec<String>>,
}

fn main() {}
