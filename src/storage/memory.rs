use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::foundation::{Error, Result};
use crate::support::sync::lock_unpoisoned;
use crate::support::DateTime;

use super::adapter::{StorageAdapter, StorageVisibility};
use super::path::{normalize_path, normalize_prefix};
use super::stored_file::{StorageObject, StoredFile};

#[derive(Clone, Debug)]
struct MemoryObject {
    bytes: Vec<u8>,
    modified_at: DateTime,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStorageAdapter {
    objects: Arc<Mutex<HashMap<String, MemoryObject>>>,
}

impl MemoryStorageAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    fn file_name(path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_string()
    }

    fn missing_object(path: &str) -> Error {
        Error::message(format!("storage object `{path}` not found"))
    }

    fn prefix_matches(path: &str, prefix: &str) -> bool {
        if path == prefix {
            return true;
        }

        let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
        path.strip_prefix(prefix)
            .is_some_and(|remaining| remaining.starts_with('/'))
    }
}

#[async_trait]
impl StorageAdapter for MemoryStorageAdapter {
    async fn put_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
        _visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let path = normalize_path(path)?;
        let object = MemoryObject {
            bytes: bytes.to_vec(),
            modified_at: DateTime::now(),
        };

        lock_unpoisoned(&self.objects, "memory storage objects").insert(path.clone(), object);

        Ok(StoredFile {
            disk: String::new(),
            path: path.clone(),
            name: Self::file_name(&path),
            size: bytes.len() as u64,
            content_type: content_type.map(str::to_string),
            url: None,
        })
    }

    async fn put_file(
        &self,
        path: &str,
        temp_path: &Path,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let bytes = tokio::fs::read(temp_path).await.map_err(Error::other)?;
        self.put_bytes(path, &bytes, content_type, visibility).await
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let path = normalize_path(path)?;
        lock_unpoisoned(&self.objects, "memory storage objects")
            .get(&path)
            .map(|object| object.bytes.clone())
            .ok_or_else(|| Self::missing_object(&path))
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let path = normalize_path(path)?;
        lock_unpoisoned(&self.objects, "memory storage objects")
            .remove(&path)
            .map(|_| ())
            .ok_or_else(|| Self::missing_object(&path))
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        let path = normalize_path(path)?;
        Ok(lock_unpoisoned(&self.objects, "memory storage objects").contains_key(&path))
    }

    async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        let mut objects = lock_unpoisoned(&self.objects, "memory storage objects");
        let mut object = objects
            .get(&from)
            .cloned()
            .ok_or_else(|| Self::missing_object(&from))?;
        object.modified_at = DateTime::now();
        objects.insert(to, object);
        Ok(())
    }

    async fn move_to(&self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        let mut objects = lock_unpoisoned(&self.objects, "memory storage objects");
        let mut object = objects
            .remove(&from)
            .ok_or_else(|| Self::missing_object(&from))?;
        object.modified_at = DateTime::now();
        objects.insert(to, object);
        Ok(())
    }

    async fn url(&self, path: &str) -> Result<String> {
        normalize_path(path)?;
        Err(Error::message(
            "URL generation is not supported for memory storage disks",
        ))
    }

    async fn temporary_url(&self, path: &str, _expires_at: DateTime) -> Result<String> {
        normalize_path(path)?;
        Err(Error::message(
            "Temporary URLs are not supported for memory storage disks",
        ))
    }

    async fn list_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<StorageObject>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let prefix = normalize_prefix(prefix)?;
        let mut objects = lock_unpoisoned(&self.objects, "memory storage objects")
            .iter()
            .filter(|(path, _)| Self::prefix_matches(path, &prefix))
            .map(|(path, object)| StorageObject {
                path: path.clone(),
                size: object.bytes.len() as u64,
                modified_at: object.modified_at,
            })
            .collect::<Vec<_>>();

        objects.sort_by(|left, right| left.path.cmp(&right.path));
        objects.truncate(limit);
        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn put_get_copy_move_delete_and_list_objects() {
        let adapter = MemoryStorageAdapter::new();

        let file = adapter
            .put_bytes(
                "attachments/b.txt",
                b"bbb",
                Some("text/plain"),
                StorageVisibility::Private,
            )
            .await
            .unwrap();
        assert_eq!(file.path, "attachments/b.txt");
        assert_eq!(file.name, "b.txt");
        assert_eq!(file.size, 3);
        assert_eq!(file.content_type.as_deref(), Some("text/plain"));
        assert!(file.disk.is_empty());

        assert_eq!(adapter.get("attachments/b.txt").await.unwrap(), b"bbb");
        assert!(adapter.exists("attachments/b.txt").await.unwrap());

        adapter
            .copy("attachments/b.txt", "attachments/a.txt")
            .await
            .unwrap();
        adapter
            .move_to("attachments/a.txt", "attachments/nested/a.txt")
            .await
            .unwrap();
        assert!(!adapter.exists("attachments/a.txt").await.unwrap());

        let objects = adapter.list_prefix("attachments/", 10).await.unwrap();
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].path, "attachments/b.txt");
        assert_eq!(objects[1].path, "attachments/nested/a.txt");

        let limited = adapter.list_prefix("attachments/", 1).await.unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].path, "attachments/b.txt");

        adapter.delete("attachments/b.txt").await.unwrap();
        assert!(!adapter.exists("attachments/b.txt").await.unwrap());
    }

    #[tokio::test]
    async fn put_file_reads_upload_bytes() {
        let adapter = MemoryStorageAdapter::new();
        let temp = TempDir::new().unwrap();
        let upload = temp.path().join("upload.bin");
        let mut file = std::fs::File::create(&upload).unwrap();
        file.write_all(b"from temp").unwrap();

        let stored = adapter
            .put_file(
                "uploads/file.bin",
                &upload,
                Some("application/octet-stream"),
                StorageVisibility::Public,
            )
            .await
            .unwrap();

        assert_eq!(stored.path, "uploads/file.bin");
        assert_eq!(stored.size, 9);
        assert_eq!(adapter.get("uploads/file.bin").await.unwrap(), b"from temp");
    }

    #[tokio::test]
    async fn missing_objects_and_urls_return_errors() {
        let adapter = MemoryStorageAdapter::new();

        assert!(adapter
            .get("missing.txt")
            .await
            .unwrap_err()
            .to_string()
            .contains("storage object `missing.txt` not found"));
        assert!(adapter
            .delete("missing.txt")
            .await
            .unwrap_err()
            .to_string()
            .contains("storage object `missing.txt` not found"));
        assert!(adapter
            .url("missing.txt")
            .await
            .unwrap_err()
            .to_string()
            .contains("URL generation is not supported"));
        assert!(adapter
            .temporary_url("missing.txt", DateTime::now())
            .await
            .unwrap_err()
            .to_string()
            .contains("Temporary URLs are not supported"));
    }

    #[tokio::test]
    async fn prefix_matching_does_not_include_sibling_prefixes() {
        let adapter = MemoryStorageAdapter::new();
        adapter
            .put_bytes("attachments/a.txt", b"a", None, StorageVisibility::Private)
            .await
            .unwrap();
        adapter
            .put_bytes(
                "attachments-other/b.txt",
                b"b",
                None,
                StorageVisibility::Private,
            )
            .await
            .unwrap();

        let objects = adapter.list_prefix("attachments", 10).await.unwrap();

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].path, "attachments/a.txt");
    }

    #[tokio::test]
    async fn invalid_paths_are_rejected() {
        let adapter = MemoryStorageAdapter::new();

        assert!(adapter
            .put_bytes("../secret.txt", b"nope", None, StorageVisibility::Private)
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid storage path"));
        assert!(adapter
            .list_prefix("../", 10)
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid storage prefix"));
    }
}
