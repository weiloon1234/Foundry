use foundry::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct StatusLabel(String);

impl FromDbValue for StatusLabel {
    fn from_db_value(value: &DbValue) -> Result<Self> {
        Ok(Self(String::from_db_value(value)?))
    }
}

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct UserRow {
    #[foundry(source = "email")]
    email: String,
    #[foundry(alias = "status_label", source = "status", db_type = "text")]
    status: StatusLabel,
}

fn main() {
    let _ = UserRow::EMAIL;
    let _ = UserRow::STATUS;
    let _ = UserRow::projection_meta();
}
