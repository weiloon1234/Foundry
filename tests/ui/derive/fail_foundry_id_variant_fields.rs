use foundry::FoundryId;

#[derive(FoundryId)]
#[foundry(id = foundry::GuardId, rename_all = "snake_case")]
enum Guard {
    Api(String),
}

fn main() {}
