#[derive(foundry::Validate)]
struct FileRuleExtraArg {
    #[validate(max_file_size(1024, 2048))]
    avatar: foundry::UploadedFile,
}

fn main() {}
