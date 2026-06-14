use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users", primary_key_strategy = "manual")]
struct User {
    id: i64,
    #[foundry(write_mutator = "hash_password")]
    password: String,
}

impl User {
    async fn hash_password(_ctx: &ModelHookContext<'_>, value: String) -> Result<bool> {
        Ok(!value.is_empty())
    }
}

fn main() {}
