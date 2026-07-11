use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use object_store::aws::{AmazonS3Builder, AmazonS3ConfigKey};
use object_store::path::Path as ObjectPath;
use object_store::{
    Attribute, MultipartUpload, ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions,
    PutPayload,
};
use tokio::io::AsyncReadExt as _;

use crate::foundation::{Error, Result};
use crate::support::DateTime;

use super::adapter::{StorageAdapter, StorageReadStream, StorageVisibility, StorageWriteStream};
use super::config::ResolvedS3Config;
use super::path::{join_url_prefix, normalize_path, normalize_prefix};
use super::stored_file::{StorageObject, StoredFile};

pub struct S3StorageAdapter {
    inner: Arc<object_store::aws::AmazonS3>,
    bucket: String,
    region: String,
    endpoint: Option<String>,
    url_prefix: Option<String>,
    visibility: StorageVisibility,
}

impl S3StorageAdapter {
    pub fn from_config(config: &ResolvedS3Config) -> Result<Self> {
        let mut builder = match (&config.key, &config.secret) {
            (Some(key), Some(secret)) => {
                let builder = AmazonS3Builder::new()
                    .with_access_key_id(key)
                    .with_secret_access_key(secret);
                match &config.session_token {
                    Some(token) => builder.with_token(token),
                    None => builder,
                }
            }
            (None, None) => AmazonS3Builder::from_env(),
            _ => {
                return Err(Error::message(
                    "S3 explicit credentials require both key and secret",
                ));
            }
        }
        .with_bucket_name(&config.bucket)
        .with_region(&config.region);

        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_config(AmazonS3ConfigKey::S3Endpoint, endpoint);
        }
        if config.use_path_style {
            builder = builder.with_virtual_hosted_style_request(false);
        }

        let store = builder.build().map_err(Error::other)?;
        Ok(Self {
            inner: Arc::new(store),
            bucket: config.bucket.clone(),
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
            url_prefix: config.url.clone(),
            visibility: config.visibility,
        })
    }

    fn file_name(path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string()
    }

    fn public_url(&self, path: &str) -> Option<String> {
        if let Some(prefix) = &self.url_prefix {
            return join_url_prefix(prefix, path);
        }
        if let Some(endpoint) = &self.endpoint {
            return join_url_prefix(endpoint, &format!("{}/{path}", self.bucket));
        }

        Some(format!(
            "https://{}.s3.{}.amazonaws.com/{path}",
            self.bucket, self.region
        ))
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

    fn put_options(content_type: Option<&str>, _visibility: StorageVisibility) -> PutOptions {
        let mut options = PutOptions::default();
        if let Some(content_type) = content_type {
            options
                .attributes
                .insert(Attribute::ContentType, content_type.to_string().into());
        }
        options
    }

    fn put_multipart_options(
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> PutMultipartOptions {
        let options = Self::put_options(content_type, visibility);
        PutMultipartOptions {
            attributes: options.attributes,
            ..Default::default()
        }
    }

    async fn abort_with_error(upload: &mut dyn MultipartUpload, error: Error) -> Error {
        match upload.abort().await {
            Ok(()) => error,
            Err(abort_error) => Error::message(format!(
                "{error}; additionally failed to abort S3 multipart upload: {abort_error}"
            )),
        }
    }

    async fn upload_stream(
        store: &dyn ObjectStore,
        object_path: &ObjectPath,
        mut stream: StorageWriteStream,
        put_options: PutOptions,
        multipart_options: PutMultipartOptions,
    ) -> Result<u64> {
        const READ_CHUNK_SIZE: usize = 64 * 1024;
        const MULTIPART_CHUNK_SIZE: usize = 5 * 1024 * 1024;

        let mut read_buffer = vec![0; READ_CHUNK_SIZE];
        let first_read = stream.read(&mut read_buffer).await.map_err(Error::other)?;
        if first_read == 0 {
            store
                .put_opts(object_path, PutPayload::new(), put_options)
                .await
                .map_err(Error::other)?;
            return Ok(0);
        }

        let mut upload = store
            .put_multipart_opts(object_path, multipart_options)
            .await
            .map_err(Error::other)?;
        let mut part = Vec::with_capacity(MULTIPART_CHUNK_SIZE);
        part.extend_from_slice(&read_buffer[..first_read]);
        let mut size = first_read as u64;
        let mut finished = false;

        while !finished {
            while part.len() < MULTIPART_CHUNK_SIZE {
                let remaining = MULTIPART_CHUNK_SIZE - part.len();
                let read = match stream
                    .read(&mut read_buffer[..remaining.min(READ_CHUNK_SIZE)])
                    .await
                {
                    Ok(read) => read,
                    Err(error) => {
                        return Err(
                            Self::abort_with_error(upload.as_mut(), Error::other(error)).await
                        );
                    }
                };
                if read == 0 {
                    finished = true;
                    break;
                }

                part.extend_from_slice(&read_buffer[..read]);
                size = match size.checked_add(read as u64) {
                    Some(size) => size,
                    None => {
                        return Err(Self::abort_with_error(
                            upload.as_mut(),
                            Error::message("storage stream size exceeds u64"),
                        )
                        .await);
                    }
                };
            }

            if part.is_empty() {
                continue;
            }

            let payload = PutPayload::from(std::mem::replace(
                &mut part,
                Vec::with_capacity(MULTIPART_CHUNK_SIZE),
            ));
            if let Err(error) = upload.put_part(payload).await {
                return Err(Self::abort_with_error(upload.as_mut(), Error::other(error)).await);
            }
        }

        if let Err(error) = upload.complete().await {
            return Err(Self::abort_with_error(upload.as_mut(), Error::other(error)).await);
        }

        Ok(size)
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

        Ok(self.stored_file(path, bytes.len() as u64, content_type, visibility))
    }

    async fn put_file(
        &self,
        path: &str,
        temp_path: &Path,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let file = tokio::fs::File::open(temp_path)
            .await
            .map_err(Error::other)?;
        self.put_stream(path, Box::pin(file), content_type, visibility)
            .await
    }

    async fn put_stream(
        &self,
        path: &str,
        stream: StorageWriteStream,
        content_type: Option<&str>,
        visibility: StorageVisibility,
    ) -> Result<StoredFile> {
        let path = normalize_path(path)?;
        let object_path = ObjectPath::from(path.as_str());
        let size = Self::upload_stream(
            self.inner.as_ref(),
            &object_path,
            stream,
            Self::put_options(content_type, visibility),
            Self::put_multipart_options(content_type, visibility),
        )
        .await?;

        Ok(self.stored_file(path, size, content_type, visibility))
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let path = normalize_path(path)?;
        let object_path = ObjectPath::from(path.as_str());
        let result = self.inner.get(&object_path).await.map_err(Error::other)?;
        let bytes = result.bytes().await.map_err(Error::other)?;
        Ok(bytes.to_vec())
    }

    async fn get_stream(&self, path: &str) -> Result<StorageReadStream> {
        let path = normalize_path(path)?;
        let object_path = ObjectPath::from(path.as_str());
        let result = self.inner.get(&object_path).await.map_err(Error::other)?;
        let stream = result
            .into_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()).map_err(Error::other));
        Ok(Box::pin(stream))
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
        if self.visibility == StorageVisibility::Private {
            return Err(Error::message(
                "private storage disks do not expose stable public URLs; use temporary_url instead",
            ));
        }

        self.public_url(&path).ok_or_else(|| {
            Error::message("stable public URL generation is not supported for this S3 disk")
        })
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
        let prefix = ObjectPath::from(prefix.as_str());
        let after = after.map(|cursor| ObjectPath::from(cursor.as_str()));
        let mut stream = match after.as_ref() {
            Some(after) => self.inner.list_with_offset(Some(&prefix), after),
            None => self.inner.list(Some(&prefix)),
        };
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
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use object_store::memory::InMemory;
    use object_store::path::Path as ObjectPath;
    use object_store::{Attribute, ObjectStoreExt, PutMultipartOptions, PutOptions};
    use tokio::io::{AsyncRead, ReadBuf};

    use super::{ResolvedS3Config, S3StorageAdapter, StorageAdapter, StorageVisibility};

    fn config(visibility: StorageVisibility) -> ResolvedS3Config {
        ResolvedS3Config {
            bucket: "foundry-test-bucket".to_string(),
            region: "ap-southeast-1".to_string(),
            endpoint: None,
            key: Some("access-key".to_string()),
            secret: Some("secret-key".to_string()),
            session_token: None,
            url: None,
            use_path_style: false,
            visibility,
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
                return Poll::Ready(Err(std::io::Error::other("source stream failed")));
            }

            self.yielded = true;
            buffer.put_slice(b"partial");
            Poll::Ready(Ok(()))
        }
    }

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

    #[test]
    fn builds_with_aws_default_credential_provider_resolution() {
        let mut config = config(StorageVisibility::Private);
        config.key = None;
        config.secret = None;

        S3StorageAdapter::from_config(&config).unwrap();
    }

    #[test]
    fn builds_with_explicit_temporary_credentials() {
        let mut config = config(StorageVisibility::Private);
        config.session_token = Some("session-token".to_string());

        S3StorageAdapter::from_config(&config).unwrap();
    }

    #[tokio::test]
    async fn public_urls_use_explicit_prefix_standard_aws_or_custom_endpoint() {
        let mut explicit = config(StorageVisibility::Public);
        explicit.url = Some("https://cdn.example.com/assets/".to_string());
        let explicit = S3StorageAdapter::from_config(&explicit).unwrap();
        assert_eq!(
            explicit.url("images/a.jpg").await.unwrap(),
            "https://cdn.example.com/assets/images/a.jpg"
        );

        let aws = S3StorageAdapter::from_config(&config(StorageVisibility::Public)).unwrap();
        assert_eq!(
            aws.url("images/a.jpg").await.unwrap(),
            "https://foundry-test-bucket.s3.ap-southeast-1.amazonaws.com/images/a.jpg"
        );

        let mut custom = config(StorageVisibility::Public);
        custom.endpoint = Some("https://objects.example.com/".to_string());
        let custom = S3StorageAdapter::from_config(&custom).unwrap();
        assert_eq!(
            custom.url("images/a.jpg").await.unwrap(),
            "https://objects.example.com/foundry-test-bucket/images/a.jpg"
        );
    }

    #[tokio::test]
    async fn private_s3_urls_require_temporary_url_instead() {
        let mut config = config(StorageVisibility::Private);
        config.url = Some("https://cdn.example.com".to_string());
        let adapter = S3StorageAdapter::from_config(&config).unwrap();

        let error = adapter.url("private/report.pdf").await.unwrap_err();

        assert!(error.to_string().contains("private storage disks"));
        assert!(error.to_string().contains("temporary_url"));
    }

    #[test]
    fn stored_file_url_matches_write_visibility() {
        let adapter = S3StorageAdapter::from_config(&config(StorageVisibility::Public)).unwrap();

        let public = adapter.stored_file(
            "images/a.jpg".to_string(),
            12,
            Some("image/jpeg"),
            StorageVisibility::Public,
        );
        let private = adapter.stored_file(
            "images/a.jpg".to_string(),
            12,
            Some("image/jpeg"),
            StorageVisibility::Private,
        );

        assert_eq!(
            public.url.as_deref(),
            Some("https://foundry-test-bucket.s3.ap-southeast-1.amazonaws.com/images/a.jpg")
        );
        assert!(private.url.is_none());
    }

    #[tokio::test]
    async fn object_store_upload_stream_uses_bounded_multipart_io() {
        let store = InMemory::new();
        let path = ObjectPath::from("streams/large.bin");
        let contents = vec![7; 5 * 1024 * 1024 + 128 * 1024];

        let size = tokio::time::timeout(
            Duration::from_secs(5),
            S3StorageAdapter::upload_stream(
                &store,
                &path,
                Box::pin(std::io::Cursor::new(contents.clone())),
                PutOptions::default(),
                PutMultipartOptions::default(),
            ),
        )
        .await
        .expect("multipart stream upload timed out")
        .unwrap();
        let stored = store.get(&path).await.unwrap().bytes().await.unwrap();

        assert_eq!(size, contents.len() as u64);
        assert_eq!(stored.as_ref(), contents);
    }

    #[tokio::test]
    async fn object_store_upload_stream_aborts_after_source_error() {
        let store = InMemory::new();
        let path = ObjectPath::from("streams/failure.bin");

        let error = S3StorageAdapter::upload_stream(
            &store,
            &path,
            Box::pin(FailingReader { yielded: false }),
            PutOptions::default(),
            PutMultipartOptions::default(),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("source stream failed"));
        assert!(matches!(
            store.head(&path).await,
            Err(object_store::Error::NotFound { .. })
        ));
    }
}
