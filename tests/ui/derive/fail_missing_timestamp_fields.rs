use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "posts", timestamps = true)]
struct Post {
    id: ModelId<Post>,
    title: String,
}

fn main() {}
