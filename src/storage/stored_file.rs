#[derive(
    Clone,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct StoredFile {
    pub disk: String,
    pub path: String,
    pub name: String,
    #[ts(type = "number")]
    pub size: u64,
    pub content_type: Option<String>,
    pub url: Option<String>,
}

#[derive(
    Clone,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct StorageObject {
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
    pub modified_at: crate::support::DateTime,
}
