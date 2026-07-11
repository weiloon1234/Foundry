use std::path::Path;
use std::sync::Arc;

use tokio::io::AsyncRead;

use crate::foundation::{Error, Result};
use crate::support::DateTime;

use super::adapter::{StorageAdapter, StorageReadStream, StorageVisibility, StorageWriteStream};
use super::callback;
use super::path::{normalize_path, normalize_prefix};
use super::stored_file::{StorageObject, StoredFile};

#[derive(Clone)]
pub struct StorageDisk {
    name: String,
    visibility: StorageVisibility,
    adapter: Arc<dyn StorageAdapter>,
}

impl std::fmt::Debug for StorageDisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageDisk")
            .field("name", &self.name)
            .field("visibility", &self.visibility)
            .finish()
    }
}

impl StorageDisk {
    pub(crate) fn new(
        name: String,
        visibility: StorageVisibility,
        adapter: Arc<dyn StorageAdapter>,
    ) -> Self {
        Self {
            name,
            visibility,
            adapter,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn visibility(&self) -> StorageVisibility {
        self.visibility
    }

    fn finish_put(&self, mut file: StoredFile) -> StoredFile {
        file.disk = self.name.clone();
        if self.visibility == StorageVisibility::Private {
            file.url = None;
        }
        file
    }

    pub async fn put(&self, path: &str, contents: impl AsRef<[u8]>) -> Result<StoredFile> {
        let path = normalize_path(path)?;
        let bytes = contents.as_ref();
        let file = callback::run_storage_operation(&self.name, "put", || {
            self.adapter.put_bytes(&path, bytes, None, self.visibility)
        })
        .await?;
        Ok(self.finish_put(file))
    }

    pub async fn put_bytes(&self, path: &str, bytes: impl AsRef<[u8]>) -> Result<StoredFile> {
        let path = normalize_path(path)?;
        let bytes = bytes.as_ref();
        let file = callback::run_storage_operation(&self.name, "put_bytes", || {
            self.adapter.put_bytes(&path, bytes, None, self.visibility)
        })
        .await?;
        Ok(self.finish_put(file))
    }

    pub async fn put_file(
        &self,
        path: &str,
        temp_path: &Path,
        content_type: Option<&str>,
    ) -> Result<StoredFile> {
        let path = normalize_path(path)?;
        let file = callback::run_storage_operation(&self.name, "put_file", || {
            self.adapter
                .put_file(&path, temp_path, content_type, self.visibility)
        })
        .await?;
        Ok(self.finish_put(file))
    }

    pub async fn put_stream<R>(
        &self,
        path: &str,
        stream: R,
        content_type: Option<&str>,
    ) -> Result<StoredFile>
    where
        R: AsyncRead + Send + 'static,
    {
        let path = normalize_path(path)?;
        let stream: StorageWriteStream = Box::pin(stream);
        let file = callback::run_storage_operation(&self.name, "put_stream", || {
            self.adapter
                .put_stream(&path, stream, content_type, self.visibility)
        })
        .await?;
        Ok(self.finish_put(file))
    }

    pub async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let path = normalize_path(path)?;
        callback::run_storage_operation(&self.name, "get", || self.adapter.get(&path)).await
    }

    pub async fn get_stream(&self, path: &str) -> Result<StorageReadStream> {
        let path = normalize_path(path)?;
        callback::run_storage_operation(&self.name, "get_stream", || self.adapter.get_stream(&path))
            .await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let path = normalize_path(path)?;
        callback::run_storage_operation(&self.name, "delete", || self.adapter.delete(&path)).await
    }

    pub async fn exists(&self, path: &str) -> Result<bool> {
        let path = normalize_path(path)?;
        callback::run_storage_operation(&self.name, "exists", || self.adapter.exists(&path)).await
    }

    pub async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        callback::run_storage_operation(&self.name, "copy", || self.adapter.copy(&from, &to)).await
    }

    pub async fn move_to(&self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        callback::run_storage_operation(&self.name, "move_to", || self.adapter.move_to(&from, &to))
            .await
    }

    pub async fn url(&self, path: &str) -> Result<String> {
        let path = normalize_path(path)?;
        if self.visibility == StorageVisibility::Private {
            return Err(Error::message(format!(
                "storage disk `{}` is private and does not expose stable public URLs",
                self.name
            )));
        }
        callback::run_storage_operation(&self.name, "url", || self.adapter.url(&path)).await
    }

    pub async fn temporary_url(&self, path: &str, expires_at: DateTime) -> Result<String> {
        let path = normalize_path(path)?;
        callback::run_storage_operation(&self.name, "temporary_url", || {
            self.adapter.temporary_url(&path, expires_at)
        })
        .await
    }

    pub async fn list_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<StorageObject>> {
        let prefix = normalize_prefix(prefix)?;
        callback::run_storage_operation(&self.name, "list_prefix", || {
            self.adapter.list_prefix(&prefix, limit)
        })
        .await
    }

    pub async fn list_prefix_after(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StorageObject>> {
        let prefix = normalize_prefix(prefix)?;
        let after = after.map(normalize_path).transpose()?;
        if after
            .as_deref()
            .is_some_and(|cursor| !cursor.starts_with(&prefix))
        {
            return Err(Error::message(format!(
                "storage list cursor must be inside prefix `{prefix}`"
            )));
        }
        callback::run_storage_operation(&self.name, "list_prefix_after", || {
            self.adapter
                .list_prefix_after(&prefix, after.as_deref(), limit)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use futures_util::StreamExt as _;

    use super::*;
    use crate::foundation::Error;

    struct PanickingAdapter;

    #[async_trait]
    impl StorageAdapter for PanickingAdapter {
        async fn put_bytes(
            &self,
            _path: &str,
            _bytes: &[u8],
            _content_type: Option<&str>,
            _visibility: StorageVisibility,
        ) -> Result<StoredFile> {
            panic!("put bytes exploded")
        }

        async fn put_file(
            &self,
            _path: &str,
            _temp_path: &Path,
            _content_type: Option<&str>,
            _visibility: StorageVisibility,
        ) -> Result<StoredFile> {
            panic!("put file exploded")
        }

        async fn get(&self, _path: &str) -> Result<Vec<u8>> {
            panic!("get exploded")
        }

        async fn delete(&self, _path: &str) -> Result<()> {
            panic!("delete exploded")
        }

        async fn exists(&self, _path: &str) -> Result<bool> {
            panic!("exists exploded")
        }

        async fn copy(&self, _from: &str, _to: &str) -> Result<()> {
            panic!("copy exploded")
        }

        async fn move_to(&self, _from: &str, _to: &str) -> Result<()> {
            panic!("move exploded")
        }

        async fn url(&self, _path: &str) -> Result<String> {
            panic!("url exploded")
        }

        async fn temporary_url(&self, _path: &str, _expires_at: DateTime) -> Result<String> {
            panic!("temporary url exploded")
        }

        async fn list_prefix(&self, _prefix: &str, _limit: usize) -> Result<Vec<StorageObject>> {
            panic!("list prefix exploded")
        }
    }

    struct ErrorAdapter;

    #[async_trait]
    impl StorageAdapter for ErrorAdapter {
        async fn put_bytes(
            &self,
            _path: &str,
            _bytes: &[u8],
            _content_type: Option<&str>,
            _visibility: StorageVisibility,
        ) -> Result<StoredFile> {
            Err(Error::message("put bytes failed"))
        }

        async fn put_file(
            &self,
            _path: &str,
            _temp_path: &Path,
            _content_type: Option<&str>,
            _visibility: StorageVisibility,
        ) -> Result<StoredFile> {
            Err(Error::message("put file failed"))
        }

        async fn get(&self, _path: &str) -> Result<Vec<u8>> {
            Err(Error::message("get failed"))
        }

        async fn delete(&self, _path: &str) -> Result<()> {
            Err(Error::message("delete failed"))
        }

        async fn exists(&self, _path: &str) -> Result<bool> {
            Err(Error::message("exists failed"))
        }

        async fn copy(&self, _from: &str, _to: &str) -> Result<()> {
            Err(Error::message("copy failed"))
        }

        async fn move_to(&self, _from: &str, _to: &str) -> Result<()> {
            Err(Error::message("move failed"))
        }

        async fn url(&self, _path: &str) -> Result<String> {
            Err(Error::message("url failed"))
        }

        async fn temporary_url(&self, _path: &str, _expires_at: DateTime) -> Result<String> {
            Err(Error::message("temporary url failed"))
        }
    }

    #[derive(Default)]
    struct BufferedDefaultAdapter {
        bytes: Mutex<Vec<u8>>,
    }

    #[async_trait]
    impl StorageAdapter for BufferedDefaultAdapter {
        async fn put_bytes(
            &self,
            path: &str,
            bytes: &[u8],
            content_type: Option<&str>,
            _visibility: StorageVisibility,
        ) -> Result<StoredFile> {
            *self.bytes.lock().unwrap() = bytes.to_vec();
            Ok(StoredFile {
                disk: String::new(),
                path: path.to_string(),
                name: path.to_string(),
                size: bytes.len() as u64,
                content_type: content_type.map(ToOwned::to_owned),
                url: Some("https://should-be-hidden.example/file".to_string()),
            })
        }

        async fn put_file(
            &self,
            _path: &str,
            _temp_path: &Path,
            _content_type: Option<&str>,
            _visibility: StorageVisibility,
        ) -> Result<StoredFile> {
            Err(Error::message("not used"))
        }

        async fn get(&self, _path: &str) -> Result<Vec<u8>> {
            Ok(self.bytes.lock().unwrap().clone())
        }

        async fn delete(&self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn exists(&self, _path: &str) -> Result<bool> {
            Ok(true)
        }

        async fn copy(&self, _from: &str, _to: &str) -> Result<()> {
            Ok(())
        }

        async fn move_to(&self, _from: &str, _to: &str) -> Result<()> {
            Ok(())
        }

        async fn url(&self, path: &str) -> Result<String> {
            Ok(format!("https://example.com/{path}"))
        }

        async fn temporary_url(&self, _path: &str, _expires_at: DateTime) -> Result<String> {
            Err(Error::message("not used"))
        }
    }

    fn disk(adapter: impl StorageAdapter) -> StorageDisk {
        StorageDisk::new(
            "panic".to_string(),
            StorageVisibility::Private,
            Arc::new(adapter),
        )
    }

    fn assert_storage_panic(error: Error, operation: &str, panic: &str) {
        let message = error.to_string();
        assert!(
            message.contains(&format!(
                "storage disk `panic` {operation} panicked: {panic}"
            )),
            "{message}"
        );
    }

    #[tokio::test]
    async fn adapter_operation_panics_become_errors() {
        let disk = disk(PanickingAdapter);
        let temp_path = Path::new("unused-upload.bin");
        let expires_at = DateTime::now();

        assert_storage_panic(
            disk.put("file.txt", b"hello").await.unwrap_err(),
            "put",
            "put bytes exploded",
        );
        assert_storage_panic(
            disk.put_bytes("file.txt", b"hello").await.unwrap_err(),
            "put_bytes",
            "put bytes exploded",
        );
        assert_storage_panic(
            disk.put_file("file.txt", temp_path, Some("text/plain"))
                .await
                .unwrap_err(),
            "put_file",
            "put file exploded",
        );
        assert_storage_panic(
            disk.put_stream(
                "file.txt",
                std::io::Cursor::new(b"hello".to_vec()),
                Some("text/plain"),
            )
            .await
            .unwrap_err(),
            "put_stream",
            "put bytes exploded",
        );
        assert_storage_panic(
            disk.get("file.txt").await.unwrap_err(),
            "get",
            "get exploded",
        );
        assert_storage_panic(
            disk.get_stream("file.txt")
                .await
                .err()
                .expect("stream open should fail"),
            "get_stream",
            "get exploded",
        );
        assert_storage_panic(
            disk.delete("file.txt").await.unwrap_err(),
            "delete",
            "delete exploded",
        );
        assert_storage_panic(
            disk.exists("file.txt").await.unwrap_err(),
            "exists",
            "exists exploded",
        );
        assert_storage_panic(
            disk.copy("from.txt", "to.txt").await.unwrap_err(),
            "copy",
            "copy exploded",
        );
        assert_storage_panic(
            disk.move_to("from.txt", "to.txt").await.unwrap_err(),
            "move_to",
            "move exploded",
        );
        let public_disk = StorageDisk::new(
            "panic".to_string(),
            StorageVisibility::Public,
            Arc::new(PanickingAdapter),
        );
        assert_storage_panic(
            public_disk.url("file.txt").await.unwrap_err(),
            "url",
            "url exploded",
        );
        assert_storage_panic(
            disk.temporary_url("file.txt", expires_at)
                .await
                .unwrap_err(),
            "temporary_url",
            "temporary url exploded",
        );
        assert_storage_panic(
            disk.list_prefix("files/", 10).await.unwrap_err(),
            "list_prefix",
            "list prefix exploded",
        );
    }

    #[tokio::test]
    async fn adapter_operation_errors_remain_unchanged() {
        let disk = disk(ErrorAdapter);

        let error = disk.get("file.txt").await.unwrap_err();

        assert_eq!(error.to_string(), "get failed");
    }

    #[tokio::test]
    async fn default_list_prefix_is_unsupported_for_custom_adapters() {
        let disk = disk(ErrorAdapter);

        let error = disk.list_prefix("files/", 10).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("storage adapter does not support prefix listing"));
    }

    #[tokio::test]
    async fn custom_adapters_get_backward_compatible_streaming_defaults() {
        let disk = StorageDisk::new(
            "memory".to_string(),
            StorageVisibility::Private,
            Arc::new(BufferedDefaultAdapter::default()),
        );

        let stored = disk
            .put_stream(
                "files/report.txt",
                std::io::Cursor::new(b"streamed through defaults".to_vec()),
                Some("text/plain"),
            )
            .await
            .unwrap();
        let mut stream = disk.get_stream("files/report.txt").await.unwrap();
        let chunk = stream.next().await.unwrap().unwrap();

        assert_eq!(stored.disk, "memory");
        assert!(stored.url.is_none());
        assert_eq!(chunk, b"streamed through defaults");
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn private_disk_rejects_stable_urls_before_calling_the_adapter() {
        let disk = disk(PanickingAdapter);

        let error = disk.url("file.txt").await.unwrap_err();

        assert!(error.to_string().contains("disk `panic` is private"));
    }

    #[tokio::test]
    async fn invalid_paths_are_rejected_before_adapter_calls() {
        let disk = disk(PanickingAdapter);

        let error = disk.put_bytes("../secret.txt", b"hello").await.unwrap_err();
        assert!(error.to_string().contains("invalid storage path"));

        let error = disk.copy("file.txt", "/tmp/outside.txt").await.unwrap_err();
        assert!(error.to_string().contains("invalid storage path"));

        let error = disk.list_prefix("../", 10).await.unwrap_err();
        assert!(error.to_string().contains("invalid storage prefix"));
    }
}
