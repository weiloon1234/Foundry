use foundry::FoundryId;

#[derive(FoundryId)]
#[foundry(id = foundry::GuardId, prefix = "admin")]
enum Guard {
    #[foundry(value = "api")]
    Api,
}

fn main() {}
