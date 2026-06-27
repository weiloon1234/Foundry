#[derive(ts_rs::TS, foundry::Validate)]
struct TsRenameValidate {
    #[ts(rename = "emailAddress")]
    email: String,
}

fn main() {}
