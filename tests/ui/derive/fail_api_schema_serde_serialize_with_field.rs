use serde::Serialize;

fn serialize_email<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value)
}

#[derive(Serialize, ts_rs::TS, foundry::ApiSchema)]
struct SerializeWithApiSchema {
    #[serde(serialize_with = "serialize_email")]
    email: String,
}

fn main() {}
