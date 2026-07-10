use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::path::Path as ObjectPath;
use object_store::{Attribute, ObjectStore, ObjectStoreExt, PutOptions};

use crate::foundation::{Error, Result};
use crate::support::DateTime;

use super::adapter::{StorageAdapter, StorageVisibility};
use super::config::ResolvedS3Config;
use super::path::{normalize_path, normalize_prefix};
use super::stored_file::{StorageObject, StoredFile};

pub struct S3StorageAdapter {
    inner: Arc<object_store::aws::AmazonS3>,
    bucket: String,
    region: String,
    url_prefix: Option<String>,
}

impl S3StorageAdapter {
    pub fn from_config(config: &ResolvedS3Config) -> Result<Self> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_region(&config.region)
            .with_access_key_id(&config.key)
            .with_secret_access_key(&config.secret);

        if let Some(endpoint) = &config.endpoint {
            if !endpoint.is_empty() {
                builder = builder.with_endpoint(endpoint);
            }
        }
        if config.use_path_style {
            builder = builder.with_virtual_hosted_style_request(false);
        }

        let store = builder.build().map_err(Error::other)?;
        Ok(Self {
            inner: Arc::new(store),
            bucket: config.bucket.clone(),
            region: config.region.clone(),
            url_prefix: config.url.clone(),
        })
    }

    fn file_name(path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string()
    }

    fn put_options(content_type: Option<&str>, _visibility: StorageVisibility) -> PutOptions {
        let mut options = PutOptions::default();
        if let Some(content_type) = content_type {
            options
                .attributes
                .insert(Attribute::ContentType, content_type.to_string().into());
        }
        options
    }
}

#[async_trait]
impl StorageAdapter for S3StorageAdapter {
    async fn put_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let path = normalize_path(path)?;
        let object_path = ObjectPath::from(path.as_str());
        self.inner
            .put_opts(
                &object_path,
                bytes.to_vec().into(),
                Self::put_options(content_type, visibility),
            )
            .await
            .map_err(Error::other)?;

        Ok(StoredFile {
            disk: String::new(),
            path: path.clone(),
            name: Self::file_name(&path),
            size: bytes.len() as u64,
            content_type: content_type.map(|s| s.to_string()),
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
        let path = normalize_path(path)?;
        let bytes = tokio::fs::read(temp_path).await.map_err(Error::other)?;
        self.put_bytes(&path, &bytes, content_type, visibility)
            .await
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let path = normalize_path(path)?;
        let object_path = ObjectPath::from(path.as_str());
        let result = self.inner.get(&object_path).await.map_err(Error::other)?;
        let bytes = result.bytes().await.map_err(Error::other)?;
        Ok(bytes.to_vec())
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let path = normalize_path(path)?;
        let object_path = ObjectPath::from(path.as_str());
        self.inner.delete(&object_path).await.map_err(Error::other)
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        let path = normalize_path(path)?;
        let object_path = ObjectPath::from(path.as_str());
        match self.inner.head(&object_path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(Error::other(e)),
        }
    }

    async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        let from_path = ObjectPath::from(from.as_str());
        let to_path = ObjectPath::from(to.as_str());
        self.inner
            .copy(&from_path, &to_path)
            .await
            .map_err(Error::other)
    }

    async fn move_to(&self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        let from_path = ObjectPath::from(from.as_str());
        let to_path = ObjectPath::from(to.as_str());
        self.inner
            .rename(&from_path, &to_path)
            .await
            .map_err(Error::other)
    }

    async fn url(&self, path: &str) -> Result<String> {
        let path = normalize_path(path)?;
        match &self.url_prefix {
            Some(prefix) => Ok(format!("{prefix}/{path}")),
            None => Ok(format!(
                "https://{}.s3.{}.amazonaws.com/{path}",
                self.bucket, self.region
            )),
        }
    }

    async fn temporary_url(&self, path: &str, expires_at: DateTime) -> Result<String> {
        use object_store::signer::Signer;
        use std::time::Duration;

        let path = normalize_path(path)?;

        let now_ms = DateTime::now().timestamp_millis();
        let expires_ms = expires_at.timestamp_millis();
        let secs = (expires_ms - now_ms) / 1000;
        if secs <= 0 {
            return Err(Error::message("expiration must be in the future"));
        }

        let object_path = ObjectPath::from(path.as_str());
        let url = self
            .inner
            .signed_url(
                reqwest::Method::GET,
                &object_path,
                Duration::from_secs(secs as u64),
            )
            .await
            .map_err(Error::other)?;

        Ok(url.to_string())
    }

    async fn list_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<StorageObject>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let prefix = normalize_prefix(prefix)?;
        let prefix = ObjectPath::from(prefix.as_str());
        let mut stream = self.inner.list(Some(&prefix));
        let mut objects = Vec::new();

        while let Some(next) = stream.next().await {
            let meta = next.map_err(Error::other)?;
            objects.push(StorageObject {
                path: meta.location.to_string(),
                size: meta.size,
                modified_at: DateTime::from_chrono(meta.last_modified),
            });
            if objects.len() >= limit {
                break;
            }
        }

        objects.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use object_store::Attribute;

    use super::{S3StorageAdapter, StorageVisibility};

    #[test]
    fn put_options_include_content_type_object_metadata() {
        let options = S3StorageAdapter::put_options(Some("image/webp"), StorageVisibility::Private);

        assert_eq!(
            options
                .attributes
                .get(&Attribute::ContentType)
                .map(|value| value.as_ref()),
            Some("image/webp")
        );
    }

    #[test]
    fn put_options_leave_content_type_unset_when_not_provided() {
        let options = S3StorageAdapter::put_options(None, StorageVisibility::Private);

        assert!(options.attributes.is_empty());
    }

    #[test]
    fn put_options_do_not_translate_visibility_into_an_object_acl() {
        let private = S3StorageAdapter::put_options(None, StorageVisibility::Private);
        let public = S3StorageAdapter::put_options(None, StorageVisibility::Public);

        assert_eq!(private, public);
    }
}
