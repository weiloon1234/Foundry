use std::collections::BTreeMap;

use serde::Deserialize;

use crate::foundation::{Error, Result};

use super::adapter::StorageVisibility;

pub const DEFAULT_MAX_UPLOAD_SIZE_BYTES: u64 = 100 * 1024 * 1024;
pub const DEFAULT_MAX_UPLOAD_FILE_SIZE_BYTES: u64 = 50 * 1024 * 1024;
pub const DEFAULT_MAX_UPLOAD_FILES: u64 = 20;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub default: String,
    pub max_upload_size_bytes: u64,
    pub max_upload_file_size_bytes: u64,
    pub max_upload_files: u64,
    pub upload_temp_retention_seconds: u64,
    pub upload_temp_prune_interval_ms: u64,
    pub upload_temp_prune_batch_size: u64,
    pub image_max_input_bytes: u64,
    pub image_max_pixels: u64,
    pub image_max_width: u64,
    pub image_max_height: u64,
    pub attachment_orphan_audit_enabled: bool,
    pub attachment_orphan_delete_enabled: bool,
    pub attachment_orphan_retention_seconds: u64,
    pub attachment_orphan_prune_interval_ms: u64,
    pub attachment_orphan_prune_batch_size: u64,
    pub attachment_orphan_prefix: String,
    #[serde(default)]
    pub disks: BTreeMap<String, toml::Table>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            default: "local".to_string(),
            max_upload_size_bytes: DEFAULT_MAX_UPLOAD_SIZE_BYTES,
            max_upload_file_size_bytes: DEFAULT_MAX_UPLOAD_FILE_SIZE_BYTES,
            max_upload_files: DEFAULT_MAX_UPLOAD_FILES,
            upload_temp_retention_seconds: 3600,
            upload_temp_prune_interval_ms: 3_600_000,
            upload_temp_prune_batch_size: 1000,
            image_max_input_bytes: 52_428_800,
            image_max_pixels: 50_000_000,
            image_max_width: 12_000,
            image_max_height: 12_000,
            attachment_orphan_audit_enabled: true,
            attachment_orphan_delete_enabled: false,
            attachment_orphan_retention_seconds: 604_800,
            attachment_orphan_prune_interval_ms: 3_600_000,
            attachment_orphan_prune_batch_size: 100,
            attachment_orphan_prefix: "attachments/".to_string(),
            disks: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedLocalConfig {
    pub root: String,
    pub url: Option<String>,
    pub visibility: StorageVisibility,
}

impl ResolvedLocalConfig {
    pub fn from_table(table: &toml::Table) -> Result<Self> {
        let root = table
            .get("root")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("Missing required field 'root' for local disk config"))?
            .to_string();

        let url = table
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let visibility = visibility_from_table(table);

        Ok(Self {
            root,
            url,
            visibility,
        })
    }
}

#[derive(Clone)]
pub struct ResolvedS3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub key: String,
    pub secret: String,
    pub url: Option<String>,
    pub use_path_style: bool,
    pub visibility: StorageVisibility,
}

impl std::fmt::Debug for ResolvedS3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedS3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("key", &self.key)
            .field("secret", &crate::support::redaction::REDACTED)
            .field("url", &self.url)
            .field("use_path_style", &self.use_path_style)
            .field("visibility", &self.visibility)
            .finish()
    }
}

impl ResolvedS3Config {
    pub fn from_table(table: &toml::Table) -> Result<Self> {
        let bucket = table
            .get("bucket")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("Missing required field 'bucket' for S3 disk config"))?
            .to_string();

        let region = table
            .get("region")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("Missing required field 'region' for S3 disk config"))?
            .to_string();

        let key = table
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("Missing required field 'key' for S3 disk config"))?
            .to_string();

        let secret = table
            .get("secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("Missing required field 'secret' for S3 disk config"))?
            .to_string();

        let endpoint = table
            .get("endpoint")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let url = table
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let use_path_style = table
            .get("use_path_style")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let visibility = visibility_from_table(table);

        Ok(Self {
            bucket,
            region,
            endpoint,
            key,
            secret,
            url,
            use_path_style,
            visibility,
        })
    }
}

pub(crate) fn visibility_from_table(table: &toml::Table) -> StorageVisibility {
    table
        .get("visibility")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "public" => Some(StorageVisibility::Public),
            "private" => Some(StorageVisibility::Private),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_storage_config_has_local_default_and_empty_disks() {
        let config = StorageConfig::default();
        assert_eq!(config.default, "local");
        assert_eq!(config.max_upload_size_bytes, 104_857_600);
        assert_eq!(config.max_upload_file_size_bytes, 52_428_800);
        assert_eq!(config.max_upload_files, 20);
        assert_eq!(config.upload_temp_retention_seconds, 3600);
        assert_eq!(config.upload_temp_prune_interval_ms, 3_600_000);
        assert_eq!(config.upload_temp_prune_batch_size, 1000);
        assert_eq!(config.image_max_input_bytes, 52_428_800);
        assert_eq!(config.image_max_pixels, 50_000_000);
        assert_eq!(config.image_max_width, 12_000);
        assert_eq!(config.image_max_height, 12_000);
        assert!(config.attachment_orphan_audit_enabled);
        assert!(!config.attachment_orphan_delete_enabled);
        assert_eq!(config.attachment_orphan_retention_seconds, 604_800);
        assert_eq!(config.attachment_orphan_prune_interval_ms, 3_600_000);
        assert_eq!(config.attachment_orphan_prune_batch_size, 100);
        assert_eq!(config.attachment_orphan_prefix, "attachments/");
        assert!(config.disks.is_empty());
    }

    #[test]
    fn parses_storage_config_with_local_disk() {
        let raw = r#"
            default = "local"
            max_upload_size_bytes = 1048576
            max_upload_file_size_bytes = 524288
            max_upload_files = 5
            upload_temp_retention_seconds = 900
            upload_temp_prune_interval_ms = 60000
            upload_temp_prune_batch_size = 50
            image_max_input_bytes = 1024
            image_max_pixels = 2000000
            image_max_width = 2000
            image_max_height = 1000
            attachment_orphan_audit_enabled = false
            attachment_orphan_delete_enabled = true
            attachment_orphan_retention_seconds = 120
            attachment_orphan_prune_interval_ms = 30000
            attachment_orphan_prune_batch_size = 25
            attachment_orphan_prefix = "tenant-a/attachments/"

            [disks.local]
            root = "/tmp/storage"
            url = "http://localhost/storage"
            visibility = "public"
        "#;
        let config: StorageConfig = toml::from_str(raw).unwrap();
        assert_eq!(config.default, "local");
        assert_eq!(config.max_upload_size_bytes, 1_048_576);
        assert_eq!(config.max_upload_file_size_bytes, 524_288);
        assert_eq!(config.max_upload_files, 5);
        assert_eq!(config.upload_temp_retention_seconds, 900);
        assert_eq!(config.upload_temp_prune_interval_ms, 60_000);
        assert_eq!(config.upload_temp_prune_batch_size, 50);
        assert_eq!(config.image_max_input_bytes, 1024);
        assert_eq!(config.image_max_pixels, 2_000_000);
        assert_eq!(config.image_max_width, 2_000);
        assert_eq!(config.image_max_height, 1_000);
        assert!(!config.attachment_orphan_audit_enabled);
        assert!(config.attachment_orphan_delete_enabled);
        assert_eq!(config.attachment_orphan_retention_seconds, 120);
        assert_eq!(config.attachment_orphan_prune_interval_ms, 30_000);
        assert_eq!(config.attachment_orphan_prune_batch_size, 25);
        assert_eq!(config.attachment_orphan_prefix, "tenant-a/attachments/");
        assert!(config.disks.contains_key("local"));

        let local_table = &config.disks["local"];
        let resolved = ResolvedLocalConfig::from_table(local_table).unwrap();
        assert_eq!(resolved.root, "/tmp/storage");
        assert_eq!(resolved.url.as_deref(), Some("http://localhost/storage"));
        assert_eq!(resolved.visibility, StorageVisibility::Public);
    }

    #[test]
    fn parses_storage_config_with_s3_disk() {
        let raw = r#"
            default = "s3"

            [disks.s3]
            bucket = "my-bucket"
            region = "us-east-1"
            key = "AKIAIOSFODNN7EXAMPLE"
            secret = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
            endpoint = "https://s3.example.com"
            url = "https://cdn.example.com"
            use_path_style = true
            visibility = "public"
        "#;
        let config: StorageConfig = toml::from_str(raw).unwrap();
        assert_eq!(config.default, "s3");
        assert!(config.disks.contains_key("s3"));

        let s3_table = &config.disks["s3"];
        let resolved = ResolvedS3Config::from_table(s3_table).unwrap();
        assert_eq!(resolved.bucket, "my-bucket");
        assert_eq!(resolved.region, "us-east-1");
        assert_eq!(resolved.key, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(resolved.secret, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        assert_eq!(resolved.endpoint.as_deref(), Some("https://s3.example.com"));
        assert_eq!(resolved.url.as_deref(), Some("https://cdn.example.com"));
        assert!(resolved.use_path_style);
        assert_eq!(resolved.visibility, StorageVisibility::Public);
    }

    #[test]
    fn resolved_local_config_defaults_visibility_to_private() {
        let mut table = toml::Table::new();
        table.insert(
            "root".to_string(),
            toml::Value::String("/tmp/storage".to_string()),
        );

        let resolved = ResolvedLocalConfig::from_table(&table).unwrap();
        assert_eq!(resolved.visibility, StorageVisibility::Private);
        assert!(resolved.url.is_none());
    }

    #[test]
    fn resolved_local_config_missing_root_returns_error() {
        let table = toml::Table::new();
        let result = ResolvedLocalConfig::from_table(&table);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("root"));
    }

    #[test]
    fn resolved_s3_config_missing_bucket_returns_error() {
        let mut table = toml::Table::new();
        table.insert(
            "region".to_string(),
            toml::Value::String("us-east-1".to_string()),
        );
        table.insert("key".to_string(), toml::Value::String("key".to_string()));
        table.insert(
            "secret".to_string(),
            toml::Value::String("secret".to_string()),
        );

        let result = ResolvedS3Config::from_table(&table);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("bucket"));
    }

    #[test]
    fn resolved_s3_config_missing_region_returns_error() {
        let mut table = toml::Table::new();
        table.insert(
            "bucket".to_string(),
            toml::Value::String("my-bucket".to_string()),
        );
        table.insert("key".to_string(), toml::Value::String("key".to_string()));
        table.insert(
            "secret".to_string(),
            toml::Value::String("secret".to_string()),
        );

        let result = ResolvedS3Config::from_table(&table);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("region"));
    }

    #[test]
    fn resolved_s3_config_missing_key_returns_error() {
        let mut table = toml::Table::new();
        table.insert(
            "bucket".to_string(),
            toml::Value::String("my-bucket".to_string()),
        );
        table.insert(
            "region".to_string(),
            toml::Value::String("us-east-1".to_string()),
        );
        table.insert(
            "secret".to_string(),
            toml::Value::String("secret".to_string()),
        );

        let result = ResolvedS3Config::from_table(&table);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("key"));
    }

    #[test]
    fn resolved_s3_config_missing_secret_returns_error() {
        let mut table = toml::Table::new();
        table.insert(
            "bucket".to_string(),
            toml::Value::String("my-bucket".to_string()),
        );
        table.insert(
            "region".to_string(),
            toml::Value::String("us-east-1".to_string()),
        );
        table.insert("key".to_string(), toml::Value::String("key".to_string()));

        let result = ResolvedS3Config::from_table(&table);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("secret"));
    }

    #[test]
    fn resolved_s3_config_defaults() {
        let mut table = toml::Table::new();
        table.insert(
            "bucket".to_string(),
            toml::Value::String("my-bucket".to_string()),
        );
        table.insert(
            "region".to_string(),
            toml::Value::String("us-east-1".to_string()),
        );
        table.insert("key".to_string(), toml::Value::String("key".to_string()));
        table.insert(
            "secret".to_string(),
            toml::Value::String("secret".to_string()),
        );

        let resolved = ResolvedS3Config::from_table(&table).unwrap();
        assert!(resolved.endpoint.is_none());
        assert!(resolved.url.is_none());
        assert!(!resolved.use_path_style);
        assert_eq!(resolved.visibility, StorageVisibility::Private);
    }

    #[test]
    fn visibility_from_table_handles_invalid_gracefully() {
        let mut table = toml::Table::new();
        table.insert(
            "visibility".to_string(),
            toml::Value::String("invalid".to_string()),
        );
        assert_eq!(visibility_from_table(&table), StorageVisibility::Private);

        let empty_table = toml::Table::new();
        assert_eq!(
            visibility_from_table(&empty_table),
            StorageVisibility::Private
        );
    }
}
