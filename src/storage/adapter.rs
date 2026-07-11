use std::path::Path;
use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;
use tokio::io::{AsyncRead, AsyncReadExt as _};

use crate::foundation::{Error, Result};
use crate::support::DateTime;

use super::stored_file::{StorageObject, StoredFile};

/// An owned async byte source accepted by streaming storage writes.
pub type StorageWriteStream = Pin<Box<dyn AsyncRead + Send + 'static>>;

/// A fallible stream of bounded byte chunks returned by streaming storage reads.
pub type StorageReadStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + Send + 'static>>;

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

    /// Stream an object into this adapter.
    ///
    /// The default keeps existing custom adapters source-compatible by buffering
    /// the input and delegating to [`StorageAdapter::put_bytes`]. Adapters that
    /// support native streaming should override this method.
    async fn put_stream(
        &self,
        path: &str,
        mut stream: StorageWriteStream,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.map_err(Error::other)?;
        self.put_bytes(path, &bytes, content_type, visibility).await
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>>;

    /// Stream an object from this adapter in bounded chunks.
    ///
    /// The default keeps existing custom adapters source-compatible by yielding
    /// the value returned by [`StorageAdapter::get`] as a single chunk. Adapters
    /// that support native streaming should override this method.
    async fn get_stream(&self, path: &str) -> Result<StorageReadStream> {
        let bytes = self.get(path).await?;
        Ok(Box::pin(futures_util::stream::once(
            async move { Ok(bytes) },
        )))
    }

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

    /// Return a lexicographically ordered page of objects after an exclusive path cursor.
    ///
    /// Existing custom adapters remain source compatible. They can serve the first page through
    /// `list_prefix`, but must override this method to support complete paginated scans.
    async fn list_prefix_after(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StorageObject>> {
        if after.is_some() {
            return Err(crate::foundation::Error::message(
                "storage adapter does not support prefix pagination",
            ));
        }
        self.list_prefix(prefix, limit).await
    }
}
