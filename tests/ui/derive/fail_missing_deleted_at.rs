use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "posts", soft_deletes = true)]
struct Post {
    id: ModelId<Post>,
    title: String,
    created_at: DateTime,
    updated_at: DateTime,
}

fn main() {}
