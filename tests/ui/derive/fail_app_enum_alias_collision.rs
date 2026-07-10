#[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
enum Status {
    #[foundry(aliases = ["legacy"])]
    Active,
    #[foundry(key = "legacy")]
    Archived,
}

fn main() {}
