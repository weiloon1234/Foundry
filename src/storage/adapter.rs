use std::path::Path;

use async_trait::async_trait;

use crate::foundation::Result;
use crate::support::DateTime;

use super::stored_file::{StorageObject, StoredFile};

/// Application-level access intent for a storage disk.
///
/// This does not require adapters to emit provider object ACLs. For example,
/// S3 public access may be supplied by bucket policy or a CDN because modern AWS
/// buckets and some S3-compatible providers disable object ACLs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageVisibility {
    #[default]
    Private,
    Public,
}

#[async_trait]
pub trait StorageAdapter: Send + Sync + 'static {
    async fn put_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile>;

    async fn put_file(
        &self,
        path: &str,
        temp_path: &Path,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile>;

    async fn get(&self, path: &str) -> Result<Vec<u8>>;
    async fn delete(&self, path: &str) -> Result<()>;
    async fn exists(&self, path: &str) -> Result<bool>;
    async fn copy(&self, from: &str, to: &str) -> Result<()>;
    async fn move_to(&self, from: &str, to: &str) -> Result<()>;
    async fn url(&self, path: &str) -> Result<String>;
    async fn temporary_url(&self, path: &str, expires_at: DateTime) -> Result<String>;

    async fn list_prefix(&self, _prefix: &str, _limit: usize) -> Result<Vec<StorageObject>> {
        Err(crate::foundation::Error::message(
            "storage adapter does not support prefix listing",
        ))
    }
}
