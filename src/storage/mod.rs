pub mod adapter;
pub(crate) mod callback;
pub mod config;
pub mod disk;
pub mod local;
pub mod multipart;
pub(crate) mod path;
pub mod s3;
pub mod stored_file;
pub mod upload;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::config::ConfigRepository;
use crate::foundation::{Error, Result};
use crate::support::sync::lock_unpoisoned;

pub type StorageDriverFactory = Arc<
    dyn Fn(
            &ConfigRepository,
            &toml::Table,
        )
            -> Pin<Box<dyn Future<Output = Result<Arc<dyn adapter::StorageAdapter>>> + Send>>
        + Send
        + Sync,
>;

pub(crate) type StorageDriverRegistryHandle = Arc<Mutex<StorageDriverRegistryBuilder>>;

pub(crate) struct StorageDriverRegistryBuilder {
    drivers: HashMap<String, StorageDriverFactory>,
}

impl StorageDriverRegistryBuilder {
    pub(crate) fn shared() -> StorageDriverRegistryHandle {
        Arc::new(Mutex::new(Self {
            drivers: HashMap::new(),
        }))
    }

    pub(crate) fn register(&mut self, name: String, factory: StorageDriverFactory) -> Result<()> {
        if self.drivers.contains_key(&name) {
            return Err(Error::message(format!(
                "storage driver `{name}` already registered"
            )));
        }
        self.drivers.insert(name, factory);
        Ok(())
    }

    pub(crate) fn freeze_shared(
        handle: StorageDriverRegistryHandle,
    ) -> HashMap<String, StorageDriverFactory> {
        let mut builder = lock_unpoisoned(&handle, "storage driver registry");
        std::mem::take(&mut builder.drivers)
    }
}

#[derive(Clone)]
pub struct StorageManager {
    default: String,
    disks: Arc<HashMap<String, disk::StorageDisk>>,
}

impl std::fmt::Debug for StorageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageManager")
            .field("default", &self.default)
            .field("disks", &self.disks.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl StorageManager {
    pub async fn from_config(
        config: &ConfigRepository,
        custom_drivers: HashMap<String, StorageDriverFactory>,
    ) -> Result<Self> {
        let storage_config = config.storage()?;

        if storage_config.disks.is_empty() {
            return Ok(Self {
                default: storage_config.default,
                disks: Arc::new(HashMap::new()),
            });
        }

        let mut disks = HashMap::new();
        for (name, table) in &storage_config.disks {
            let driver = table
                .get("driver")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    Error::message(format!("disk `{name}` missing required 'driver' field"))
                })?;

            let adapter: Arc<dyn adapter::StorageAdapter> = match driver {
                "local" => {
                    let resolved = config::ResolvedLocalConfig::from_table(table)?;
                    Arc::new(local::LocalStorageAdapter::from_config(&resolved)?)
                }
                "s3" => {
                    let resolved = config::ResolvedS3Config::from_table(table)?;
                    Arc::new(s3::S3StorageAdapter::from_config(&resolved)?)
                }
                custom_name => {
                    let factory = custom_drivers.get(custom_name).ok_or_else(|| {
                        Error::message(format!("unknown storage driver `{custom_name}`"))
                    })?;
                    callback::build_storage_driver(custom_name, factory, config, table).await?
                }
            };

            let visibility = config::visibility_from_table(table);
            disks.insert(
                name.clone(),
                disk::StorageDisk::new(name.clone(), visibility, adapter),
            );
        }

        // Validate default disk exists
        if !disks.contains_key(&storage_config.default) && !storage_config.disks.is_empty() {
            return Err(Error::message(format!(
                "default disk `{}` is not configured",
                storage_config.default
            )));
        }

        Ok(Self {
            default: storage_config.default,
            disks: Arc::new(disks),
        })
    }

    pub fn default_disk(&self) -> Result<disk::StorageDisk> {
        self.disk(&self.default)
    }

    pub fn disk(&self, name: &str) -> Result<disk::StorageDisk> {
        self.disks
            .get(name)
            .cloned()
            .ok_or_else(|| Error::message(format!("disk `{name}` is not configured")))
    }

    pub fn default_disk_name(&self) -> &str {
        &self.default
    }

    pub fn configured_disks(&self) -> Vec<String> {
        let mut names: Vec<String> = self.disks.keys().cloned().collect();
        names.sort();
        names
    }

    // Convenience methods — delegate to default disk

    pub async fn put(
        &self,
        path: &str,
        contents: impl AsRef<[u8]>,
    ) -> Result<stored_file::StoredFile> {
        self.default_disk()?.put(path, contents).await
    }

    pub async fn put_bytes(
        &self,
        path: &str,
        bytes: impl AsRef<[u8]>,
    ) -> Result<stored_file::StoredFile> {
        self.default_disk()?.put_bytes(path, bytes).await
    }

    pub async fn put_file(
        &self,
        path: &str,
        temp_path: &std::path::Path,
        content_type: Option<&str>,
    ) -> Result<stored_file::StoredFile> {
        self.default_disk()?
            .put_file(path, temp_path, content_type)
            .await
    }

    pub async fn put_stream<R>(
        &self,
        path: &str,
        stream: R,
        content_type: Option<&str>,
    ) -> Result<stored_file::StoredFile>
    where
        R: tokio::io::AsyncRead + Send + 'static,
    {
        self.default_disk()?
            .put_stream(path, stream, content_type)
            .await
    }

    pub async fn get(&self, path: &str) -> Result<Vec<u8>> {
        self.default_disk()?.get(path).await
    }

    pub async fn get_stream(&self, path: &str) -> Result<adapter::StorageReadStream> {
        self.default_disk()?.get_stream(path).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.default_disk()?.delete(path).await
    }

    pub async fn exists(&self, path: &str) -> Result<bool> {
        self.default_disk()?.exists(path).await
    }

    pub async fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.default_disk()?.copy(from, to).await
    }

    pub async fn move_to(&self, from: &str, to: &str) -> Result<()> {
        self.default_disk()?.move_to(from, to).await
    }

    pub async fn url(&self, path: &str) -> Result<String> {
        self.default_disk()?.url(path).await
    }

    pub async fn temporary_url(
        &self,
        path: &str,
        expires_at: crate::support::DateTime,
    ) -> Result<String> {
        self.default_disk()?.temporary_url(path, expires_at).await
    }

    pub async fn list_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<stored_file::StorageObject>> {
        self.default_disk()?.list_prefix(prefix, limit).await
    }

    pub async fn list_prefix_after(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<stored_file::StorageObject>> {
        self.default_disk()?
            .list_prefix_after(prefix, after, limit)
            .await
    }
}

pub use adapter::{StorageAdapter, StorageReadStream, StorageVisibility, StorageWriteStream};
pub use config::{ResolvedLocalConfig, ResolvedS3Config, StorageConfig};
pub use disk::StorageDisk;
pub use local::LocalStorageAdapter;
pub use multipart::MultipartForm;
pub use s3::S3StorageAdapter;
pub use stored_file::{StorageObject, StoredFile};
pub use upload::{scope_upload_limits, UploadCounters, UploadLimits, UploadedFile};

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::TempDir;

    use super::*;

    fn config_from_toml(raw: &str) -> ConfigRepository {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("storage.toml");
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(raw.as_bytes()).unwrap();
        ConfigRepository::from_dir(dir.path()).unwrap()
    }

    #[tokio::test]
    async fn storage_manager_empty_config_returns_empty_disks() {
        let config = ConfigRepository::empty();
        let manager = StorageManager::from_config(&config, HashMap::new())
            .await
            .unwrap();

        assert_eq!(manager.default_disk_name(), "local");
        assert!(manager.configured_disks().is_empty());
        assert!(manager.default_disk().is_err());
    }

    #[tokio::test]
    async fn storage_manager_from_config_with_local_disk() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "local"

            [storage.disks.local]
            driver = "local"
            root = "/tmp/foundry-test-storage"
        "#,
        );
        let manager = StorageManager::from_config(&config, HashMap::new())
            .await
            .unwrap();

        assert_eq!(manager.default_disk_name(), "local");
        assert_eq!(manager.configured_disks(), vec!["local"]);

        let disk = manager.default_disk().unwrap();
        assert_eq!(disk.name(), "local");
    }

    #[tokio::test]
    async fn storage_manager_unknown_driver_returns_error() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "custom"

            [storage.disks.custom]
            driver = "custom_driver"
            root = "/tmp/test"
        "#,
        );
        let result = StorageManager::from_config(&config, HashMap::new()).await;
        let err = result.expect_err("should fail with unknown driver");
        assert!(err
            .to_string()
            .contains("unknown storage driver `custom_driver`"));
    }

    #[tokio::test]
    async fn storage_manager_missing_driver_field_returns_error() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "local"

            [storage.disks.local]
            root = "/tmp/test"
        "#,
        );
        let result = StorageManager::from_config(&config, HashMap::new()).await;
        let err = result.expect_err("should fail with missing driver field");
        assert!(err.to_string().contains("missing required 'driver' field"));
    }

    #[tokio::test]
    async fn storage_manager_default_disk_not_configured_returns_error() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "missing"

            [storage.disks.local]
            driver = "local"
            root = "/tmp/test"
        "#,
        );
        let result = StorageManager::from_config(&config, HashMap::new()).await;
        let err = result.expect_err("should fail with missing default disk");
        assert!(err
            .to_string()
            .contains("default disk `missing` is not configured"));
    }

    #[tokio::test]
    async fn storage_manager_disk_not_found_returns_error() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "local"

            [storage.disks.local]
            driver = "local"
            root = "/tmp/test"
        "#,
        );
        let manager = StorageManager::from_config(&config, HashMap::new())
            .await
            .unwrap();

        let result = manager.disk("nonexistent");
        let err = result.expect_err("should fail with unknown disk name");
        assert!(err
            .to_string()
            .contains("disk `nonexistent` is not configured"));
    }

    #[tokio::test]
    async fn storage_manager_custom_driver_via_registry() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "memory"

            [storage.disks.memory]
            driver = "memory"
        "#,
        );

        let factory: StorageDriverFactory = Arc::new(|_config, _table| {
            Box::pin(async { Err(Error::message("memory driver not yet implemented")) })
        });

        let mut custom = HashMap::new();
        custom.insert("memory".to_string(), factory);

        let result = StorageManager::from_config(&config, custom).await;
        let err = result.expect_err("should fail with factory error");
        assert!(err
            .to_string()
            .contains("memory driver not yet implemented"));
    }

    #[tokio::test]
    async fn storage_driver_factory_panic_becomes_error() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "panic"

            [storage.disks.panic]
            driver = "panic"
        "#,
        );

        let factory: StorageDriverFactory =
            Arc::new(|_config, _table| panic!("storage factory exploded"));
        let mut custom = HashMap::new();
        custom.insert("panic".to_string(), factory);

        let error = StorageManager::from_config(&config, custom)
            .await
            .expect_err("panicking storage driver factory should become an error");

        assert!(error
            .to_string()
            .contains("storage driver `panic` factory panicked: storage factory exploded"));
    }

    #[tokio::test]
    async fn storage_driver_factory_future_panic_becomes_error() {
        let config = config_from_toml(
            r#"
            [storage]
            default = "panic"

            [storage.disks.panic]
            driver = "panic"
        "#,
        );

        let factory: StorageDriverFactory = Arc::new(|_config, _table| {
            Box::pin(async { panic!("storage factory future exploded") })
        });
        let mut custom = HashMap::new();
        custom.insert("panic".to_string(), factory);

        let error = StorageManager::from_config(&config, custom)
            .await
            .expect_err("panicking storage driver factory future should become an error");

        assert!(error
            .to_string()
            .contains("storage driver `panic` factory panicked: storage factory future exploded"));
    }
}
