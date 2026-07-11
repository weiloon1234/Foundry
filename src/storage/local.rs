use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::foundation::{Error, Result};
use crate::support::DateTime;

use super::adapter::{StorageAdapter, StorageReadStream, StorageVisibility, StorageWriteStream};
use super::config::ResolvedLocalConfig;
use super::path::{join_url_prefix, normalize_path, normalize_prefix};
use super::stored_file::{StorageObject, StoredFile};

pub struct LocalStorageAdapter {
    root: PathBuf,
    url: Option<String>,
    visibility: StorageVisibility,
}

impl LocalStorageAdapter {
    pub fn from_config(config: &ResolvedLocalConfig) -> Result<Self> {
        Ok(Self {
            root: PathBuf::from(&config.root),
            url: config.url.clone(),
            visibility: config.visibility,
        })
    }

    fn full_path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    fn file_name(path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string()
    }

    fn public_url(&self, path: &str) -> Option<String> {
        self.url
            .as_deref()
            .and_then(|prefix| join_url_prefix(prefix, path))
    }

    fn stored_file(
        &self,
        path: String,
        size: u64,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> StoredFile {
        StoredFile {
            disk: String::new(),
            name: Self::file_name(&path),
            size,
            content_type: content_type.map(ToOwned::to_owned),
            url: if visibility == StorageVisibility::Public {
                self.public_url(&path)
            } else {
                None
            },
            path,
        }
    }

    fn object_path(&self, full_path: &Path) -> Result<String> {
        let relative = full_path.strip_prefix(&self.root).map_err(Error::other)?;
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }

    async fn prepare_write_path(&self, path: &str) -> Result<(String, PathBuf)> {
        let path = normalize_path(path)?;
        let full = self.full_path(&path);

        self.reject_symlink_components(&path, false).await?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(Error::other)?;
        }
        self.reject_symlink_components(&path, true).await?;

        Ok((path, full))
    }

    async fn open_unique_temp_file(&self, full: &Path) -> Result<(PathBuf, tokio::fs::File)> {
        let parent = full
            .parent()
            .ok_or_else(|| Error::message("storage path has no parent directory"))?;
        let file_name = full
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");

        for _ in 0..16 {
            let temp_path =
                parent.join(format!(".foundry-tmp-{}-{file_name}", uuid::Uuid::now_v7()));
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .await
            {
                Ok(file) => return Ok((temp_path, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(Error::other(error)),
            }
        }

        Err(Error::message(
            "failed to create a unique storage temp file",
        ))
    }

    async fn write_bytes_atomically(&self, full: &Path, bytes: &[u8]) -> Result<u64> {
        let (temp_path, mut file) = self.open_unique_temp_file(full).await?;
        let result = async {
            file.write_all(bytes).await.map_err(Error::other)?;
            file.flush().await.map_err(Error::other)?;
            drop(file);
            tokio::fs::rename(&temp_path, full)
                .await
                .map_err(Error::other)?;
            Ok(bytes.len() as u64)
        }
        .await;

        if result.is_err() {
            let _ = tokio::fs::remove_file(&temp_path).await;
        }

        result
    }

    async fn copy_file_atomically(&self, source: &Path, full: &Path) -> Result<u64> {
        let (temp_path, mut file) = self.open_unique_temp_file(full).await?;
        let result = async {
            let mut source = tokio::fs::File::open(source).await.map_err(Error::other)?;
            let bytes = tokio::io::copy(&mut source, &mut file)
                .await
                .map_err(Error::other)?;
            file.flush().await.map_err(Error::other)?;
            drop(file);
            tokio::fs::rename(&temp_path, full)
                .await
                .map_err(Error::other)?;
            Ok(bytes)
        }
        .await;

        if result.is_err() {
            let _ = tokio::fs::remove_file(&temp_path).await;
        }

        result
    }

    async fn write_stream_atomically(
        &self,
        full: &Path,
        mut stream: StorageWriteStream,
    ) -> Result<u64> {
        let (temp_path, mut file) = self.open_unique_temp_file(full).await?;
        let result = async {
            let bytes = tokio::io::copy(&mut stream, &mut file)
                .await
                .map_err(Error::other)?;
            file.flush().await.map_err(Error::other)?;
            drop(file);
            tokio::fs::rename(&temp_path, full)
                .await
                .map_err(Error::other)?;
            Ok(bytes)
        }
        .await;

        if result.is_err() {
            let _ = tokio::fs::remove_file(&temp_path).await;
        }

        result
    }

    async fn resolve_read_path(&self, path: &str) -> Result<(String, PathBuf)> {
        let path = normalize_path(path)?;
        self.reject_symlink_components(&path, true).await?;
        let full = self.full_path(&path);
        Ok((path, full))
    }

    async fn resolve_prefix_path(&self, prefix: &str) -> Result<(String, PathBuf)> {
        let prefix = normalize_prefix(prefix)?;
        self.reject_symlink_components(prefix.trim_end_matches('/'), true)
            .await?;
        let full = self.full_path(&prefix);
        Ok((prefix, full))
    }

    async fn reject_symlink_components(&self, path: &str, include_leaf: bool) -> Result<()> {
        let segments: Vec<&str> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        let mut current = self.root.clone();
        for (index, segment) in segments.iter().enumerate() {
            let is_leaf = index + 1 == segments.len();
            if is_leaf && !include_leaf {
                break;
            }

            current.push(segment);
            match tokio::fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(Error::message(format!(
                        "storage path `{path}` resolves through a symlink"
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::other(error)),
            }
        }

        Ok(())
    }
}

#[async_trait]
impl StorageAdapter for LocalStorageAdapter {
    async fn put_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let (path, full) = self.prepare_write_path(path).await?;

        self.write_bytes_atomically(&full, bytes).await?;

        Ok(self.stored_file(path, bytes.len() as u64, content_type, visibility))
    }

    async fn put_file(
        &self,
        path: &str,
        temp_path: &Path,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let (path, full) = self.prepare_write_path(path).await?;

        let size = self.copy_file_atomically(temp_path, &full).await?;

        Ok(self.stored_file(path, size, content_type, visibility))
    }

    async fn put_stream(
        &self,
        path: &str,
        stream: StorageWriteStream,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let (path, full) = self.prepare_write_path(path).await?;
        let size = self.write_stream_atomically(&full, stream).await?;

        Ok(self.stored_file(path, size, content_type, visibility))
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let (path, full) = self.resolve_read_path(path).await?;
        tokio::fs::read(&full)
            .await
            .map_err(|e| Error::message(format!("Failed to read file '{path}': {e}")))
    }

    async fn get_stream(&self, path: &str) -> Result<StorageReadStream> {
        const READ_CHUNK_SIZE: usize = 64 * 1024;

        let (path, full) = self.resolve_read_path(path).await?;
        let file = tokio::fs::File::open(&full)
            .await
            .map_err(|error| Error::message(format!("Failed to read file '{path}': {error}")))?;
        let stream =
            futures_util::stream::try_unfold((file, path), |(mut file, path)| async move {
                let mut chunk = vec![0; READ_CHUNK_SIZE];
                let read = file.read(&mut chunk).await.map_err(|error| {
                    Error::message(format!("Failed to stream file '{path}': {error}"))
                })?;
                if read == 0 {
                    return Ok(None);
                }
                chunk.truncate(read);
                Ok(Some((chunk, (file, path))))
            });

        Ok(Box::pin(stream))
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let (path, full) = self.resolve_read_path(path).await?;
        tokio::fs::remove_file(&full)
            .await
            .map_err(|e| Error::message(format!("Failed to delete file '{path}': {e}")))
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        let (_path, full) = self.resolve_read_path(path).await?;
        match tokio::fs::metadata(&full).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::other(e)),
        }
    }

    async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let (from, src) = self.resolve_read_path(from).await?;
        let (to, dst) = self.prepare_write_path(to).await?;

        self.copy_file_atomically(&src, &dst)
            .await
            .map_err(|e| Error::message(format!("Failed to copy '{from}' to '{to}': {e}")))?;

        Ok(())
    }

    async fn move_to(&self, from: &str, to: &str) -> Result<()> {
        let (from, src) = self.resolve_read_path(from).await?;
        let (to, dst) = self.prepare_write_path(to).await?;

        if let Err(e) = tokio::fs::rename(&src, &dst).await {
            if e.raw_os_error() == Some(18)
                || e.to_string().contains("cross-device")
                || e.to_string().contains("Invalid cross-device link")
            {
                let data = tokio::fs::read(&src).await.map_err(Error::other)?;
                self.write_bytes_atomically(&dst, &data).await?;
                tokio::fs::remove_file(&src).await.map_err(Error::other)?;
            } else {
                return Err(Error::message(format!(
                    "Failed to move '{from}' to '{to}': {e}"
                )));
            }
        }

        Ok(())
    }

    async fn url(&self, path: &str) -> Result<String> {
        let path = normalize_path(path)?;
        if self.visibility == StorageVisibility::Private {
            return Err(Error::message(
                "private storage disks do not expose stable public URLs",
            ));
        }

        self.public_url(&path).ok_or_else(|| {
            Error::message(
                "stable public URL generation is not supported for this local disk (no url configured)",
            )
        })
    }

    async fn temporary_url(&self, path: &str, _expires_at: DateTime) -> Result<String> {
        normalize_path(path)?;
        Err(Error::message(
            "Temporary URLs are not supported for local disk",
        ))
    }

    async fn list_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<StorageObject>> {
        self.list_prefix_after(prefix, None, limit).await
    }

    async fn list_prefix_after(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StorageObject>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

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
        let (_prefix, start) = self.resolve_prefix_path(&prefix).await?;
        if tokio::fs::metadata(&start)
            .await
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(Vec::new());
        }

        let mut pending = vec![start];
        let mut objects = Vec::new();

        while let Some(path) = pending.pop() {
            let metadata = match tokio::fs::symlink_metadata(&path).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(Error::other(error)),
            };

            if metadata.file_type().is_symlink() {
                continue;
            }

            if metadata.is_file() {
                let modified_at = metadata.modified().map_err(Error::other)?;
                let modified_at: chrono::DateTime<chrono::Utc> = modified_at.into();
                objects.push(StorageObject {
                    path: self.object_path(&path)?,
                    size: metadata.len(),
                    modified_at: DateTime::from_chrono(modified_at),
                });
                continue;
            }

            if !metadata.is_dir() {
                continue;
            }

            let mut entries = tokio::fs::read_dir(&path).await.map_err(Error::other)?;
            while let Some(entry) = entries.next_entry().await.map_err(Error::other)? {
                pending.push(entry.path());
            }
        }

        objects.sort_by(|left, right| left.path.cmp(&right.path));
        if let Some(after) = after.as_deref() {
            objects.retain(|object| object.path.as_str() > after);
        }
        objects.truncate(limit);
        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use futures_util::StreamExt as _;
    use tempfile::TempDir;
    use tokio::io::{AsyncRead, ReadBuf};

    use super::*;

    fn make_adapter(dir: &TempDir) -> LocalStorageAdapter {
        LocalStorageAdapter {
            root: dir.path().to_path_buf(),
            url: None,
            visibility: StorageVisibility::Private,
        }
    }

    fn make_adapter_with_url(dir: &TempDir, url: &str) -> LocalStorageAdapter {
        LocalStorageAdapter {
            root: dir.path().to_path_buf(),
            url: Some(url.to_string()),
            visibility: StorageVisibility::Public,
        }
    }

    struct FailingReader {
        yielded: bool,
    }

    impl AsyncRead for FailingReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.yielded {
                return Poll::Ready(Err(std::io::Error::other("stream failed")));
            }

            self.yielded = true;
            buffer.put_slice(b"partial");
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn put_bytes_and_read_back() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let file = adapter
            .put_bytes(
                "hello.txt",
                b"hello world",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap();

        assert_eq!(file.path, "hello.txt");
        assert_eq!(file.name, "hello.txt");
        assert_eq!(file.size, 11);
        assert!(file.disk.is_empty());

        let data = adapter.get("hello.txt").await.unwrap();
        assert_eq!(data, b"hello world");
    }

    #[tokio::test]
    async fn put_file_and_read_back() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let temp = TempDir::new().unwrap();
        let temp_file_path = temp.path().join("upload.bin");
        {
            let mut f = std::fs::File::create(&temp_file_path).unwrap();
            f.write_all(b"file contents").unwrap();
        }

        let file = adapter
            .put_file(
                "uploads/file.bin",
                &temp_file_path,
                Some("application/octet-stream"),
                StorageVisibility::Public,
            )
            .await
            .unwrap();

        assert_eq!(file.path, "uploads/file.bin");
        assert_eq!(file.name, "file.bin");
        assert_eq!(file.size, 13);
        assert_eq!(
            file.content_type.as_deref(),
            Some("application/octet-stream")
        );

        let data = adapter.get("uploads/file.bin").await.unwrap();
        assert_eq!(data, b"file contents");
    }

    #[tokio::test]
    async fn streams_large_files_in_bounded_chunks_with_a_timeout() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);
        let contents = vec![42; 160 * 1024];

        let stored = tokio::time::timeout(
            Duration::from_secs(2),
            adapter.put_stream(
                "streams/large.bin",
                Box::pin(std::io::Cursor::new(contents.clone())),
                Some("application/octet-stream"),
                StorageVisibility::Private,
            ),
        )
        .await
        .expect("streaming write timed out")
        .unwrap();
        assert_eq!(stored.size, contents.len() as u64);

        let mut stream = tokio::time::timeout(
            Duration::from_secs(2),
            adapter.get_stream("streams/large.bin"),
        )
        .await
        .expect("opening streaming read timed out")
        .unwrap();
        let mut read_back = Vec::new();
        let mut chunks = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.len() <= 64 * 1024);
            chunks += 1;
            read_back.extend_from_slice(&chunk);
        }

        assert!(chunks > 1);
        assert_eq!(read_back, contents);
    }

    #[tokio::test]
    async fn failed_streaming_write_removes_temp_and_target_files() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let error = adapter
            .put_stream(
                "streams/failure.bin",
                Box::pin(FailingReader { yielded: false }),
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("stream failed"));
        assert!(!dir.path().join("streams/failure.bin").exists());
        let files = std::fs::read_dir(dir.path().join("streams"))
            .unwrap()
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap();
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn public_writes_include_configured_url_and_private_writes_do_not() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter_with_url(&dir, "/storage/");

        let public = adapter
            .put_bytes(
                "images/public.jpg",
                b"public",
                None,
                StorageVisibility::Public,
            )
            .await
            .unwrap();
        let private = adapter
            .put_bytes(
                "images/private.jpg",
                b"private",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap();

        assert_eq!(public.url.as_deref(), Some("/storage/images/public.jpg"));
        assert!(private.url.is_none());
    }

    #[tokio::test]
    async fn delete_file() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        adapter
            .put_bytes("to_delete.txt", b"bye", None, StorageVisibility::Private)
            .await
            .unwrap();

        adapter.delete("to_delete.txt").await.unwrap();

        assert!(!adapter.exists("to_delete.txt").await.unwrap());
    }

    #[tokio::test]
    async fn exists_returns_true_for_existing_file() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        adapter
            .put_bytes("exists.txt", b"data", None, StorageVisibility::Private)
            .await
            .unwrap();

        assert!(adapter.exists("exists.txt").await.unwrap());
    }

    #[tokio::test]
    async fn exists_returns_false_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        assert!(!adapter.exists("missing.txt").await.unwrap());
    }

    #[tokio::test]
    async fn copy_file() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        adapter
            .put_bytes("original.txt", b"copy me", None, StorageVisibility::Private)
            .await
            .unwrap();

        adapter.copy("original.txt", "copy.txt").await.unwrap();

        let original = adapter.get("original.txt").await.unwrap();
        let copy = adapter.get("copy.txt").await.unwrap();
        assert_eq!(original, copy);
    }

    #[tokio::test]
    async fn move_file() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        adapter
            .put_bytes("src.txt", b"move me", None, StorageVisibility::Private)
            .await
            .unwrap();

        adapter.move_to("src.txt", "dst.txt").await.unwrap();

        assert!(!adapter.exists("src.txt").await.unwrap());
        let data = adapter.get("dst.txt").await.unwrap();
        assert_eq!(data, b"move me");
    }

    #[tokio::test]
    async fn url_returns_url_when_configured() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter_with_url(&dir, "http://localhost/storage");

        let url = adapter.url("images/photo.jpg").await.unwrap();
        assert_eq!(url, "http://localhost/storage/images/photo.jpg");
    }

    #[tokio::test]
    async fn url_trims_configured_trailing_slash() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter_with_url(&dir, "http://localhost/storage/");

        let url = adapter.url("images/photo.jpg").await.unwrap();
        assert_eq!(url, "http://localhost/storage/images/photo.jpg");
    }

    #[tokio::test]
    async fn url_returns_error_when_not_configured() {
        let dir = TempDir::new().unwrap();
        let mut adapter = make_adapter(&dir);
        adapter.visibility = StorageVisibility::Public;

        let result = adapter.url("test.txt").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("URL"));
    }

    #[tokio::test]
    async fn private_url_is_unsupported_even_when_a_base_url_is_configured() {
        let dir = TempDir::new().unwrap();
        let mut adapter = make_adapter_with_url(&dir, "/storage");
        adapter.visibility = StorageVisibility::Private;

        let error = adapter.url("test.txt").await.unwrap_err();

        assert!(error.to_string().contains("private storage disks"));
    }

    #[tokio::test]
    async fn temporary_url_always_errors() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let result = adapter.temporary_url("test.txt", DateTime::now()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Temporary"));
    }

    #[tokio::test]
    async fn list_prefix_returns_bounded_sorted_file_metadata() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        adapter
            .put_bytes(
                "attachments/b.txt",
                b"bbb",
                Some("text/plain"),
                StorageVisibility::Private,
            )
            .await
            .unwrap();
        adapter
            .put_bytes(
                "attachments/nested/a.txt",
                b"a",
                Some("text/plain"),
                StorageVisibility::Private,
            )
            .await
            .unwrap();
        adapter
            .put_bytes("other.txt", b"nope", None, StorageVisibility::Private)
            .await
            .unwrap();

        let objects = adapter.list_prefix("attachments/", 10).await.unwrap();

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].path, "attachments/b.txt");
        assert_eq!(objects[0].size, 3);
        assert_eq!(objects[1].path, "attachments/nested/a.txt");

        let limited = adapter.list_prefix("attachments/", 1).await.unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].path, "attachments/b.txt");

        let next = adapter
            .list_prefix_after("attachments/", Some("attachments/b.txt"), 1)
            .await
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].path, "attachments/nested/a.txt");
        assert!(adapter
            .list_prefix_after("attachments/", Some("attachments/nested/a.txt"), 1)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn list_prefix_returns_empty_for_missing_prefix() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let objects = adapter.list_prefix("missing/", 10).await.unwrap();

        assert!(objects.is_empty());
    }

    #[tokio::test]
    async fn rejects_traversal_paths_without_escaping_root() {
        let workspace = TempDir::new().unwrap();
        let root = workspace.path().join("storage");
        std::fs::create_dir_all(&root).unwrap();
        let adapter = LocalStorageAdapter {
            root,
            url: None,
            visibility: StorageVisibility::Private,
        };
        let outside = workspace.path().join("outside.txt");

        let error = adapter
            .put_bytes("../outside.txt", b"nope", None, StorageVisibility::Private)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("invalid storage path"));
        assert!(!outside.exists());
    }

    #[tokio::test]
    async fn rejects_absolute_paths() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let error = adapter.get("/etc/passwd").await.unwrap_err();

        assert!(error.to_string().contains("invalid storage path"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_prefix_skips_symlinked_directories() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"secret").unwrap();

        let adapter = make_adapter(&dir);
        std::fs::create_dir_all(dir.path().join("attachments")).unwrap();
        std::os::unix::fs::symlink(
            outside.path(),
            dir.path().join("attachments").join("outside"),
        )
        .unwrap();
        adapter
            .put_bytes(
                "attachments/inside.txt",
                b"inside",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap();

        let objects = adapter.list_prefix("attachments/", 10).await.unwrap();

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].path, "attachments/inside.txt");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_rejects_symlinked_leaf() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let outside_file = outside.path().join("target.txt");
        std::fs::write(&outside_file, b"original").unwrap();
        let adapter = make_adapter(&dir);
        std::fs::create_dir_all(dir.path().join("attachments")).unwrap();
        std::os::unix::fs::symlink(
            &outside_file,
            dir.path().join("attachments").join("target.txt"),
        )
        .unwrap();

        let error = adapter
            .put_bytes(
                "attachments/target.txt",
                b"changed",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_rejects_symlinked_parent_before_creating_children() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);
        std::os::unix::fs::symlink(outside.path(), dir.path().join("attachments")).unwrap();

        let error = adapter
            .put_bytes(
                "attachments/nested/target.txt",
                b"changed",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert!(!outside.path().join("nested").exists());
    }

    #[tokio::test]
    async fn write_uses_temp_file_then_atomic_rename() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        adapter
            .put_bytes(
                "attachments/file.txt",
                b"data",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap();

        let mut entries = std::fs::read_dir(dir.path().join("attachments")).unwrap();
        let names = entries
            .by_ref()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["file.txt".to_string()]);
        assert_eq!(
            std::fs::read(dir.path().join("attachments/file.txt")).unwrap(),
            b"data"
        );
    }

    #[tokio::test]
    async fn parent_directories_are_auto_created() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        adapter
            .put_bytes(
                "a/b/c/deep.txt",
                b"nested",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap();

        let data = adapter.get("a/b/c/deep.txt").await.unwrap();
        assert_eq!(data, b"nested");
    }

    #[tokio::test]
    async fn delete_missing_file_returns_error() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let result = adapter.delete("nonexistent.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn from_config_creates_adapter() {
        let config = ResolvedLocalConfig {
            root: "/tmp/test-storage".to_string(),
            url: Some("http://example.com/files".to_string()),
            visibility: StorageVisibility::Public,
        };

        let adapter = LocalStorageAdapter::from_config(&config).unwrap();
        assert_eq!(adapter.root, PathBuf::from("/tmp/test-storage"));
        assert_eq!(adapter.url.as_deref(), Some("http://example.com/files"));
    }

    #[tokio::test]
    async fn put_bytes_with_content_type() {
        let dir = TempDir::new().unwrap();
        let adapter = make_adapter(&dir);

        let file = adapter
            .put_bytes(
                "data.json",
                b"{}",
                Some("application/json"),
                StorageVisibility::Private,
            )
            .await
            .unwrap();

        assert_eq!(file.content_type.as_deref(), Some("application/json"));
    }
}
