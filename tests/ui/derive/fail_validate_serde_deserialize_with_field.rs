use serde::Deserialize;

fn deserialize_email<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer)
}

#[derive(foundry::Validate)]
struct DeserializeWithValidate {
    #[serde(deserialize_with = "deserialize_email")]
    email: String,
}

fn main() {}
