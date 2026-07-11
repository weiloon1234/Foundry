use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::foundation::{Error, Result};
use crate::storage::{
    StorageAdapter, StorageDriverFactory, StorageObject, StorageVisibility, StoredFile,
};
use crate::support::sync::lock_unpoisoned;
use crate::support::DateTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFakeFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    pub visibility: StorageVisibility,
}

#[derive(Clone)]
struct FakeStorageEntry {
    file: StoredFakeFile,
    modified_at: DateTime,
}

/// In-memory storage adapter with deterministic reads and fluent assertions.
#[derive(Clone, Default)]
pub struct StorageFake {
    entries: Arc<Mutex<HashMap<String, FakeStorageEntry>>>,
    writes: Arc<Mutex<Vec<String>>>,
}

impl StorageFake {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a custom storage-driver factory backed by this fake.
    ///
    /// Register it from a test provider and select that driver in the test
    /// storage config when application code resolves [`crate::storage::StorageManager`].
    pub fn driver_factory(&self) -> StorageDriverFactory {
        let fake = self.clone();
        Arc::new(move |_config, _table| {
            let adapter = fake.clone();
            Box::pin(async move { Ok(Arc::new(adapter) as Arc<dyn StorageAdapter>) })
        })
    }

    pub fn files(&self) -> Vec<StoredFakeFile> {
        let mut files = lock_unpoisoned(&self.entries, "storage fake")
            .values()
            .map(|entry| entry.file.clone())
            .collect::<Vec<_>>();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        files
    }

    pub fn reset(&self) -> &Self {
        lock_unpoisoned(&self.entries, "storage fake").clear();
        lock_unpoisoned(&self.writes, "storage fake writes").clear();
        self
    }

    #[track_caller]
    pub fn assert_exists(&self, path: &str) -> &Self {
        assert!(
            lock_unpoisoned(&self.entries, "storage fake").contains_key(path),
            "expected fake storage path `{path}` to exist"
        );
        self
    }

    #[track_caller]
    pub fn assert_missing(&self, path: &str) -> &Self {
        assert!(
            !lock_unpoisoned(&self.entries, "storage fake").contains_key(path),
            "expected fake storage path `{path}` to be missing"
        );
        self
    }

    #[track_caller]
    pub fn assert_content(&self, path: &str, expected: impl AsRef<[u8]>) -> &Self {
        let entries = lock_unpoisoned(&self.entries, "storage fake");
        let entry = entries
            .get(path)
            .unwrap_or_else(|| panic!("fake storage path `{path}` does not exist"));
        assert_eq!(
            entry.file.bytes,
            expected.as_ref(),
            "unexpected content for fake storage path `{path}`"
        );
        self
    }

    #[track_caller]
    pub fn assert_written_count(&self, expected: usize) -> &Self {
        let actual = lock_unpoisoned(&self.writes, "storage fake writes").len();
        assert_eq!(
            actual, expected,
            "expected {expected} fake storage write(s), recorded {actual}"
        );
        self
    }

    fn stored_file(path: &str, entry: &StoredFakeFile) -> StoredFile {
        let name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_string();
        StoredFile {
            disk: String::new(),
            path: path.to_string(),
            name,
            size: entry.bytes.len() as u64,
            content_type: entry.content_type.clone(),
            url: (entry.visibility == StorageVisibility::Public)
                .then(|| format!("https://storage.fake/{path}")),
        }
    }
}

#[async_trait]
impl StorageAdapter for StorageFake {
    async fn put_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let file = StoredFakeFile {
            path: path.to_string(),
            bytes: bytes.to_vec(),
            content_type: content_type.map(ToOwned::to_owned),
            visibility,
        };
        lock_unpoisoned(&self.entries, "storage fake").insert(
            path.to_string(),
            FakeStorageEntry {
                file: file.clone(),
                modified_at: DateTime::now(),
            },
        );
        lock_unpoisoned(&self.writes, "storage fake writes").push(path.to_string());
        Ok(Self::stored_file(path, &file))
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
        lock_unpoisoned(&self.entries, "storage fake")
            .get(path)
            .map(|entry| entry.file.bytes.clone())
            .ok_or_else(|| Error::message(format!("fake storage path `{path}` does not exist")))
    }

    async fn delete(&self, path: &str) -> Result<()> {
        lock_unpoisoned(&self.entries, "storage fake").remove(path);
        Ok(())
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        Ok(lock_unpoisoned(&self.entries, "storage fake").contains_key(path))
    }

    async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let source = lock_unpoisoned(&self.entries, "storage fake")
            .get(from)
            .cloned()
            .ok_or_else(|| Error::message(format!("fake storage path `{from}` does not exist")))?;
        let mut copied = source.file;
        copied.path = to.to_string();
        lock_unpoisoned(&self.entries, "storage fake").insert(
            to.to_string(),
            FakeStorageEntry {
                file: copied,
                modified_at: DateTime::now(),
            },
        );
        lock_unpoisoned(&self.writes, "storage fake writes").push(to.to_string());
        Ok(())
    }

    async fn move_to(&self, from: &str, to: &str) -> Result<()> {
        self.copy(from, to).await?;
        self.delete(from).await
    }

    async fn url(&self, path: &str) -> Result<String> {
        if !self.exists(path).await? {
            return Err(Error::message(format!(
                "fake storage path `{path}` does not exist"
            )));
        }
        Ok(format!("https://storage.fake/{path}"))
    }

    async fn temporary_url(&self, path: &str, expires_at: DateTime) -> Result<String> {
        if !self.exists(path).await? {
            return Err(Error::message(format!(
                "fake storage path `{path}` does not exist"
            )));
        }
        Ok(format!(
            "https://storage.fake/{path}?expires={}",
            expires_at.timestamp_millis()
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
        let entries = lock_unpoisoned(&self.entries, "storage fake");
        let mut objects = entries
            .values()
            .filter(|entry| entry.file.path.starts_with(prefix))
            .filter(|entry| after.is_none_or(|cursor| entry.file.path.as_str() > cursor))
            .map(|entry| StorageObject {
                path: entry.file.path.clone(),
                size: entry.file.bytes.len() as u64,
                modified_at: entry.modified_at,
            })
            .collect::<Vec<_>>();
        objects.sort_by(|left, right| left.path.cmp(&right.path));
        objects.truncate(limit);
        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_supports_crud_listing_and_assertions() {
        let fake = StorageFake::new();
        fake.put_bytes(
            "reports/one.txt",
            b"one",
            Some("text/plain"),
            StorageVisibility::Private,
        )
        .await
        .unwrap();
        fake.copy("reports/one.txt", "reports/two.txt")
            .await
            .unwrap();

        fake.assert_exists("reports/one.txt")
            .assert_content("reports/two.txt", b"one")
            .assert_written_count(2);
        assert_eq!(fake.list_prefix("reports/", 10).await.unwrap().len(), 2);

        fake.delete("reports/one.txt").await.unwrap();
        fake.assert_missing("reports/one.txt");
    }
}
