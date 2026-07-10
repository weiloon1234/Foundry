use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use axum::extract::multipart::Field;
use axum::extract::FromRef;
use axum::extract::FromRequest;
use axum::response::{IntoResponse, Response};
use tokio::io::AsyncWriteExt as _;

use crate::foundation::{AppContext, Error, Result};
use crate::support::filename::{safe_extension_from_name, sanitize_filename};
use serde::de::{self, Deserialize, Deserializer};

use super::stored_file::StoredFile;
use super::StorageConfig;
use super::StorageManager;

const UPLOAD_TEMP_PREFIX: &str = "foundry-upload-";
const UPLOAD_TEMP_DIR: &str = "foundry/uploads";
const FALLBACK_UPLOAD_FILENAME: &str = "upload";
const MAX_UPLOAD_FILENAME_BYTES: usize = 255;

/// Represents a file received from an HTTP request (multipart upload).
///
/// Contains metadata about the upload (original name, content type, size)
/// and the temporary path where the file body was written by the HTTP layer.
///
/// Helper methods generate safe storage names and paths. The borrowed `store*`
/// methods intentionally leave the temporary file in place so callers can read,
/// transform, or store it more than once. When the caller owns the upload and no
/// longer needs its temporary bytes, prefer the consuming `store*_and_cleanup`
/// methods so Foundry removes its temporary file immediately after the storage
/// attempt.
#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub field_name: String,
    pub original_name: Option<String>,
    pub content_type: Option<String>,
    pub size: u64,
    pub temp_path: PathBuf,
}

impl ts_rs::TS for UploadedFile {
    type WithoutGenerics = Self;

    fn name() -> String {
        "File".to_string()
    }

    fn inline() -> String {
        "File".to_string()
    }

    fn inline_flattened() -> String {
        panic!("{} cannot be flattened", Self::name())
    }

    fn decl() -> String {
        panic!("{} cannot be declared", Self::name())
    }

    fn decl_concrete() -> String {
        panic!("{} cannot be declared", Self::name())
    }
}

impl crate::openapi::ApiSchema for UploadedFile {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type": "string", "format": "binary"})
    }

    fn schema_name() -> &'static str {
        "UploadedFile"
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadLimits {
    pub max_upload_size_bytes: u64,
    pub max_upload_file_size_bytes: u64,
    pub max_upload_files: u64,
}

impl UploadLimits {
    pub fn from_config(config: &StorageConfig) -> Self {
        Self {
            max_upload_size_bytes: config.max_upload_size_bytes,
            max_upload_file_size_bytes: config.max_upload_file_size_bytes,
            max_upload_files: config.max_upload_files,
        }
    }

    pub(crate) fn from_app(app: &AppContext) -> Self {
        app.config()
            .storage()
            .map(|config| Self::from_config(&config))
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadCounters {
    pub uploaded_bytes: u64,
    pub uploaded_files: u64,
}

tokio::task_local! {
    static CURRENT_UPLOAD_LIMITS: UploadLimits;
}

pub async fn scope_upload_limits<F>(limits: UploadLimits, future: F) -> F::Output
where
    F: Future,
{
    CURRENT_UPLOAD_LIMITS.scope(limits, future).await
}

pub fn current_upload_limits() -> UploadLimits {
    CURRENT_UPLOAD_LIMITS
        .try_with(|limits| *limits)
        .unwrap_or_default()
}

/// `UploadedFile` cannot be deserialized from JSON — it is populated
/// exclusively via multipart extraction (`FromMultipart`). This impl
/// exists to satisfy `Deserialize` bounds on structs that contain both
/// text fields and file fields.
impl<'de> Deserialize<'de> for UploadedFile {
    fn deserialize<D>(_deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Err(de::Error::custom(
            "UploadedFile cannot be deserialized from JSON; use multipart/form-data",
        ))
    }
}

impl UploadedFile {
    /// Returns a display-safe filename for upload metadata.
    ///
    /// This strips path components for both Unix and Windows separators,
    /// removes control characters, trims unsafe wrapper whitespace/quotes,
    /// caps length, and falls back to `upload` when no safe name remains.
    pub fn sanitize_name(name: &str) -> String {
        sanitize_filename(name, FALLBACK_UPLOAD_FILENAME, MAX_UPLOAD_FILENAME_BYTES)
    }

    /// Generates a UUIDv7-based filename, preserving a safe normalized extension.
    pub fn generate_storage_name(&self) -> String {
        let uuid = uuid::Uuid::now_v7().to_string();
        match self.original_extension() {
            Some(ext) => format!("{uuid}.{ext}"),
            None => uuid,
        }
    }

    pub async fn from_multipart_field(
        field_name: String,
        field: Field<'_>,
        counters: &mut UploadCounters,
    ) -> Result<Option<Self>> {
        uploaded_file_from_multipart_field(field_name, field, current_upload_limits(), counters)
            .await
    }

    /// Extracts and normalizes the file extension from the original filename.
    ///
    /// Returns `None` if there is no extension, or if the extension contains
    /// dangerous characters (path separators) or exceeds 32 characters.
    pub fn original_extension(&self) -> Option<String> {
        self.original_name
            .as_ref()
            .map(|name| Self::sanitize_name(name))
            .and_then(|name| safe_extension_from_name(&name))
    }

    /// Normalizes a user-provided filename by stripping any path components,
    /// keeping only the final file name segment.
    pub fn normalize_name(name: &str) -> String {
        Self::sanitize_name(name)
    }

    /// Builds a storage path from a directory and filename.
    fn storage_path(dir: &str, name: &str) -> String {
        format!("{dir}/{name}")
    }

    /// Stores the uploaded file on the default disk in the given directory.
    ///
    /// Generates a unique filename (UUIDv7-based) preserving the original extension.
    /// The temporary file is retained for reuse; prefer [`Self::store_and_cleanup`]
    /// when this is the final use of the upload.
    pub async fn store(&self, app: &AppContext, dir: &str) -> Result<StoredFile> {
        let storage = app.resolve::<StorageManager>()?;
        let disk = storage.default_disk()?;
        let name = self.generate_storage_name();
        let path = Self::storage_path(dir, &name);
        disk.put_file(&path, &self.temp_path, self.content_type.as_deref())
            .await
    }

    /// Stores the uploaded file on a named disk in the given directory.
    ///
    /// Generates a unique filename (UUIDv7-based) preserving the original extension.
    /// The temporary file is retained for reuse; prefer
    /// [`Self::store_on_and_cleanup`] when this is the final use of the upload.
    pub async fn store_on(
        &self,
        app: &AppContext,
        disk_name: &str,
        dir: &str,
    ) -> Result<StoredFile> {
        let storage = app.resolve::<StorageManager>()?;
        let disk = storage.disk(disk_name)?;
        let name = self.generate_storage_name();
        let path = Self::storage_path(dir, &name);
        disk.put_file(&path, &self.temp_path, self.content_type.as_deref())
            .await
    }

    /// Stores the uploaded file on the default disk with a custom filename.
    ///
    /// The name is normalized (path components stripped) before storage.
    /// The temporary file is retained for reuse; prefer
    /// [`Self::store_as_and_cleanup`] when this is the final use of the upload.
    pub async fn store_as(&self, app: &AppContext, dir: &str, name: &str) -> Result<StoredFile> {
        let storage = app.resolve::<StorageManager>()?;
        let disk = storage.default_disk()?;
        let safe_name = Self::normalize_name(name);
        let path = Self::storage_path(dir, &safe_name);
        disk.put_file(&path, &self.temp_path, self.content_type.as_deref())
            .await
    }

    /// Stores the uploaded file on a named disk with a custom filename.
    ///
    /// The name is normalized (path components stripped) before storage.
    /// The temporary file is retained for reuse; prefer
    /// [`Self::store_as_on_and_cleanup`] when this is the final use of the upload.
    pub async fn store_as_on(
        &self,
        app: &AppContext,
        disk_name: &str,
        dir: &str,
        name: &str,
    ) -> Result<StoredFile> {
        let storage = app.resolve::<StorageManager>()?;
        let disk = storage.disk(disk_name)?;
        let safe_name = Self::normalize_name(name);
        let path = Self::storage_path(dir, &safe_name);
        disk.put_file(&path, &self.temp_path, self.content_type.as_deref())
            .await
    }

    /// Stores this upload on the default disk, then removes its Foundry-owned
    /// temporary file whether storage succeeds or fails.
    ///
    /// This consumes the handle to make final-use intent explicit. Foundry-owned
    /// temporary bytes are no longer reusable after the call; a non-Foundry
    /// `temp_path` is never removed. If storage and cleanup both fail, the storage
    /// error is returned as the primary error.
    pub async fn store_and_cleanup(self, app: &AppContext, dir: &str) -> Result<StoredFile> {
        let result = self.store(app, dir).await;
        self.finish_store_and_cleanup(result).await
    }

    /// Stores this upload on a named disk, then removes its Foundry-owned temporary
    /// file whether storage succeeds or fails.
    ///
    /// This consumes the handle to make final-use intent explicit. Foundry-owned
    /// temporary bytes are no longer reusable after the call; a non-Foundry
    /// `temp_path` is never removed. If storage and cleanup both fail, the storage
    /// error is returned as the primary error.
    pub async fn store_on_and_cleanup(
        self,
        app: &AppContext,
        disk_name: &str,
        dir: &str,
    ) -> Result<StoredFile> {
        let result = self.store_on(app, disk_name, dir).await;
        self.finish_store_and_cleanup(result).await
    }

    /// Stores this upload with a custom filename on the default disk, then removes
    /// its Foundry-owned temporary file whether storage succeeds or fails.
    ///
    /// This consumes the handle to make final-use intent explicit. Foundry-owned
    /// temporary bytes are no longer reusable after the call; a non-Foundry
    /// `temp_path` is never removed. If storage and cleanup both fail, the storage
    /// error is returned as the primary error.
    pub async fn store_as_and_cleanup(
        self,
        app: &AppContext,
        dir: &str,
        name: &str,
    ) -> Result<StoredFile> {
        let result = self.store_as(app, dir, name).await;
        self.finish_store_and_cleanup(result).await
    }

    /// Stores this upload with a custom filename on a named disk, then removes its
    /// Foundry-owned temporary file whether storage succeeds or fails.
    ///
    /// This consumes the handle to make final-use intent explicit. Foundry-owned
    /// temporary bytes are no longer reusable after the call; a non-Foundry
    /// `temp_path` is never removed. If storage and cleanup both fail, the storage
    /// error is returned as the primary error.
    pub async fn store_as_on_and_cleanup(
        self,
        app: &AppContext,
        disk_name: &str,
        dir: &str,
        name: &str,
    ) -> Result<StoredFile> {
        let result = self.store_as_on(app, disk_name, dir, name).await;
        self.finish_store_and_cleanup(result).await
    }

    async fn finish_store_and_cleanup(
        self,
        store_result: Result<StoredFile>,
    ) -> Result<StoredFile> {
        let cleanup_result = remove_foundry_upload_temp_path(&self.temp_path).await;

        match store_result {
            Err(store_error) => {
                if let Err(cleanup_error) = cleanup_result {
                    tracing::warn!(
                        target: "foundry.storage",
                        path = %self.temp_path.display(),
                        error = %cleanup_error,
                        "Failed to clean up uploaded temp file after storage failed"
                    );
                }
                Err(store_error)
            }
            Ok(stored) => {
                cleanup_result?;
                Ok(stored)
            }
        }
    }
}

pub async fn uploaded_file_from_multipart_field(
    field_name: String,
    field: Field<'_>,
    limits: UploadLimits,
    counters: &mut UploadCounters,
) -> Result<Option<UploadedFile>> {
    if field.file_name().is_none() {
        return Ok(None);
    }

    let next_count = counters.uploaded_files.saturating_add(1);
    if limits.max_upload_files > 0 && next_count > limits.max_upload_files {
        return Err(upload_too_many_files_error());
    }
    counters.uploaded_files = next_count;

    let original_name = field.file_name().map(UploadedFile::sanitize_name);
    let content_type = field.content_type().and_then(normalize_content_type);
    let temp_path = foundry_upload_temp_path();

    uploaded_file_from_multipart_field_at_path(
        field_name,
        original_name,
        content_type,
        field,
        limits,
        counters,
        temp_path,
    )
    .await
}

async fn uploaded_file_from_multipart_field_at_path(
    field_name: String,
    original_name: Option<String>,
    content_type: Option<String>,
    field: Field<'_>,
    limits: UploadLimits,
    counters: &mut UploadCounters,
    temp_path: PathBuf,
) -> Result<Option<UploadedFile>> {
    let size = stream_field_to_temp_file(field, &temp_path, limits, counters).await?;
    Ok(Some(UploadedFile {
        field_name,
        original_name,
        content_type,
        size,
        temp_path,
    }))
}

async fn stream_field_to_temp_file(
    mut field: Field<'_>,
    temp_path: &Path,
    limits: UploadLimits,
    counters: &mut UploadCounters,
) -> Result<u64> {
    if let Some(parent) = temp_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| Error::message(format!("temp directory error: {error}")))?;
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .await
        .map_err(|error| Error::message(format!("temp file error: {error}")))?;

    let mut file_size = 0u64;
    let result = async {
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|error| Error::message(format!("chunk error: {error}")))?
        {
            let chunk_size = chunk.len() as u64;
            file_size = file_size
                .checked_add(chunk_size)
                .ok_or_else(upload_too_large_error)?;
            counters.uploaded_bytes = counters
                .uploaded_bytes
                .checked_add(chunk_size)
                .ok_or_else(upload_too_large_error)?;

            if limits.max_upload_file_size_bytes > 0
                && file_size > limits.max_upload_file_size_bytes
            {
                return Err(upload_file_too_large_error());
            }
            if limits.max_upload_size_bytes > 0
                && counters.uploaded_bytes > limits.max_upload_size_bytes
            {
                return Err(upload_too_large_error());
            }

            file.write_all(&chunk)
                .await
                .map_err(|error| Error::message(format!("write error: {error}")))?;
        }

        Ok(file_size)
    }
    .await;

    if result.is_err() {
        drop(file);
        let _ = tokio::fs::remove_file(temp_path).await;
    }

    result
}

pub async fn prune_stale_upload_temp_files(retention_seconds: u64, batch_size: u64) -> Result<u64> {
    prune_stale_upload_temp_files_in_dir(&foundry_upload_temp_dir(), retention_seconds, batch_size)
        .await
}

pub async fn remove_uploaded_temp_file(file: &UploadedFile) -> bool {
    remove_foundry_upload_temp_path(&file.temp_path)
        .await
        .unwrap_or(false)
}

pub async fn cleanup_uploaded_files<'a, I>(files: I)
where
    I: IntoIterator<Item = &'a UploadedFile>,
{
    for file in files {
        let _ = remove_uploaded_temp_file(file).await;
    }
}

async fn prune_stale_upload_temp_files_in_dir(
    dir: &Path,
    retention_seconds: u64,
    batch_size: u64,
) -> Result<u64> {
    if retention_seconds == 0 || batch_size == 0 {
        return Ok(0);
    }

    if !tokio::fs::try_exists(dir).await.map_err(Error::other)? {
        return Ok(0);
    }

    let mut deleted = 0u64;
    let now = SystemTime::now();
    let retention = Duration::from_secs(retention_seconds);
    let mut entries = tokio::fs::read_dir(dir).await.map_err(Error::other)?;

    while let Some(entry) = entries.next_entry().await.map_err(Error::other)? {
        if deleted >= batch_size {
            break;
        }

        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(UPLOAD_TEMP_PREFIX) {
            continue;
        }

        let metadata = match entry.metadata().await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now.duration_since(modified).unwrap_or_default() < retention {
            continue;
        }

        match tokio::fs::remove_file(entry.path()).await {
            Ok(()) => deleted += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::other(error)),
        }
    }

    Ok(deleted)
}

async fn remove_foundry_upload_temp_path(path: &Path) -> Result<bool> {
    if !is_foundry_upload_temp_path(path) {
        return Ok(false);
    }

    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::message(format!(
            "failed to remove uploaded temp file `{}`: {error}",
            path.display()
        ))),
    }
}

fn upload_too_large_error() -> Error {
    Error::http_with_code(413, "Upload is too large", "upload_too_large")
}

fn upload_file_too_large_error() -> Error {
    Error::http_with_code(413, "Uploaded file is too large", "uploaded_file_too_large")
}

fn upload_too_many_files_error() -> Error {
    Error::http_with_code(413, "Too many uploaded files", "too_many_uploaded_files")
}

fn normalize_content_type(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    Some(value)
}

fn is_foundry_upload_temp_path(path: &Path) -> bool {
    path.parent()
        .is_some_and(|parent| parent == foundry_upload_temp_dir())
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(UPLOAD_TEMP_PREFIX))
}

pub(crate) fn foundry_upload_temp_dir() -> PathBuf {
    std::env::temp_dir().join(UPLOAD_TEMP_DIR)
}

fn foundry_upload_temp_path() -> PathBuf {
    foundry_upload_temp_dir().join(format!("{UPLOAD_TEMP_PREFIX}{}", uuid::Uuid::now_v7()))
}

pub(crate) fn invalid_multipart_response(status: u16, error: impl std::fmt::Display) -> Response {
    Error::http_with_code(
        status,
        format!("Invalid multipart request: {error}"),
        "invalid_multipart_request",
    )
    .into_response()
}

/// Extracts the first file field from a multipart request.
///
/// Returns `400 Bad Request` if no file field is found in the request body.
impl<S> FromRequest<S> for UploadedFile
where
    S: Send + Sync,
    AppContext: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let app = AppContext::from_ref(state);
        let limits = UploadLimits::from_app(&app);

        let mut multipart = axum::extract::Multipart::from_request(req, state)
            .await
            .map_err(|rejection| {
                invalid_multipart_response(rejection.status().as_u16(), rejection)
            })?;

        let mut counters = UploadCounters::default();
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|error| invalid_multipart_response(400, error))?
        {
            let field_name = field.name().unwrap_or("").to_string();

            if let Some(file) =
                uploaded_file_from_multipart_field(field_name, field, limits, &mut counters)
                    .await
                    .map_err(IntoResponse::into_response)?
            {
                return Ok(file);
            }
        }

        Err(Error::http_with_code(400, "No file uploaded", "missing_uploaded_file").into_response())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::body::Body;
    use axum::extract::FromRequest as _;
    use axum::http::{header, Request, StatusCode};
    use axum::routing::post;
    use serde_json::Value;
    use tower::ServiceExt as _;

    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::validation::RuleRegistry;

    use super::*;

    async fn accept_upload(file: UploadedFile) -> String {
        file.original_name.unwrap_or_default()
    }

    fn app_with_storage_config(config: &str) -> AppContext {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("foundry.toml"), config).unwrap();
        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        AppContext::new(Container::new(), config, RuleRegistry::new()).unwrap()
    }

    fn multipart_request(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=foundry-test",
            )
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn make_upload(original_name: Option<&str>) -> UploadedFile {
        UploadedFile {
            field_name: "file".to_string(),
            original_name: original_name.map(|s| s.to_string()),
            content_type: Some("image/png".to_string()),
            size: 1024,
            temp_path: PathBuf::from("/tmp/upload123"),
        }
    }

    fn make_upload_at(temp_path: PathBuf, original_name: &str, size: u64) -> UploadedFile {
        UploadedFile {
            field_name: "file".to_string(),
            original_name: Some(original_name.to_string()),
            content_type: Some("text/plain".to_string()),
            size,
            temp_path,
        }
    }

    fn foundry_owned_temp_file(contents: &[u8]) -> PathBuf {
        let path = foundry_upload_temp_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    async fn app_with_local_storage(root: &Path) -> AppContext {
        let root = toml::Value::String(root.to_string_lossy().into_owned());
        let app = app_with_storage_config(&format!(
            r#"
            [storage]
            default = "local"

            [storage.disks.local]
            driver = "local"
            root = {root}
            "#
        ));
        let storage = StorageManager::from_config(app.config(), HashMap::new())
            .await
            .unwrap();
        app.container().singleton(storage).unwrap();
        app
    }

    #[test]
    fn generate_storage_name_produces_uuid_with_extension() {
        let upload = make_upload(Some("photo.JPG"));
        let name = upload.generate_storage_name();

        // UUIDv7 format: 8-4-4-4-12 hex chars, then .jpg
        let parts: Vec<&str> = name.split('.').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1], "jpg"); // normalized to lowercase

        let uuid_part = parts[0];
        let segments: Vec<&str> = uuid_part.split('-').collect();
        assert_eq!(segments.len(), 5);
        assert_eq!(segments[0].len(), 8);
        assert_eq!(segments[1].len(), 4);
        assert_eq!(segments[2].len(), 4);
        assert_eq!(segments[3].len(), 4);
        assert_eq!(segments[4].len(), 12);
    }

    #[test]
    fn generate_storage_name_without_original_name_is_just_uuid() {
        let upload = make_upload(None);
        let name = upload.generate_storage_name();

        // No extension — just the UUID
        assert!(!name.contains('.'));
        let segments: Vec<&str> = name.split('-').collect();
        assert_eq!(segments.len(), 5);
        assert_eq!(segments[0].len(), 8);
    }

    #[test]
    fn original_extension_normalizes_to_lowercase() {
        let upload = make_upload(Some("document.PDF"));
        assert_eq!(upload.original_extension(), Some("pdf".to_string()));
    }

    #[test]
    fn original_extension_strips_dangerous_slash() {
        let upload = make_upload(Some("file.sh/evil"));
        assert_eq!(upload.original_extension(), None);
    }

    #[test]
    fn original_extension_strips_dangerous_backslash() {
        let upload = make_upload(Some("file.exe\\evil"));
        assert_eq!(upload.original_extension(), None);
    }

    #[test]
    fn original_extension_returns_none_for_no_extension() {
        let upload = make_upload(Some("README"));
        assert_eq!(upload.original_extension(), None);
    }

    #[test]
    fn original_extension_returns_none_for_none_original_name() {
        let upload = make_upload(None);
        assert_eq!(upload.original_extension(), None);
    }

    #[test]
    fn original_extension_rejects_overly_long_extension() {
        let long_ext = "a".repeat(33);
        let upload = make_upload(Some(&format!("file.{long_ext}")));
        assert_eq!(upload.original_extension(), None);
    }

    #[test]
    fn original_extension_accepts_max_length_extension() {
        let ext = "a".repeat(32);
        let upload = make_upload(Some(&format!("file.{ext}")));
        assert_eq!(upload.original_extension(), Some(ext));
    }

    #[test]
    fn sanitize_name_handles_paths_controls_quotes_and_fallback() {
        assert_eq!(UploadedFile::sanitize_name("/etc/passwd"), "passwd");
        assert_eq!(
            UploadedFile::sanitize_name(r#"C:\Users\admin\avatar.JPG"#),
            "avatar.JPG"
        );
        assert_eq!(
            UploadedFile::sanitize_name(" \" report\u{0000}\u{001f}.pdf \" "),
            "report.pdf"
        );
        assert_eq!(
            UploadedFile::sanitize_name("////"),
            FALLBACK_UPLOAD_FILENAME
        );
        assert_eq!(UploadedFile::sanitize_name("照片.png"), "照片.png");
    }

    #[test]
    fn sanitize_name_caps_long_names_and_preserves_extension() {
        let input = format!("{}.png", "a".repeat(400));
        let name = UploadedFile::sanitize_name(&input);

        assert!(name.len() <= MAX_UPLOAD_FILENAME_BYTES);
        assert!(name.ends_with(".png"));
    }

    #[test]
    fn normalize_name_strips_path_components() {
        // Unix-style paths are stripped by std::path::Path::file_name
        assert_eq!(UploadedFile::normalize_name("/etc/passwd"), "passwd");
        assert_eq!(
            UploadedFile::normalize_name("subdir/photo.jpg"),
            "photo.jpg"
        );
        assert_eq!(
            UploadedFile::normalize_name(r#"C:\tmp\photo.jpg"#),
            "photo.jpg"
        );
        assert_eq!(UploadedFile::normalize_name("simple.txt"), "simple.txt");
    }

    #[test]
    fn normalize_name_returns_input_for_bare_name() {
        assert_eq!(UploadedFile::normalize_name("photo.jpg"), "photo.jpg");
    }

    #[test]
    fn storage_path_combines_dir_and_name() {
        let path = UploadedFile::storage_path("avatars", "uuid.png");
        assert_eq!(path, "avatars/uuid.png");
    }

    #[tokio::test]
    async fn consuming_store_removes_foundry_owned_temp_file_after_success() {
        let root = tempfile::tempdir().unwrap();
        let app = app_with_local_storage(root.path()).await;
        let temp_path = foundry_owned_temp_file(b"stored contents");
        let upload = make_upload_at(temp_path.clone(), "report.txt", 15);

        let stored = upload
            .store_as_and_cleanup(&app, "reports", "final.txt")
            .await
            .unwrap();

        assert_eq!(stored.path, "reports/final.txt");
        assert_eq!(
            std::fs::read(root.path().join("reports/final.txt")).unwrap(),
            b"stored contents"
        );
        assert!(!temp_path.exists());
    }

    #[tokio::test]
    async fn consuming_store_removes_foundry_owned_temp_file_after_storage_error() {
        let app = app_with_storage_config("[storage]\n");
        let temp_path = foundry_owned_temp_file(b"cleanup on error");
        let upload = make_upload_at(temp_path.clone(), "report.txt", 16);

        let error = upload.store_and_cleanup(&app, "reports").await.unwrap_err();

        assert!(error.to_string().contains("StorageManager"));
        assert!(error.to_string().contains("not registered"));
        assert!(!temp_path.exists());
    }

    #[tokio::test]
    async fn consuming_store_preserves_storage_error_when_cleanup_also_fails() {
        let temp_path = foundry_upload_temp_path();
        std::fs::create_dir_all(&temp_path).unwrap();
        let upload = make_upload_at(temp_path.clone(), "report.txt", 0);

        let error = upload
            .finish_store_and_cleanup(Err(Error::message("primary storage error")))
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "primary storage error");
        assert!(temp_path.is_dir());
        std::fs::remove_dir(temp_path).unwrap();
    }

    #[tokio::test]
    async fn consuming_store_reports_cleanup_error_after_storage_succeeds() {
        let temp_path = foundry_upload_temp_path();
        std::fs::create_dir_all(&temp_path).unwrap();
        let upload = make_upload_at(temp_path.clone(), "report.txt", 0);
        let stored = StoredFile {
            disk: "local".to_string(),
            path: "reports/report.txt".to_string(),
            name: "report.txt".to_string(),
            size: 0,
            content_type: Some("text/plain".to_string()),
            url: None,
        };

        let error = upload
            .finish_store_and_cleanup(Ok(stored))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to remove uploaded temp file"));
        assert!(temp_path.is_dir());
        std::fs::remove_dir(temp_path).unwrap();
    }

    #[tokio::test]
    async fn borrowed_store_keeps_temp_file_reusable() {
        let root = tempfile::tempdir().unwrap();
        let app = app_with_local_storage(root.path()).await;
        let temp_path = foundry_owned_temp_file(b"reusable contents");
        let upload = make_upload_at(temp_path.clone(), "report.txt", 17);

        upload.store_as(&app, "reports", "first.txt").await.unwrap();
        upload
            .store_as(&app, "reports", "second.txt")
            .await
            .unwrap();

        assert!(temp_path.exists());
        assert_eq!(
            std::fs::read(root.path().join("reports/first.txt")).unwrap(),
            b"reusable contents"
        );
        assert_eq!(
            std::fs::read(root.path().join("reports/second.txt")).unwrap(),
            b"reusable contents"
        );
        assert!(remove_uploaded_temp_file(&upload).await);
    }

    #[tokio::test]
    async fn consuming_store_does_not_remove_non_foundry_temp_path() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let app = app_with_local_storage(root.path()).await;
        let temp_path = source.path().join("external.txt");
        std::fs::write(&temp_path, b"external contents").unwrap();
        let upload = make_upload_at(temp_path.clone(), "external.txt", 17);

        upload
            .store_as_and_cleanup(&app, "reports", "external.txt")
            .await
            .unwrap();

        assert!(temp_path.exists());
    }

    #[test]
    fn upload_limits_resolve_from_storage_config() {
        let config = StorageConfig {
            max_upload_size_bytes: 1024,
            max_upload_file_size_bytes: 512,
            max_upload_files: 3,
            ..StorageConfig::default()
        };

        assert_eq!(
            UploadLimits::from_config(&config),
            UploadLimits {
                max_upload_size_bytes: 1024,
                max_upload_file_size_bytes: 512,
                max_upload_files: 3,
            }
        );
    }

    #[tokio::test]
    async fn prune_stale_upload_temp_files_is_bounded_to_foundry_uploads() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join(format!("{UPLOAD_TEMP_PREFIX}stale"));
        let fresh = dir.path().join(format!("{UPLOAD_TEMP_PREFIX}fresh"));
        let unrelated = dir.path().join("other-temp-file");

        std::fs::write(&stale, b"old").unwrap();
        std::fs::write(&fresh, b"new").unwrap();
        std::fs::write(&unrelated, b"keep").unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        std::fs::write(&fresh, b"newer").unwrap();

        let deleted = prune_stale_upload_temp_files_in_dir(dir.path(), 1, 10)
            .await
            .unwrap();

        assert_eq!(deleted, 1);
        assert!(!stale.exists());
        assert!(fresh.exists());
        assert!(unrelated.exists());
    }

    #[tokio::test]
    async fn uploaded_file_uses_private_foundry_temp_directory() {
        let request = multipart_request(concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "abc\r\n",
            "--foundry-test--\r\n"
        ));
        let mut multipart = axum::extract::Multipart::from_request(request, &())
            .await
            .unwrap();
        let field = multipart.next_field().await.unwrap().unwrap();
        let mut counters = UploadCounters::default();

        let upload = uploaded_file_from_multipart_field(
            "file".to_string(),
            field,
            UploadLimits::default(),
            &mut counters,
        )
        .await
        .unwrap()
        .unwrap();

        let temp_dir = foundry_upload_temp_dir();
        assert_eq!(upload.temp_path.parent(), Some(temp_dir.as_path()));
        assert!(is_foundry_upload_temp_path(&upload.temp_path));
        assert!(remove_uploaded_temp_file(&upload).await);
    }

    #[tokio::test]
    async fn stream_field_to_temp_file_uses_create_new() {
        let request = multipart_request(concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "abcdef\r\n",
            "--foundry-test--\r\n"
        ));
        let mut multipart = axum::extract::Multipart::from_request(request, &())
            .await
            .unwrap();
        let field = multipart.next_field().await.unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join(format!(
            "{UPLOAD_TEMP_PREFIX}existing-{}",
            uuid::Uuid::now_v7()
        ));
        std::fs::write(&temp_path, b"existing").unwrap();
        let mut counters = UploadCounters::default();

        let error = uploaded_file_from_multipart_field_at_path(
            "file".to_string(),
            Some("a.txt".to_string()),
            Some("text/plain".to_string()),
            field,
            UploadLimits::default(),
            &mut counters,
            temp_path.clone(),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("temp file error"));
        assert_eq!(std::fs::read(&temp_path).unwrap(), b"existing");
    }

    #[tokio::test]
    async fn remove_uploaded_temp_file_only_removes_private_foundry_uploads() {
        let temp_dir = foundry_upload_temp_dir();
        std::fs::create_dir_all(&temp_dir).unwrap();
        let owned_path = temp_dir.join(format!(
            "{UPLOAD_TEMP_PREFIX}owned-{}",
            uuid::Uuid::now_v7()
        ));
        let unowned_path = std::env::temp_dir().join(format!(
            "{UPLOAD_TEMP_PREFIX}unowned-{}",
            uuid::Uuid::now_v7()
        ));
        std::fs::write(&owned_path, b"owned").unwrap();
        std::fs::write(&unowned_path, b"unowned").unwrap();

        let owned = UploadedFile {
            temp_path: owned_path.clone(),
            ..make_upload(Some("owned.txt"))
        };
        let unowned = UploadedFile {
            temp_path: unowned_path.clone(),
            ..make_upload(Some("unowned.txt"))
        };

        assert!(remove_uploaded_temp_file(&owned).await);
        assert!(!remove_uploaded_temp_file(&unowned).await);
        assert!(!owned_path.exists());
        assert!(unowned_path.exists());
        std::fs::remove_file(unowned_path).unwrap();
    }

    #[tokio::test]
    async fn multipart_limit_error_removes_known_temp_file() {
        let request = multipart_request(concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "abcdef\r\n",
            "--foundry-test--\r\n"
        ));
        let mut multipart = axum::extract::Multipart::from_request(request, &())
            .await
            .unwrap();
        let field = multipart.next_field().await.unwrap().unwrap();
        let temp_path =
            std::env::temp_dir().join(format!("{UPLOAD_TEMP_PREFIX}test-{}", uuid::Uuid::now_v7()));
        let mut counters = UploadCounters::default();

        let error = uploaded_file_from_multipart_field_at_path(
            "file".to_string(),
            Some("a.txt".to_string()),
            Some("text/plain".to_string()),
            field,
            UploadLimits {
                max_upload_file_size_bytes: 3,
                ..UploadLimits::default()
            },
            &mut counters,
            temp_path.clone(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.to_string(), "Uploaded file is too large");
        assert!(!temp_path.exists());
    }

    #[tokio::test]
    async fn uploaded_file_returns_json_error_when_missing() {
        let app = app_with_storage_config("[storage]\n");
        let router = axum::Router::new()
            .route("/", post(accept_upload))
            .with_state(app);
        let body = concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"title\"\r\n\r\n",
            "hello\r\n",
            "--foundry-test--\r\n"
        );

        let response = router.oneshot(multipart_request(body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "No file uploaded");
        assert_eq!(json["error_code"], "missing_uploaded_file");
    }

    #[tokio::test]
    async fn uploaded_file_sanitizes_multipart_filename_on_extraction() {
        let app = app_with_storage_config("[storage]\n");
        let router = axum::Router::new()
            .route("/", post(accept_upload))
            .with_state(app);
        let body = concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"C:\\\\tmp\\\\photo.JPG\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "hello\r\n",
            "--foundry-test--\r\n"
        );

        let response = router.oneshot(multipart_request(body)).await.unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        assert_eq!(std::str::from_utf8(&body).unwrap(), "photo.JPG");
    }

    #[tokio::test]
    async fn uploaded_file_returns_json_error_when_file_size_is_exceeded() {
        let app = app_with_storage_config(
            r#"
            [storage]
            max_upload_file_size_bytes = 3
            "#,
        );
        let router = axum::Router::new()
            .route("/", post(accept_upload))
            .with_state(app);
        let body = concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "abcdef\r\n",
            "--foundry-test--\r\n"
        );

        let response = router.oneshot(multipart_request(body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Uploaded file is too large");
        assert_eq!(json["error_code"], "uploaded_file_too_large");
    }
}
