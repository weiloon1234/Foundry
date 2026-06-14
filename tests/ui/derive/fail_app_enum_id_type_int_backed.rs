#[derive(Clone, Copy, Debug, PartialEq, Eq, foundry::AppEnum)]
#[foundry(id_type = foundry::PermissionId)]
enum Status {
    Draft = 1,
    Published = 2,
}

fn main() {}
