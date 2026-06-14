use foundry::FoundryId;

#[derive(FoundryId)]
enum Guard {
    #[foundry(value = "api")]
    Api,
}

fn main() {}
