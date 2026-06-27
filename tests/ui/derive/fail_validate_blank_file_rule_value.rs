#[derive(foundry::Validate)]
struct BlankFileRuleValue {
    #[validate(allowed_extensions("jpg", " png "))]
    avatar: foundry::UploadedFile,
}

fn main() {}
