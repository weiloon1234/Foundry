use foundry::FoundryId;

#[derive(FoundryId)]
#[foundry(id = foundry::GuardId)]
enum Guard {
    #[foundry(value = "api")]
    Api,
    #[foundry(value = "api")]
    Admin,
}

fn main() {}
