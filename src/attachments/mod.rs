mod callback;
mod orphans;

use std::collections::{BTreeSet, HashMap};
#[cfg(test)]
use std::future::Future;
use std::io::Cursor;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::database::extensions::{
    current_extension_scope, uuid_array_from_ids, AnyModelExtension, ModelExtensionLoader,
};
use crate::database::{DbType, DbValue, OrderBy, Query, QueryExecutor};
use crate::foundation::{AppContext, AppTransaction, Error, Result};
use crate::imaging::{ImageDecodeLimitViolation, ImageDecodeLimits, ImageFormat};
use crate::storage::UploadedFile;
use crate::support::DateTime;

const LOCALIZED_COLLECTION_SEPARATOR: &str = ":";
const ATTACHMENTS_TABLE: &str = "attachments";

pub(crate) use orphans::{
    audit_attachment_orphans_with_lock, builtin_cli_registrar, AttachmentOrphanOptions,
};

/// A file attachment record from the `attachments` table.
#[derive(Clone, Debug)]
pub struct Attachment {
    pub id: String,
    pub attachable_type: String,
    pub attachable_id: String,
    pub collection: String,
    pub disk: String,
    pub path: String,
    pub name: String,
    pub original_name: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
    pub sort_order: i32,
    pub custom_properties: serde_json::Value,
}

impl Attachment {
    /// Start building an attachment upload pipeline.
    pub fn upload(file: UploadedFile) -> AttachmentUploadBuilder {
        AttachmentUploadBuilder {
            file,
            collection: "default".to_string(),
            disk: None,
            image_transforms: Vec::new(),
            output_format: None,
            quality: None,
            allow_upscale: true,
            require_image: false,
        }
    }

    pub fn is_image(&self) -> bool {
        self.mime_type
            .as_deref()
            .is_some_and(|m| m.starts_with("image/"))
    }

    pub fn is_video(&self) -> bool {
        self.mime_type
            .as_deref()
            .is_some_and(|m| m.starts_with("video/"))
    }

    pub async fn update_custom_properties(
        app: &AppContext,
        attachment_id: &str,
        custom_properties: serde_json::Value,
    ) -> Result<u64> {
        let db = app.database()?;
        Self::update_custom_properties_with(&*db, attachment_id, custom_properties).await
    }

    pub async fn update_custom_properties_with<E>(
        executor: &E,
        attachment_id: &str,
        custom_properties: serde_json::Value,
    ) -> Result<u64>
    where
        E: QueryExecutor,
    {
        Query::update_table(ATTACHMENTS_TABLE)
            .value("custom_properties", DbValue::Json(custom_properties))
            .where_eq("id", parse_attachment_uuid(attachment_id, "attachment_id")?)
            .execute(executor)
            .await
    }

    pub fn is_audio(&self) -> bool {
        self.mime_type
            .as_deref()
            .is_some_and(|m| m.starts_with("audio/"))
    }

    pub fn is_document(&self) -> bool {
        self.mime_type.as_deref().is_some_and(|m| {
            matches!(
                m,
                "application/pdf"
                    | "application/msword"
                    | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    | "application/vnd.ms-excel"
                    | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                    | "text/csv"
                    | "text/plain"
            )
        })
    }

    pub fn extension(&self) -> Option<&str> {
        self.name
            .rsplit('.')
            .next()
            .filter(|ext| ext.len() < 10 && !ext.is_empty())
    }

    pub fn human_size(&self) -> String {
        let size = self.size as f64;
        if size < 1024.0 {
            return format!("{} B", self.size);
        }
        if size < 1024.0 * 1024.0 {
            return format!("{:.1} KB", size / 1024.0);
        }
        if size < 1024.0 * 1024.0 * 1024.0 {
            return format!("{:.1} MB", size / (1024.0 * 1024.0));
        }
        format!("{:.1} GB", size / (1024.0 * 1024.0 * 1024.0))
    }

    pub async fn url(&self, app: &AppContext) -> Result<String> {
        let storage = app.storage()?;
        let disk = storage.disk(&self.disk)?;
        disk.url(&self.path).await
    }

    pub async fn temporary_url(&self, app: &AppContext, expires_at: DateTime) -> Result<String> {
        let storage = app.storage()?;
        let disk = storage.disk(&self.disk)?;
        disk.temporary_url(&self.path, expires_at).await
    }

    /// Load this attachment's file into the image processing module.
    pub async fn image(&self, app: &AppContext) -> Result<crate::imaging::ImageProcessor> {
        let storage = app.storage()?;
        let disk = storage.disk(&self.disk)?;
        let bytes = disk.get(&self.path).await?;
        let limits = app.config().storage()?.image_decode_limits();
        crate::imaging::ImageProcessor::process_bytes_with_limits(bytes, limits, Ok).await
    }
}

/// Build the concrete attachment collection name for a locale-specific asset.
///
/// This keeps localized assets in the existing `attachments.collection` column
/// without adding another table or duplicating locale configuration.
pub fn localized_attachment_collection(collection: &str, locale: &str) -> String {
    format!(
        "{}{}{}",
        collection.trim(),
        LOCALIZED_COLLECTION_SEPARATOR,
        locale.trim()
    )
}

/// Return the loaded i18n locales used by localized attachment helpers.
///
/// Locale folders under the configured i18n resource path are the source of
/// truth, matching `I18nManager::locale_list()`.
pub fn available_attachment_locales(app: &AppContext) -> Result<Vec<String>> {
    let manager = app.i18n().map_err(|_| {
        Error::http_with_code(
            400,
            "localized attachments require i18n to be configured",
            "i18n_not_configured",
        )
    })?;
    let mut locales = manager
        .locale_list()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    locales.sort();
    Ok(locales)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentImageResize {
    Exact { width: u32, height: u32 },
    Fit { max_width: u32, max_height: u32 },
    Fill { width: u32, height: u32 },
}

impl AttachmentImageResize {
    fn target_dimensions(self) -> (u32, u32) {
        match self {
            Self::Exact { width, height } | Self::Fill { width, height } => (width, height),
            Self::Fit {
                max_width,
                max_height,
            } => (max_width, max_height),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentImagePolicy {
    pub resize: Option<AttachmentImageResize>,
    pub format: Option<ImageFormat>,
    pub quality: Option<u8>,
    pub upscale: bool,
}

impl Default for AttachmentImagePolicy {
    fn default() -> Self {
        Self {
            resize: None,
            format: None,
            quality: None,
            upscale: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentSpecKind {
    File,
    Image,
}

pub struct AttachmentSpec<M> {
    collection: String,
    kind: AttachmentSpecKind,
    single: bool,
    image_policy: Option<AttachmentImagePolicy>,
    hooks: Vec<Arc<dyn AttachmentSpecHook<M>>>,
    _model: PhantomData<fn() -> M>,
}

impl<M> Clone for AttachmentSpec<M> {
    fn clone(&self) -> Self {
        Self {
            collection: self.collection.clone(),
            kind: self.kind,
            single: self.single,
            image_policy: self.image_policy,
            hooks: self.hooks.clone(),
            _model: PhantomData,
        }
    }
}

impl<M> AttachmentSpec<M> {
    pub fn file(collection: impl Into<String>) -> Self {
        Self {
            collection: collection.into(),
            kind: AttachmentSpecKind::File,
            single: false,
            image_policy: None,
            hooks: Vec::new(),
            _model: PhantomData,
        }
    }

    pub fn image(collection: impl Into<String>) -> Self {
        Self {
            collection: collection.into(),
            kind: AttachmentSpecKind::Image,
            single: false,
            image_policy: Some(AttachmentImagePolicy::default()),
            hooks: Vec::new(),
            _model: PhantomData,
        }
    }

    pub fn single(mut self) -> Self {
        self.single = true;
        self
    }

    pub fn resize_exact(mut self, width: u32, height: u32) -> Self {
        self.image_policy_mut().resize = Some(AttachmentImageResize::Exact { width, height });
        self
    }

    pub fn resize_to_fit(mut self, max_width: u32, max_height: u32) -> Self {
        self.image_policy_mut().resize = Some(AttachmentImageResize::Fit {
            max_width,
            max_height,
        });
        self
    }

    pub fn resize_to_fill(mut self, width: u32, height: u32) -> Self {
        self.image_policy_mut().resize = Some(AttachmentImageResize::Fill { width, height });
        self
    }

    pub fn format(mut self, format: ImageFormat) -> Self {
        self.image_policy_mut().format = Some(format);
        self
    }

    pub fn quality(mut self, quality: u8) -> Self {
        self.image_policy_mut().quality = Some(quality.clamp(1, 100));
        self
    }

    pub fn upscale(mut self, upscale: bool) -> Self {
        self.image_policy_mut().upscale = upscale;
        self
    }

    pub fn hook<H>(mut self, hook: H) -> Self
    where
        M: Send + Sync,
        H: AttachmentSpecHook<M> + 'static,
    {
        self.hooks.push(Arc::new(hook));
        self
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    pub fn kind(&self) -> AttachmentSpecKind {
        self.kind
    }

    pub fn is_single(&self) -> bool {
        self.single
    }

    pub fn image_policy(&self) -> Option<AttachmentImagePolicy> {
        self.image_policy
    }

    fn hooks(&self) -> &[Arc<dyn AttachmentSpecHook<M>>] {
        &self.hooks
    }

    fn image_policy_mut(&mut self) -> &mut AttachmentImagePolicy {
        self.kind = AttachmentSpecKind::Image;
        self.image_policy
            .get_or_insert_with(AttachmentImagePolicy::default)
    }
}

pub struct AttachmentBeforeStoreContext<'a, M> {
    pub app: &'a AppContext,
    pub model: &'a M,
    pub spec: &'a AttachmentSpec<M>,
    pub collection: &'a str,
    pub locale: Option<&'a str>,
    pub file: &'a UploadedFile,
}

pub struct AttachmentAfterStoreContext<'a, M> {
    pub app: &'a AppContext,
    pub model: &'a M,
    pub spec: &'a AttachmentSpec<M>,
    pub collection: &'a str,
    pub locale: Option<&'a str>,
    pub file: &'a UploadedFile,
    pub attachment: &'a Attachment,
}

#[async_trait]
pub trait AttachmentSpecHook<M>: Send + Sync
where
    M: Send + Sync,
{
    async fn before_store(&self, _ctx: AttachmentBeforeStoreContext<'_, M>) -> Result<()> {
        Ok(())
    }

    async fn after_store(&self, _ctx: AttachmentAfterStoreContext<'_, M>) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Upload pipeline builder
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageTransform {
    Resize(u32, u32),
    ResizeToFit(u32, u32),
    ResizeToFill(u32, u32),
}

/// Chainable builder for uploading files as attachments.
///
/// ```ignore
/// Attachment::upload(file)
///     .collection("avatar")
///     .disk("s3")
///     .resize(800, 600)
///     .quality(80)
///     .store(&app, "users", &user.id.to_string())
///     .await?;
/// ```
pub struct AttachmentUploadBuilder {
    file: UploadedFile,
    collection: String,
    disk: Option<String>,
    image_transforms: Vec<ImageTransform>,
    output_format: Option<ImageFormat>,
    quality: Option<u8>,
    allow_upscale: bool,
    require_image: bool,
}

struct PreparedAttachment {
    attachment: Attachment,
    attachable_uuid: Uuid,
}

pub(crate) fn attachment_extension_loader<M>(collection: String) -> AnyModelExtension<M>
where
    M: HasAttachments + Send + Sync + 'static,
{
    Arc::new(AttachmentExtensionLoader {
        collection,
        _model: PhantomData,
    })
}

struct AttachmentExtensionLoader<M> {
    collection: String,
    _model: PhantomData<fn() -> M>,
}

#[async_trait]
impl<M> ModelExtensionLoader<M> for AttachmentExtensionLoader<M>
where
    M: HasAttachments + Send + Sync + 'static,
{
    async fn load(&self, executor: &dyn QueryExecutor, models: &[M]) -> Result<()> {
        let Some(scope) = current_extension_scope() else {
            return Ok(());
        };

        let ids = collect_attachment_ids(models)?;
        if ids.is_empty() {
            return Ok(());
        }

        let attachable_type = attachment_model_type::<M>()?;
        let missing_ids = scope.missing_attachment_ids(attachable_type, &self.collection, &ids);
        if missing_ids.is_empty() {
            return Ok(());
        }

        let rows =
            load_attachment_rows(executor, attachable_type, &self.collection, &missing_ids).await?;
        scope.store_attachments(attachable_type, &self.collection, &missing_ids, rows);
        Ok(())
    }
}

impl AttachmentUploadBuilder {
    pub fn collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }

    pub fn disk(mut self, disk: impl Into<String>) -> Self {
        self.disk = Some(disk.into());
        self
    }

    pub fn resize(mut self, width: u32, height: u32) -> Self {
        self.image_transforms
            .push(ImageTransform::Resize(width, height));
        self
    }

    pub fn resize_to_fit(mut self, max_width: u32, max_height: u32) -> Self {
        self.image_transforms
            .push(ImageTransform::ResizeToFit(max_width, max_height));
        self
    }

    pub fn resize_to_fill(mut self, width: u32, height: u32) -> Self {
        self.image_transforms
            .push(ImageTransform::ResizeToFill(width, height));
        self
    }

    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = Some(quality.clamp(1, 100));
        self
    }

    pub fn format(mut self, format: ImageFormat) -> Self {
        self.output_format = Some(format);
        self
    }

    pub fn upscale(mut self, upscale: bool) -> Self {
        self.allow_upscale = upscale;
        self
    }

    fn apply_spec<M>(mut self, spec: &AttachmentSpec<M>) -> Self {
        if let Some(policy) = spec.image_policy() {
            self.require_image = true;
            if let Some(resize) = policy.resize {
                match resize {
                    AttachmentImageResize::Exact { width, height } => {
                        self = self.resize(width, height)
                    }
                    AttachmentImageResize::Fit {
                        max_width,
                        max_height,
                    } => self = self.resize_to_fit(max_width, max_height),
                    AttachmentImageResize::Fill { width, height } => {
                        self = self.resize_to_fill(width, height)
                    }
                }
            }
            if let Some(format) = policy.format {
                self = self.format(format);
            }
            if let Some(quality) = policy.quality {
                self = self.quality(quality);
            }
            self = self.upscale(policy.upscale);
        }
        self
    }

    fn should_process_image(&self) -> bool {
        self.require_image
            || !self.image_transforms.is_empty()
            || self.output_format.is_some()
            || self.quality.is_some()
    }

    /// Store the file and create the attachment record.
    pub async fn store(
        self,
        app: &AppContext,
        attachable_type: &str,
        attachable_id: &str,
    ) -> Result<Attachment> {
        let prepared = self.prepare(app, attachable_type, attachable_id).await?;
        let transaction = match app.begin_transaction().await {
            Ok(transaction) => transaction,
            Err(error) => return cleanup_prepared_attachment(app, &prepared, error).await,
        };
        if let Err(error) = acquire_attachment_collection_lock(
            &transaction,
            attachable_type,
            prepared.attachable_uuid,
            &prepared.attachment.collection,
        )
        .await
        {
            return rollback_prepared_attachment(transaction, app, &prepared, error).await;
        }
        let sort_order = match next_attachment_sort_order(
            &transaction,
            attachable_type,
            prepared.attachable_uuid,
            &prepared.attachment.collection,
        )
        .await
        {
            Ok(sort_order) => sort_order,
            Err(error) => {
                return rollback_prepared_attachment(transaction, app, &prepared, error).await;
            }
        };
        let attachment = match prepared.insert_with(&transaction, sort_order).await {
            Ok(attachment) => attachment,
            Err(error) => {
                return rollback_prepared_attachment(transaction, app, &prepared, error).await;
            }
        };
        transaction.commit().await?;
        invalidate_attachment_cache(attachable_type, attachable_id, Some(&attachment.collection));
        Ok(attachment)
    }

    async fn prepare(
        self,
        app: &AppContext,
        attachable_type: &str,
        attachable_id: &str,
    ) -> Result<PreparedAttachment> {
        let attachable_uuid = parse_attachment_uuid(attachable_id, "attachable_id")?;
        let storage = app.storage()?;

        let disk_name = self.disk.clone().unwrap_or_else(|| {
            app.config()
                .storage()
                .map(|c| c.default.clone())
                .unwrap_or_else(|_| "local".to_string())
        });

        let dir = format!("attachments/{}/{}", attachable_type, self.collection);
        let original_name = self
            .file
            .original_name
            .as_deref()
            .map(UploadedFile::normalize_name);

        let processed_image = if self.should_process_image() {
            let bytes = tokio::fs::read(&self.file.temp_path)
                .await
                .map_err(Error::other)?;
            process_image_bytes_blocking(
                bytes,
                self.file.original_name.clone(),
                OwnedImageProcessingOptions {
                    transforms: self.image_transforms.clone(),
                    output_format: self.output_format,
                    quality: self.quality,
                    allow_upscale: self.allow_upscale,
                    require_image: self.require_image,
                    decode_limits: app.config().storage()?.image_decode_limits(),
                },
            )
            .await?
        } else {
            None
        };

        // Process image transforms if any, otherwise store directly.
        let (path, name, size, content_type) = if let Some(processed) = processed_image {
            let size = processed.bytes.len() as i64;
            let ext = processed.format.extension();
            let storage_name = format!("{}.{}", uuid::Uuid::now_v7(), ext);
            let path = format!("{}/{}", dir, storage_name);
            let ct = image_mime_type(processed.format).to_string();

            let disk = storage.disk(&disk_name)?;
            disk.put(&path, &processed.bytes).await?;

            (path, storage_name, size, Some(ct))
        } else {
            let stored = self.file.store_on(app, &disk_name, &dir).await?;
            let size = self.file.size as i64;
            let ct = self.file.content_type.clone().or(stored.content_type);
            (stored.path, stored.name, size, ct)
        };
        Ok(PreparedAttachment {
            attachable_uuid,
            attachment: Attachment {
                id: Uuid::now_v7().to_string(),
                attachable_type: attachable_type.to_string(),
                attachable_id: attachable_id.to_string(),
                collection: self.collection,
                disk: disk_name,
                path,
                name,
                original_name,
                mime_type: content_type,
                size,
                sort_order: 0,
                custom_properties: serde_json::json!({}),
            },
        })
    }
}

impl PreparedAttachment {
    async fn insert_with<E>(&self, executor: &E, sort_order: i32) -> Result<Attachment>
    where
        E: QueryExecutor,
    {
        let attachment_id = parse_attachment_uuid(&self.attachment.id, "id")?;
        Query::insert_into(ATTACHMENTS_TABLE)
            .values([
                ("id", DbValue::Uuid(attachment_id)),
                (
                    "attachable_type",
                    DbValue::Text(self.attachment.attachable_type.clone()),
                ),
                ("attachable_id", DbValue::Uuid(self.attachable_uuid)),
                (
                    "collection",
                    DbValue::Text(self.attachment.collection.clone()),
                ),
                ("disk", DbValue::Text(self.attachment.disk.clone())),
                ("path", DbValue::Text(self.attachment.path.clone())),
                ("name", DbValue::Text(self.attachment.name.clone())),
                ("original_name", opt_text(&self.attachment.original_name)),
                ("mime_type", opt_text(&self.attachment.mime_type)),
                ("size", DbValue::Int64(self.attachment.size)),
                ("sort_order", DbValue::Int32(sort_order)),
                (
                    "custom_properties",
                    DbValue::Json(self.attachment.custom_properties.clone()),
                ),
            ])
            .execute(executor)
            .await
            .map_err(|error| {
                Error::message(format!("failed to create attachment record: {error}"))
            })?;

        let mut attachment = self.attachment.clone();
        attachment.sort_order = sort_order;
        Ok(attachment)
    }

    async fn delete_file(&self, app: &AppContext) -> Result<()> {
        app.storage()?
            .disk(&self.attachment.disk)?
            .delete(&self.attachment.path)
            .await
    }
}

async fn acquire_attachment_collection_lock<E>(
    executor: &E,
    attachable_type: &str,
    attachable_id: Uuid,
    collection: &str,
) -> Result<()>
where
    E: QueryExecutor,
{
    let identity = serde_json::to_string(&(attachable_type, attachable_id.to_string(), collection))
        .map_err(Error::other)?;
    executor
        .raw_query(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))::text AS locked",
            &[DbValue::Text(identity)],
        )
        .await?;
    Ok(())
}

async fn next_attachment_sort_order<E>(
    executor: &E,
    attachable_type: &str,
    attachable_id: Uuid,
    collection: &str,
) -> Result<i32>
where
    E: QueryExecutor,
{
    let rows = executor
        .raw_query(
            "SELECT (COALESCE(MAX(sort_order), -1)::BIGINT + 1) AS next_sort_order FROM attachments WHERE attachable_type = $1 AND attachable_id = $2 AND collection = $3",
            &[
                DbValue::Text(attachable_type.to_string()),
                DbValue::Uuid(attachable_id),
                DbValue::Text(collection.to_string()),
            ],
        )
        .await?;
    let next: i64 = rows
        .first()
        .ok_or_else(|| Error::message("attachment sort query returned no row"))?
        .decode("next_sort_order")?;
    i32::try_from(next).map_err(|_| Error::message("attachment sort order exceeds i32 capacity"))
}

async fn rollback_prepared_attachment<T>(
    transaction: AppTransaction,
    app: &AppContext,
    prepared: &PreparedAttachment,
    error: Error,
) -> Result<T> {
    let rollback_error = transaction.rollback().await.err();
    let cleanup_error = prepared.delete_file(app).await.err();
    match (rollback_error, cleanup_error) {
        (None, None) => Err(error),
        (rollback, cleanup) => Err(Error::message(format!(
            "attachment operation failed: {error}; rollback cleanup: {}; storage cleanup: {}",
            rollback
                .map(|error| error.to_string())
                .unwrap_or_else(|| "ok".to_string()),
            cleanup
                .map(|error| error.to_string())
                .unwrap_or_else(|| "ok".to_string())
        ))),
    }
}

async fn cleanup_prepared_attachment<T>(
    app: &AppContext,
    prepared: &PreparedAttachment,
    error: Error,
) -> Result<T> {
    match prepared.delete_file(app).await {
        Ok(()) => Err(error),
        Err(cleanup_error) => Err(Error::message(format!(
            "attachment operation failed: {error}; storage cleanup failed: {cleanup_error}"
        ))),
    }
}

async fn rollback_attachment_transaction<T>(
    transaction: AppTransaction,
    error: Error,
) -> Result<T> {
    match transaction.rollback().await {
        Ok(()) => Err(error),
        Err(rollback_error) => Err(Error::message(format!(
            "attachment operation failed: {error}; rollback failed: {rollback_error}"
        ))),
    }
}

fn validate_attachment_reorder(
    existing: &[Attachment],
    ordered_ids: &[String],
) -> Result<Vec<Uuid>> {
    let existing_ids = existing
        .iter()
        .map(|attachment| parse_attachment_uuid(&attachment.id, "id"))
        .collect::<Result<BTreeSet<_>>>()?;
    let mut requested_ids = BTreeSet::new();
    let mut ordered = Vec::with_capacity(ordered_ids.len());
    for id in ordered_ids {
        let id = parse_attachment_uuid(id, "id")?;
        if !requested_ids.insert(id) {
            return Err(Error::message(format!(
                "attachment `{id}` appears more than once in the requested order"
            )));
        }
        ordered.push(id);
    }
    if existing_ids != requested_ids {
        return Err(Error::message(format!(
            "attachment reorder must contain every current collection ID exactly once (expected {}, received {})",
            existing_ids.len(),
            requested_ids.len()
        )));
    }
    Ok(ordered)
}

// ---------------------------------------------------------------------------
// HasAttachments trait
// ---------------------------------------------------------------------------

/// Trait for models that can have file attachments.
///
/// ```ignore
/// impl HasAttachments for User {
///     fn attachable_type() -> &'static str { "users" }
///     fn attachable_id(&self) -> String { self.id.to_string() }
/// }
///
/// user.attach(&app, "avatar", uploaded_file).await?;
/// let avatar = user.attachment(&app, "avatar").await?;
/// ```
#[async_trait::async_trait]
pub trait HasAttachments: Send + Sync {
    fn attachable_type() -> &'static str;
    fn attachable_id(&self) -> String;
    fn attachment_specs() -> Vec<AttachmentSpec<Self>>
    where
        Self: Sized,
    {
        Vec::new()
    }

    async fn attach(
        &self,
        app: &AppContext,
        collection: &str,
        file: UploadedFile,
    ) -> Result<Attachment>
    where
        Self: Sized,
    {
        store_model_attachment(self, app, collection, file, false).await
    }

    async fn replace_attachment(
        &self,
        app: &AppContext,
        collection: &str,
        file: UploadedFile,
    ) -> Result<Attachment>
    where
        Self: Sized,
    {
        store_model_attachment(self, app, collection, file, true).await
    }

    /// Attach a file to a locale-specific collection.
    ///
    /// The locale must exist in `app.i18n()?.locale_list()`.
    async fn attach_localized(
        &self,
        app: &AppContext,
        collection: &str,
        locale: &str,
        file: UploadedFile,
    ) -> Result<Attachment>
    where
        Self: Sized,
    {
        let collection = localized_collection_for(app, collection, locale)?;
        self.attach(app, &collection, file).await
    }

    /// Replace the first-class localized asset for a collection and locale.
    ///
    /// The new file is stored before old files are removed, so a failed upload
    /// does not leave the locale without its previous asset.
    async fn replace_localized_attachment(
        &self,
        app: &AppContext,
        collection: &str,
        locale: &str,
        file: UploadedFile,
    ) -> Result<Attachment>
    where
        Self: Sized,
    {
        let collection = localized_collection_for(app, collection, locale)?;
        self.replace_attachment(app, &collection, file).await
    }

    /// Read the first attachment for an exact locale.
    async fn localized_attachment(
        &self,
        app: &AppContext,
        collection: &str,
        locale: &str,
    ) -> Result<Option<Attachment>> {
        let collection = localized_collection_for(app, collection, locale)?;
        self.attachment(app, &collection).await
    }

    /// Read all attachments for an exact locale.
    async fn localized_attachments(
        &self,
        app: &AppContext,
        collection: &str,
        locale: &str,
    ) -> Result<Vec<Attachment>> {
        let collection = localized_collection_for(app, collection, locale)?;
        self.attachments(app, &collection).await
    }

    /// Read a localized attachment, falling back to the i18n default locale.
    async fn localized_attachment_or_default(
        &self,
        app: &AppContext,
        collection: &str,
        locale: &str,
    ) -> Result<Option<Attachment>> {
        if let Some(attachment) = self.localized_attachment(app, collection, locale).await? {
            return Ok(Some(attachment));
        }

        let default_locale = app
            .i18n()
            .map_err(|_| {
                Error::http_with_code(
                    400,
                    "localized attachments require i18n to be configured",
                    "i18n_not_configured",
                )
            })?
            .default_locale()
            .to_string();

        if locale.trim() == default_locale {
            return Ok(None);
        }

        self.localized_attachment(app, collection, &default_locale)
            .await
    }

    /// Read a localized attachment for the current request locale, with default fallback.
    async fn current_localized_attachment(
        &self,
        app: &AppContext,
        collection: &str,
    ) -> Result<Option<Attachment>> {
        let locale = crate::translations::current_locale(app);
        self.localized_attachment_or_default(app, collection, &locale)
            .await
    }

    async fn attachment(&self, app: &AppContext, collection: &str) -> Result<Option<Attachment>> {
        let attachable_type = attachment_model_type::<Self>()?;
        let attachable_id = attachment_model_id(self)?;
        if let Some(rows) =
            cached_attachments_for_id(app, attachable_type, &attachable_id, collection).await?
        {
            return Ok(rows.into_iter().next());
        }

        let rows = order_attachment_rows(
            attachment_select_query()
                .where_eq("attachable_type", attachable_type.to_string())
                .where_eq(
                    "attachable_id",
                    parse_attachment_uuid(&attachable_id, "attachable_id")?,
                )
                .where_eq("collection", collection.to_string()),
        )
        .limit(1)
        .get(&*app.database()?)
        .await?;
        rows.first().map(row_to_attachment).transpose()
    }

    async fn attachments(&self, app: &AppContext, collection: &str) -> Result<Vec<Attachment>> {
        let attachable_type = attachment_model_type::<Self>()?;
        let attachable_id = attachment_model_id(self)?;
        attachments_for_identity(app, attachable_type, &attachable_id, collection).await
    }

    /// Replace a collection's order with an exact permutation of its attachment IDs.
    ///
    /// The collection is locked while the current membership is validated and positions are
    /// rewritten. Omitting, duplicating, or adding an ID fails without changing any row.
    async fn reorder_attachments(
        &self,
        app: &AppContext,
        collection: &str,
        ordered_ids: &[String],
    ) -> Result<Vec<Attachment>> {
        if ordered_ids.len() > i32::MAX as usize {
            return Err(Error::message(
                "attachment collection exceeds i32 sort-order capacity",
            ));
        }
        let attachable_type = attachment_model_type::<Self>()?;
        let attachable_id = attachment_model_id(self)?;
        let attachable_uuid = parse_attachment_uuid(&attachable_id, "attachable_id")?;
        let transaction = app.begin_transaction().await?;
        if let Err(error) = acquire_attachment_collection_lock(
            &transaction,
            attachable_type,
            attachable_uuid,
            collection,
        )
        .await
        {
            return rollback_attachment_transaction(transaction, error).await;
        }
        let existing = match attachments_for_identity_with(
            &transaction,
            attachable_type,
            &attachable_id,
            collection,
            true,
        )
        .await
        {
            Ok(existing) => existing,
            Err(error) => return rollback_attachment_transaction(transaction, error).await,
        };
        let ordered_uuids = match validate_attachment_reorder(&existing, ordered_ids) {
            Ok(ids) => ids,
            Err(error) => return rollback_attachment_transaction(transaction, error).await,
        };

        for (position, attachment_id) in ordered_uuids.iter().enumerate() {
            let affected = match Query::update_table(ATTACHMENTS_TABLE)
                .value("sort_order", DbValue::Int32(position as i32))
                .where_eq("id", *attachment_id)
                .where_eq("attachable_type", attachable_type.to_string())
                .where_eq("attachable_id", attachable_uuid)
                .where_eq("collection", collection.to_string())
                .execute(&transaction)
                .await
            {
                Ok(affected) => affected,
                Err(error) => return rollback_attachment_transaction(transaction, error).await,
            };
            if affected != 1 {
                return rollback_attachment_transaction(
                    transaction,
                    Error::message(format!(
                        "attachment `{attachment_id}` changed while reordering collection `{collection}`"
                    )),
                )
                .await;
            }
        }
        transaction.commit().await?;
        invalidate_attachment_cache(attachable_type, &attachable_id, Some(collection));

        let mut by_id = existing
            .into_iter()
            .map(|attachment| (attachment.id.clone(), attachment))
            .collect::<HashMap<_, _>>();
        Ok(ordered_ids
            .iter()
            .enumerate()
            .filter_map(|(position, id)| {
                by_id.remove(id).map(|mut attachment| {
                    attachment.sort_order = position as i32;
                    attachment
                })
            })
            .collect())
    }

    /// Delete an attachment and its file from storage.
    async fn detach(&self, app: &AppContext, attachment_id: &str) -> Result<()> {
        let attachable_type = attachment_model_type::<Self>()?;
        let attachable_id = attachment_model_id(self)?;
        detach_attachment_by_identity(app, attachable_type, &attachable_id, attachment_id, true)
            .await
    }

    /// Delete attachment record but keep the file on storage.
    async fn detach_keep_file(&self, app: &AppContext, attachment_id: &str) -> Result<()> {
        let attachable_type = attachment_model_type::<Self>()?;
        let attachable_id = attachment_model_id(self)?;
        detach_attachment_by_identity(app, attachable_type, &attachable_id, attachment_id, false)
            .await
    }

    /// Delete all attachments in a collection and their files.
    async fn detach_all(&self, app: &AppContext, collection: &str) -> Result<u64> {
        let attachable_type = attachment_model_type::<Self>()?;
        let attachable_id = attachment_model_id(self)?;
        let db = app.database()?;
        let rows = attachment_file_select_query()
            .where_eq("attachable_type", attachable_type.to_string())
            .where_eq(
                "attachable_id",
                parse_attachment_uuid(&attachable_id, "attachable_id")?,
            )
            .where_eq("collection", collection.to_string())
            .get(&*db)
            .await?
            .into_vec();

        let affected = Query::delete_from(ATTACHMENTS_TABLE)
            .where_eq("attachable_type", attachable_type.to_string())
            .where_eq(
                "attachable_id",
                parse_attachment_uuid(&attachable_id, "attachable_id")?,
            )
            .where_eq("collection", collection.to_string())
            .execute(&*db)
            .await?;

        delete_attachment_files(app, &rows).await;
        invalidate_attachment_cache(attachable_type, &attachable_id, Some(collection));
        Ok(affected)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn opt_text(value: &Option<String>) -> DbValue {
    match value {
        Some(s) => DbValue::Text(s.clone()),
        None => DbValue::Null(DbType::Text),
    }
}

fn attachment_model_type<M>() -> Result<&'static str>
where
    M: HasAttachments + ?Sized,
{
    let subject = format!("attachable_type for model `{}`", std::any::type_name::<M>());
    callback::run_attachment_sync(&subject, M::attachable_type)
}

fn attachment_model_id<M>(model: &M) -> Result<String>
where
    M: HasAttachments + ?Sized,
{
    let subject = format!("attachable_id for model `{}`", std::any::type_name::<M>());
    callback::run_attachment_sync(&subject, || model.attachable_id())
}

fn collect_attachment_ids<'a, M>(models: impl IntoIterator<Item = &'a M>) -> Result<Vec<String>>
where
    M: HasAttachments + ?Sized + 'a,
{
    let mut ids = Vec::new();
    for model in models {
        ids.push(attachment_model_id(model)?);
    }
    Ok(collect_unique_ids(ids))
}

struct ResolvedAttachmentSpec<M> {
    spec: AttachmentSpec<M>,
    locale: Option<String>,
}

#[derive(Debug)]
struct ProcessedImage {
    bytes: Vec<u8>,
    format: ImageFormat,
}

struct ImageProcessingOptions<'a> {
    transforms: &'a [ImageTransform],
    output_format: Option<ImageFormat>,
    quality: Option<u8>,
    allow_upscale: bool,
    require_image: bool,
    decode_limits: ImageDecodeLimits,
}

struct OwnedImageProcessingOptions {
    transforms: Vec<ImageTransform>,
    output_format: Option<ImageFormat>,
    quality: Option<u8>,
    allow_upscale: bool,
    require_image: bool,
    decode_limits: ImageDecodeLimits,
}

impl ImageProcessingOptions<'_> {
    fn should_process(&self) -> bool {
        self.require_image
            || !self.transforms.is_empty()
            || self.output_format.is_some()
            || self.quality.is_some()
    }
}

async fn process_image_bytes_blocking(
    bytes: Vec<u8>,
    original_name: Option<String>,
    owned_options: OwnedImageProcessingOptions,
) -> Result<Option<ProcessedImage>> {
    crate::support::run_blocking("attachment image processing", move || {
        let options = ImageProcessingOptions {
            transforms: &owned_options.transforms,
            output_format: owned_options.output_format,
            quality: owned_options.quality,
            allow_upscale: owned_options.allow_upscale,
            require_image: owned_options.require_image,
            decode_limits: owned_options.decode_limits,
        };
        process_image_bytes(&bytes, original_name.as_deref(), &options)
    })
    .await
}

async fn store_model_attachment<M>(
    model: &M,
    app: &AppContext,
    collection: &str,
    file: UploadedFile,
    replace_existing: bool,
) -> Result<Attachment>
where
    M: HasAttachments,
{
    let resolved = resolve_attachment_spec::<M>(collection)?;
    let attachable_type = attachment_model_type::<M>()?;
    let attachable_id = attachment_model_id(model)?;

    if let Some(resolved) = &resolved {
        run_before_store_hooks(resolved, app, model, collection, &file).await?;
    }

    let should_replace = replace_existing
        || resolved
            .as_ref()
            .map(|resolved| resolved.spec.is_single())
            .unwrap_or(false);
    let file_for_hooks = file.clone();
    let mut builder = Attachment::upload(file).collection(collection);
    if let Some(resolved) = &resolved {
        builder = builder.apply_spec(&resolved.spec);
    }
    let prepared = builder
        .prepare(app, attachable_type, &attachable_id)
        .await?;
    let transaction = match app.begin_transaction().await {
        Ok(transaction) => transaction,
        Err(error) => return cleanup_prepared_attachment(app, &prepared, error).await,
    };
    if let Err(error) = acquire_attachment_collection_lock(
        &transaction,
        attachable_type,
        prepared.attachable_uuid,
        collection,
    )
    .await
    {
        return rollback_prepared_attachment(transaction, app, &prepared, error).await;
    }
    let existing = if should_replace {
        match attachments_for_identity_with(
            &transaction,
            attachable_type,
            &attachable_id,
            collection,
            true,
        )
        .await
        {
            Ok(existing) => existing,
            Err(error) => {
                return rollback_prepared_attachment(transaction, app, &prepared, error).await;
            }
        }
    } else {
        Vec::new()
    };
    let sort_order = match existing.first() {
        Some(existing) => existing.sort_order,
        None => match next_attachment_sort_order(
            &transaction,
            attachable_type,
            prepared.attachable_uuid,
            collection,
        )
        .await
        {
            Ok(sort_order) => sort_order,
            Err(error) => {
                return rollback_prepared_attachment(transaction, app, &prepared, error).await;
            }
        },
    };
    let attachment = match prepared.insert_with(&transaction, sort_order).await {
        Ok(attachment) => attachment,
        Err(error) => {
            return rollback_prepared_attachment(transaction, app, &prepared, error).await;
        }
    };

    if let Some(resolved) = &resolved {
        if let Err(error) = run_after_store_hooks(
            resolved,
            app,
            model,
            collection,
            &file_for_hooks,
            &attachment,
        )
        .await
        {
            return rollback_prepared_attachment(transaction, app, &prepared, error).await;
        }
    }

    if !existing.is_empty() {
        let existing_ids = match existing
            .iter()
            .map(|attachment| parse_attachment_uuid(&attachment.id, "id"))
            .collect::<Result<Vec<_>>>()
        {
            Ok(ids) => ids,
            Err(error) => {
                return rollback_prepared_attachment(transaction, app, &prepared, error).await;
            }
        };
        if let Err(error) = Query::delete_from(ATTACHMENTS_TABLE)
            .where_in("id", existing_ids)
            .execute(&transaction)
            .await
        {
            return rollback_prepared_attachment(transaction, app, &prepared, error).await;
        }
    }
    transaction.commit().await?;
    delete_attachment_values(app, &existing).await;

    invalidate_attachment_cache(attachable_type, &attachable_id, Some(collection));

    Ok(attachment)
}

async fn attachments_for_identity(
    app: &AppContext,
    attachable_type: &str,
    attachable_id: &str,
    collection: &str,
) -> Result<Vec<Attachment>> {
    if let Some(rows) =
        cached_attachments_for_id(app, attachable_type, attachable_id, collection).await?
    {
        return Ok(rows);
    }

    attachments_for_identity_with(
        app.database()?.as_ref(),
        attachable_type,
        attachable_id,
        collection,
        false,
    )
    .await
}

async fn attachments_for_identity_with<E>(
    executor: &E,
    attachable_type: &str,
    attachable_id: &str,
    collection: &str,
    for_update: bool,
) -> Result<Vec<Attachment>>
where
    E: QueryExecutor,
{
    let query = order_attachment_rows(
        attachment_select_query()
            .where_eq("attachable_type", attachable_type.to_string())
            .where_eq(
                "attachable_id",
                parse_attachment_uuid(attachable_id, "attachable_id")?,
            )
            .where_eq("collection", collection.to_string()),
    );
    let query = if for_update {
        query.for_update()
    } else {
        query
    };
    let rows = query.get(executor).await?;
    rows.iter().map(row_to_attachment).collect()
}

async fn detach_attachment_by_identity(
    app: &AppContext,
    attachable_type: &str,
    attachable_id: &str,
    attachment_id: &str,
    delete_file: bool,
) -> Result<()> {
    let db = app.database()?;
    let rows = if delete_file {
        attachment_file_select_query()
            .where_eq("id", parse_attachment_uuid(attachment_id, "attachment_id")?)
            .where_eq("attachable_type", attachable_type.to_string())
            .where_eq(
                "attachable_id",
                parse_attachment_uuid(attachable_id, "attachable_id")?,
            )
            .get(&*db)
            .await?
            .into_vec()
    } else {
        Vec::new()
    };

    Query::delete_from(ATTACHMENTS_TABLE)
        .where_eq("id", parse_attachment_uuid(attachment_id, "attachment_id")?)
        .where_eq("attachable_type", attachable_type.to_string())
        .where_eq(
            "attachable_id",
            parse_attachment_uuid(attachable_id, "attachable_id")?,
        )
        .execute(&*db)
        .await?;
    if delete_file {
        delete_attachment_files(app, &rows).await;
    }
    invalidate_attachment_cache(attachable_type, attachable_id, None);
    Ok(())
}

async fn delete_attachment_files(app: &AppContext, rows: &[crate::database::DbRecord]) {
    let Ok(storage) = app.storage() else {
        return;
    };

    for row in rows {
        let (Some(DbValue::Text(disk)), Some(DbValue::Text(path))) =
            (row.get("disk"), row.get("path"))
        else {
            continue;
        };
        let Ok(disk) = storage.disk(disk) else {
            continue;
        };
        let _ = disk.delete(path).await;
    }
}

async fn delete_attachment_values(app: &AppContext, attachments: &[Attachment]) {
    let Ok(storage) = app.storage() else {
        return;
    };
    for attachment in attachments {
        let Ok(disk) = storage.disk(&attachment.disk) else {
            continue;
        };
        let _ = disk.delete(&attachment.path).await;
    }
}

async fn run_before_store_hooks<M>(
    resolved: &ResolvedAttachmentSpec<M>,
    app: &AppContext,
    model: &M,
    collection: &str,
    file: &UploadedFile,
) -> Result<()>
where
    M: Send + Sync,
{
    for hook in resolved.spec.hooks() {
        let subject = format!("before_store hook for collection `{collection}`");
        callback::run_attachment_callback(&subject, || {
            hook.before_store(AttachmentBeforeStoreContext {
                app,
                model,
                spec: &resolved.spec,
                collection,
                locale: resolved.locale.as_deref(),
                file,
            })
        })
        .await?;
    }

    Ok(())
}

async fn run_after_store_hooks<M>(
    resolved: &ResolvedAttachmentSpec<M>,
    app: &AppContext,
    model: &M,
    collection: &str,
    file: &UploadedFile,
    attachment: &Attachment,
) -> Result<()>
where
    M: Send + Sync,
{
    for hook in resolved.spec.hooks() {
        let subject = format!("after_store hook for collection `{collection}`");
        callback::run_attachment_callback(&subject, || {
            hook.after_store(AttachmentAfterStoreContext {
                app,
                model,
                spec: &resolved.spec,
                collection,
                locale: resolved.locale.as_deref(),
                file,
                attachment,
            })
        })
        .await?;
    }

    Ok(())
}

#[cfg(test)]
async fn cleanup_after_store_failure<T, C, Fut>(error: Error, cleanup: C) -> Result<T>
where
    C: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if let Err(cleanup_error) =
        callback::run_attachment_callback("cleanup after after_store failure", cleanup).await
    {
        return Err(Error::message(format!(
            "attachment after_store hook failed: {error}; cleanup failed: {cleanup_error}"
        )));
    }

    Err(error)
}

fn resolve_attachment_spec<M>(collection: &str) -> Result<Option<ResolvedAttachmentSpec<M>>>
where
    M: HasAttachments,
{
    let subject = format!("spec registry for model `{}`", std::any::type_name::<M>());
    let specs = callback::run_attachment_sync(&subject, M::attachment_specs)?;
    if let Some(spec) = specs
        .iter()
        .find(|spec| spec.collection().trim() == collection.trim())
        .cloned()
    {
        return Ok(Some(ResolvedAttachmentSpec { spec, locale: None }));
    }

    let Some((base_collection, locale)) = split_localized_collection(collection) else {
        return Ok(None);
    };
    Ok(specs
        .into_iter()
        .find(|spec| spec.collection().trim() == base_collection)
        .map(|spec| ResolvedAttachmentSpec {
            spec,
            locale: Some(locale.to_string()),
        }))
}

fn split_localized_collection(collection: &str) -> Option<(&str, &str)> {
    let (base, locale) = collection.split_once(LOCALIZED_COLLECTION_SEPARATOR)?;
    let base = base.trim();
    let locale = locale.trim();
    if base.is_empty() || locale.is_empty() {
        return None;
    }
    Some((base, locale))
}

fn process_image_bytes(
    bytes: &[u8],
    original_name: Option<&str>,
    options: &ImageProcessingOptions<'_>,
) -> Result<Option<ProcessedImage>> {
    if !options.should_process() {
        return Ok(None);
    }

    options
        .decode_limits
        .check_input_bytes(bytes.len() as u64)
        .map_err(attachment_image_decode_limit_error)?;
    let (width, height) = image_dimensions_from_bytes(bytes)?;
    options
        .decode_limits
        .check_dimensions(width, height)
        .map_err(attachment_image_decode_limit_error)?;

    let mut processor =
        crate::imaging::ImageProcessor::from_bytes_with_limits(bytes, options.decode_limits)
            .map_err(|_| invalid_attachment_image_error())?;
    let detected_format = processor.format();
    let needs_reencode = !options.transforms.is_empty()
        || options.output_format.is_some()
        || options.quality.is_some();

    if !needs_reencode {
        return Ok(None);
    }

    for transform in options.transforms {
        processor = apply_image_transform(processor, *transform, options.allow_upscale)?;
    }

    if let Some(quality) = options.quality {
        processor = processor.quality(quality);
    }

    let format = options
        .output_format
        .or_else(|| image_format_from_name(original_name))
        .or(detected_format)
        .unwrap_or(ImageFormat::Jpeg);
    let bytes = processor.to_bytes(format)?;

    Ok(Some(ProcessedImage { bytes, format }))
}

fn apply_image_transform(
    processor: crate::imaging::ImageProcessor,
    transform: ImageTransform,
    allow_upscale: bool,
) -> Result<crate::imaging::ImageProcessor> {
    if !allow_upscale {
        ensure_transform_without_upscale(&processor, transform)?;
    }

    let processor = match transform {
        ImageTransform::Resize(width, height) => processor.resize(width, height),
        ImageTransform::ResizeToFit(max_width, max_height) => {
            if !allow_upscale && processor.width() <= max_width && processor.height() <= max_height
            {
                processor
            } else {
                processor.resize_to_fit(max_width, max_height)
            }
        }
        ImageTransform::ResizeToFill(width, height) => processor.resize_to_fill(width, height),
    };

    Ok(processor)
}

fn ensure_transform_without_upscale(
    processor: &crate::imaging::ImageProcessor,
    transform: ImageTransform,
) -> Result<()> {
    let resize = match transform {
        ImageTransform::Resize(width, height) => AttachmentImageResize::Exact { width, height },
        ImageTransform::ResizeToFill(width, height) => {
            AttachmentImageResize::Fill { width, height }
        }
        ImageTransform::ResizeToFit(_, _) => {
            return Ok(());
        }
    };
    let (width, height) = resize.target_dimensions();

    if processor.width() < width || processor.height() < height {
        return Err(attachment_image_too_small_error(width, height));
    }

    Ok(())
}

fn image_dimensions_from_bytes(bytes: &[u8]) -> Result<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid_attachment_image_error())?
        .into_dimensions()
        .map_err(|_| invalid_attachment_image_error())
}

fn image_format_from_name(name: Option<&str>) -> Option<ImageFormat> {
    name.and_then(|name| name.rsplit('.').next())
        .and_then(ImageFormat::from_extension)
}

fn image_mime_type(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/png",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Gif => "image/gif",
        ImageFormat::Bmp => "image/bmp",
        ImageFormat::Tiff => "image/tiff",
        ImageFormat::Avif => "image/avif",
        ImageFormat::Ico => "image/vnd.microsoft.icon",
    }
}

fn invalid_attachment_image_error() -> Error {
    Error::http_with_code(
        422,
        "attachment must be a valid image",
        "invalid_attachment_image",
    )
}

fn attachment_image_too_small_error(width: u32, height: u32) -> Error {
    Error::http_with_code(
        422,
        format!("attachment image must be at least {width}x{height}"),
        "attachment_image_too_small",
    )
}

fn attachment_image_decode_limit_error(violation: ImageDecodeLimitViolation) -> Error {
    match violation {
        ImageDecodeLimitViolation::InputBytes { actual, max } => {
            attachment_image_input_too_large_error(actual, max)
        }
        ImageDecodeLimitViolation::Dimensions {
            width,
            height,
            max_width,
            max_height,
            max_pixels,
        } => attachment_image_dimensions_too_large_error(
            width, height, max_width, max_height, max_pixels,
        ),
    }
}

fn attachment_image_input_too_large_error(actual: u64, max: u64) -> Error {
    Error::http_with_code(
        422,
        format!("attachment image input is too large ({actual} bytes, max {max})"),
        "attachment_image_input_too_large",
    )
}

fn attachment_image_dimensions_too_large_error(
    width: u64,
    height: u64,
    max_width: u64,
    max_height: u64,
    max_pixels: u64,
) -> Error {
    Error::http_with_code(
        422,
        format!(
            "attachment image dimensions are too large ({width}x{height}; max width {max_width}, max height {max_height}, max pixels {max_pixels})"
        ),
        "attachment_image_dimensions_too_large",
    )
}

fn localized_collection_for(app: &AppContext, collection: &str, locale: &str) -> Result<String> {
    let collection = collection.trim();
    if collection.is_empty() {
        return Err(Error::http_with_code(
            400,
            "attachment collection is required",
            "invalid_attachment_collection",
        ));
    }

    let locale = validate_attachment_locale(app, locale)?;
    Ok(localized_attachment_collection(collection, &locale))
}

fn validate_attachment_locale(app: &AppContext, locale: &str) -> Result<String> {
    let locale = locale.trim();
    if locale.is_empty() {
        return Err(Error::http_with_code(
            400,
            "locale is required",
            "invalid_locale",
        ));
    }

    let available = available_attachment_locales(app)?;
    if available.iter().any(|candidate| candidate == locale) {
        return Ok(locale.to_string());
    }

    let message = if available.is_empty() {
        format!("locale `{locale}` is not available because no i18n locales are loaded")
    } else {
        format!(
            "locale `{locale}` is not available; available locales: {}",
            available.join(", ")
        )
    };

    Err(Error::http_with_code(400, message, "invalid_locale"))
}

pub(crate) fn row_to_attachment(row: &crate::database::DbRecord) -> Result<Attachment> {
    Ok(Attachment {
        id: row.try_text_or_uuid("id")?,
        attachable_type: row.try_text("attachable_type")?,
        attachable_id: row.try_text_or_uuid("attachable_id")?,
        collection: row.try_text("collection")?,
        disk: row.try_text("disk")?,
        path: row.try_text("path")?,
        name: row.try_text("name")?,
        original_name: row.optional_text("original_name"),
        mime_type: row.optional_text("mime_type"),
        size: match row.get("size") {
            Some(DbValue::Int64(n)) => *n,
            _ => 0,
        },
        sort_order: match row.get("sort_order") {
            Some(DbValue::Int32(n)) => *n,
            _ => 0,
        },
        custom_properties: match row.get("custom_properties") {
            Some(DbValue::Json(v)) => v.clone(),
            _ => serde_json::json!({}),
        },
    })
}

async fn cached_attachments_for_id(
    executor: &dyn QueryExecutor,
    attachable_type: &str,
    attachable_id: &str,
    collection: &str,
) -> Result<Option<Vec<Attachment>>> {
    let Some(scope) = current_extension_scope() else {
        return Ok(None);
    };

    if let Some(rows) = scope.cached_attachments(attachable_type, collection, attachable_id) {
        return Ok(Some(rows));
    }

    let missing_ids =
        scope.missing_attachment_ids_for_known(attachable_type, collection, attachable_id);
    if !missing_ids.is_empty() {
        let rows =
            load_attachment_rows(executor, attachable_type, collection, &missing_ids).await?;
        scope.store_attachments(attachable_type, collection, &missing_ids, rows);
    }

    Ok(Some(
        scope
            .cached_attachments(attachable_type, collection, attachable_id)
            .unwrap_or_default(),
    ))
}

async fn load_attachment_rows(
    executor: &dyn QueryExecutor,
    attachable_type: &str,
    collection: &str,
    attachable_ids: &[String],
) -> Result<Vec<Attachment>> {
    if attachable_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows = order_batched_attachment_rows(
        attachment_select_query()
            .where_eq("attachable_type", attachable_type.to_string())
            .where_in("attachable_id", uuid_array_from_ids(attachable_ids)?)
            .where_eq("collection", collection.to_string()),
    )
    .get(executor)
    .await?;
    rows.iter().map(row_to_attachment).collect()
}

fn parse_attachment_uuid(id: &str, label: &str) -> Result<Uuid> {
    Uuid::parse_str(id).map_err(|error| {
        Error::message(format!(
            "attachment expected UUID `{label}` `{id}`: {error}"
        ))
    })
}

fn attachment_select_query() -> Query {
    Query::table(ATTACHMENTS_TABLE).select([
        "id",
        "attachable_type",
        "attachable_id",
        "collection",
        "disk",
        "path",
        "name",
        "original_name",
        "mime_type",
        "size",
        "sort_order",
        "custom_properties",
    ])
}

fn attachment_file_select_query() -> Query {
    Query::table(ATTACHMENTS_TABLE).select(["disk", "path"])
}

fn order_attachment_rows(query: Query) -> Query {
    query
        .order_by(OrderBy::asc("sort_order"))
        .order_by(OrderBy::asc("created_at"))
}

fn order_batched_attachment_rows(query: Query) -> Query {
    query
        .order_by(OrderBy::asc("attachable_id"))
        .order_by(OrderBy::asc("sort_order"))
        .order_by(OrderBy::asc("created_at"))
}

fn invalidate_attachment_cache(
    attachable_type: &str,
    attachable_id: &str,
    collection: Option<&str>,
) {
    if let Some(scope) = current_extension_scope() {
        match collection {
            Some(collection) => {
                scope.invalidate_attachment_collection(attachable_type, attachable_id, collection)
            }
            None => scope.invalidate_attachments(attachable_type, attachable_id),
        }
    }
}

fn collect_unique_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| !id.trim().is_empty())
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use image::DynamicImage;
    use uuid::Uuid;

    use crate::config::ConfigRepository;
    use crate::database::{
        scope_model_extensions, DbRecord, DbValue, QueryExecutionOptions, QueryExecutor,
    };
    use crate::foundation::Container;
    use crate::validation::RuleRegistry;

    use super::*;

    struct SpecAttachable {
        id: String,
    }

    impl HasAttachments for SpecAttachable {
        fn attachable_type() -> &'static str {
            "spec_attachables"
        }

        fn attachable_id(&self) -> String {
            self.id.clone()
        }

        fn attachment_specs() -> Vec<AttachmentSpec<Self>> {
            vec![AttachmentSpec::image("main")
                .single()
                .resize_to_fill(1200, 630)
                .format(ImageFormat::WebP)
                .quality(85)
                .upscale(true)]
        }
    }

    struct PanickingSpecAttachable;

    impl HasAttachments for PanickingSpecAttachable {
        fn attachable_type() -> &'static str {
            "panicking_spec_attachables"
        }

        fn attachable_id(&self) -> String {
            Uuid::now_v7().to_string()
        }

        fn attachment_specs() -> Vec<AttachmentSpec<Self>> {
            panic!("attachment specs exploded")
        }
    }

    struct PanickingTypeAttachable;

    impl HasAttachments for PanickingTypeAttachable {
        fn attachable_type() -> &'static str {
            panic!("attachable type exploded")
        }

        fn attachable_id(&self) -> String {
            Uuid::now_v7().to_string()
        }
    }

    struct PanickingIdAttachable;

    impl HasAttachments for PanickingIdAttachable {
        fn attachable_type() -> &'static str {
            "panicking_id_attachables"
        }

        fn attachable_id(&self) -> String {
            panic!("attachable id exploded")
        }
    }

    struct NoopAttachmentHook;

    #[async_trait]
    impl AttachmentSpecHook<SpecAttachable> for NoopAttachmentHook {}

    struct RecordingAttachmentHook {
        label: &'static str,
        events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AttachmentSpecHook<SpecAttachable> for RecordingAttachmentHook {
        async fn before_store(
            &self,
            _ctx: AttachmentBeforeStoreContext<'_, SpecAttachable>,
        ) -> Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("before:{}", self.label));
            Ok(())
        }

        async fn after_store(
            &self,
            _ctx: AttachmentAfterStoreContext<'_, SpecAttachable>,
        ) -> Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("after:{}", self.label));
            Ok(())
        }
    }

    struct PanickingBeforeStoreHook {
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl AttachmentSpecHook<SpecAttachable> for PanickingBeforeStoreHook {
        async fn before_store(
            &self,
            _ctx: AttachmentBeforeStoreContext<'_, SpecAttachable>,
        ) -> Result<()> {
            self.events.lock().unwrap().push("before-panic");
            panic!("before hook exploded")
        }
    }

    struct PanickingAfterStoreHook;

    #[async_trait]
    impl AttachmentSpecHook<SpecAttachable> for PanickingAfterStoreHook {
        async fn after_store(
            &self,
            _ctx: AttachmentAfterStoreContext<'_, SpecAttachable>,
        ) -> Result<()> {
            panic!("after hook exploded")
        }
    }

    struct FailingAfterStoreHook;

    #[async_trait]
    impl AttachmentSpecHook<SpecAttachable> for FailingAfterStoreHook {
        async fn after_store(
            &self,
            _ctx: AttachmentAfterStoreContext<'_, SpecAttachable>,
        ) -> Result<()> {
            Err(Error::message("after hook failed"))
        }
    }

    #[derive(Default)]
    struct CountingAttachmentExecutor {
        query_count: AtomicUsize,
    }

    #[async_trait]
    impl QueryExecutor for CountingAttachmentExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            self.query_count.fetch_add(1, Ordering::SeqCst);
            let attachable_type = match &bindings[0] {
                DbValue::Text(value) => value.clone(),
                _ => panic!("expected attachable_type binding"),
            };
            let ids: Vec<Uuid> = bindings
                .iter()
                .filter_map(|binding| match binding {
                    DbValue::Uuid(value) => Some(*value),
                    _ => None,
                })
                .collect();
            assert!(!ids.is_empty(), "expected attachable_id uuid bindings");
            let collection = bindings
                .iter()
                .rev()
                .find_map(|binding| match binding {
                    DbValue::Text(value) if value != &attachable_type => Some(value.clone()),
                    _ => None,
                })
                .expect("expected collection binding");

            Ok(ids
                .into_iter()
                .map(|id| attachment_record(&attachable_type, id, &collection))
                .collect())
        }

        async fn raw_execute_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<u64> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct RecordingAttachmentExecutor {
        execute_calls: Mutex<Vec<(String, Vec<DbValue>)>>,
    }

    #[async_trait]
    impl QueryExecutor for RecordingAttachmentExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            Ok(Vec::new())
        }

        async fn raw_execute_with(
            &self,
            sql: &str,
            bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<u64> {
            self.execute_calls
                .lock()
                .unwrap()
                .push((sql.to_string(), bindings.to_vec()));
            Ok(1)
        }
    }

    #[tokio::test]
    async fn attachment_custom_properties_update_uses_executor() {
        let executor = RecordingAttachmentExecutor::default();
        let attachment_id = Uuid::now_v7().to_string();
        let custom_properties = serde_json::json!({ "width": 640, "height": 480 });

        let affected =
            Attachment::update_custom_properties_with(&executor, &attachment_id, custom_properties)
                .await
                .unwrap();

        assert_eq!(affected, 1);
        let calls = executor.execute_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0]
            .0
            .contains("UPDATE \"attachments\" SET \"custom_properties\""));
        assert_eq!(
            calls[0].1,
            vec![
                DbValue::Json(serde_json::json!({ "width": 640, "height": 480 })),
                DbValue::Uuid(Uuid::parse_str(&attachment_id).unwrap()),
            ]
        );
    }

    #[tokio::test]
    async fn lazy_attachment_cache_batches_known_scope_ids() {
        let executor = CountingAttachmentExecutor::default();
        let first_id = Uuid::now_v7().to_string();
        let second_id = Uuid::now_v7().to_string();

        scope_model_extensions(async {
            current_extension_scope()
                .unwrap()
                .register_model_ids("test_attachables", [first_id.clone(), second_id.clone()]);

            let first = cached_attachments_for_id(&executor, "test_attachables", &first_id, "logo")
                .await
                .unwrap()
                .unwrap();
            let second =
                cached_attachments_for_id(&executor, "test_attachables", &second_id, "logo")
                    .await
                    .unwrap()
                    .unwrap();

            assert_eq!(executor.query_count.load(Ordering::SeqCst), 1);
            assert_eq!(first[0].attachable_id, first_id);
            assert_eq!(second[0].attachable_id, second_id);
        })
        .await;
    }

    #[test]
    fn localized_attachment_collection_uses_stable_locale_suffix() {
        assert_eq!(
            localized_attachment_collection(" banner_image ", " ms "),
            "banner_image:ms"
        );
    }

    #[test]
    fn attachment_spec_resolves_exact_and_localized_collections() {
        let exact = resolve_attachment_spec::<SpecAttachable>("main")
            .unwrap()
            .unwrap();
        assert!(exact.locale.is_none());
        assert!(exact.spec.is_single());

        let localized = resolve_attachment_spec::<SpecAttachable>("main:ms")
            .unwrap()
            .unwrap();
        assert_eq!(localized.locale.as_deref(), Some("ms"));
        assert_eq!(localized.spec.collection(), "main");
        assert_eq!(
            localized.spec.image_policy().unwrap().format,
            Some(ImageFormat::WebP)
        );
    }

    #[test]
    fn image_policy_clamps_quality() {
        let low = AttachmentSpec::<SpecAttachable>::image("main").quality(0);
        assert_eq!(low.image_policy().unwrap().quality, Some(1));

        let high = AttachmentSpec::<SpecAttachable>::image("main").quality(250);
        assert_eq!(high.image_policy().unwrap().quality, Some(100));
    }

    #[test]
    fn attachment_spec_registers_hooks() {
        let spec = AttachmentSpec::<SpecAttachable>::image("main").hook(NoopAttachmentHook);
        assert_eq!(spec.hooks().len(), 1);
    }

    #[tokio::test]
    async fn attachment_spec_hooks_run_in_registration_order() {
        let app = test_app_context();
        let model = SpecAttachable {
            id: Uuid::now_v7().to_string(),
        };
        let file = test_uploaded_file("voucher.png");
        let attachment = test_attachment(&model, "main");
        let events = Arc::new(Mutex::new(Vec::new()));
        let resolved = ResolvedAttachmentSpec {
            spec: AttachmentSpec::<SpecAttachable>::image("main")
                .hook(RecordingAttachmentHook {
                    label: "one",
                    events: events.clone(),
                })
                .hook(RecordingAttachmentHook {
                    label: "two",
                    events: events.clone(),
                }),
            locale: None,
        };

        run_before_store_hooks(&resolved, &app, &model, "main", &file)
            .await
            .unwrap();
        run_after_store_hooks(&resolved, &app, &model, "main", &file, &attachment)
            .await
            .unwrap();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["before:one", "before:two", "after:one", "after:two"]
        );
    }

    #[test]
    fn attachment_specs_panic_becomes_error() {
        let error = resolve_attachment_spec::<PanickingSpecAttachable>("main")
            .err()
            .expect("attachment_specs panic should become an error");

        let message = error.to_string();
        assert!(message.contains("attachment spec registry for model"));
        assert!(message.contains("panicked: attachment specs exploded"));
    }

    #[test]
    fn attachable_type_panic_becomes_error() {
        let error = attachment_model_type::<PanickingTypeAttachable>()
            .expect_err("attachable_type panic should become an error");

        let message = error.to_string();
        assert!(message.contains("attachment attachable_type for model"));
        assert!(message.contains("panicked: attachable type exploded"));
    }

    #[test]
    fn attachable_id_panic_becomes_error() {
        let model = PanickingIdAttachable;
        let error =
            attachment_model_id(&model).expect_err("attachable_id panic should become an error");

        let message = error.to_string();
        assert!(message.contains("attachment attachable_id for model"));
        assert!(message.contains("panicked: attachable id exploded"));
    }

    #[tokio::test]
    async fn attachment_lookup_identity_panic_becomes_error_before_database() {
        let app = test_app_context();
        let model = PanickingIdAttachable;
        let error = model
            .attachment(&app, "main")
            .await
            .expect_err("attachable_id panic should become a lookup error");

        assert!(error
            .to_string()
            .contains("attachment attachable_id for model"));
    }

    #[tokio::test]
    async fn detach_identity_panic_becomes_error_before_database() {
        let app = test_app_context();
        let model = PanickingIdAttachable;
        let error = model
            .detach(&app, &Uuid::now_v7().to_string())
            .await
            .expect_err("attachable_id panic should become a detach error");

        assert!(error
            .to_string()
            .contains("attachment attachable_id for model"));
    }

    #[tokio::test]
    async fn store_identity_panic_becomes_error_before_storage() {
        let app = test_app_context();
        let model = PanickingIdAttachable;
        let file = test_uploaded_file("voucher.png");
        let error = store_model_attachment(&model, &app, "main", file, false)
            .await
            .expect_err("attachable_id panic should become a store error");

        assert!(error
            .to_string()
            .contains("attachment attachable_id for model"));
    }

    #[tokio::test]
    async fn extension_loader_identity_panic_becomes_error() {
        let executor = CountingAttachmentExecutor::default();
        let loader = AttachmentExtensionLoader::<PanickingIdAttachable> {
            collection: "main".to_string(),
            _model: PhantomData,
        };
        let models = [PanickingIdAttachable];

        let error = scope_model_extensions(async {
            loader
                .load(&executor, &models)
                .await
                .expect_err("attachable_id panic should become a loader error")
        })
        .await;

        assert!(error
            .to_string()
            .contains("attachment attachable_id for model"));
        assert_eq!(executor.query_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn before_store_hook_panic_becomes_error_and_stops_later_hooks() {
        let app = test_app_context();
        let model = SpecAttachable {
            id: Uuid::now_v7().to_string(),
        };
        let file = test_uploaded_file("voucher.png");
        let events = Arc::new(Mutex::new(Vec::new()));
        let resolved = ResolvedAttachmentSpec {
            spec: AttachmentSpec::<SpecAttachable>::image("main")
                .hook(PanickingBeforeStoreHook {
                    events: events.clone(),
                })
                .hook(RecordingAttachmentHook {
                    label: "later",
                    events: Arc::new(Mutex::new(Vec::new())),
                }),
            locale: None,
        };

        let error = run_before_store_hooks(&resolved, &app, &model, "main", &file)
            .await
            .expect_err("before_store panic should become an error");

        assert_eq!(
            error.to_string(),
            "attachment before_store hook for collection `main` panicked: before hook exploded"
        );
        assert_eq!(events.lock().unwrap().as_slice(), ["before-panic"]);
    }

    #[tokio::test]
    async fn after_store_hook_panic_becomes_error() {
        let app = test_app_context();
        let model = SpecAttachable {
            id: Uuid::now_v7().to_string(),
        };
        let file = test_uploaded_file("voucher.png");
        let attachment = test_attachment(&model, "main");
        let resolved = ResolvedAttachmentSpec {
            spec: AttachmentSpec::<SpecAttachable>::image("main").hook(PanickingAfterStoreHook),
            locale: None,
        };

        let error = run_after_store_hooks(&resolved, &app, &model, "main", &file, &attachment)
            .await
            .expect_err("after_store panic should become an error");

        assert_eq!(
            error.to_string(),
            "attachment after_store hook for collection `main` panicked: after hook exploded"
        );
    }

    #[tokio::test]
    async fn after_store_panic_runs_cleanup_and_returns_hook_error() {
        let app = test_app_context();
        let model = SpecAttachable {
            id: Uuid::now_v7().to_string(),
        };
        let file = test_uploaded_file("voucher.png");
        let attachment = test_attachment(&model, "main");
        let resolved = ResolvedAttachmentSpec {
            spec: AttachmentSpec::<SpecAttachable>::image("main").hook(PanickingAfterStoreHook),
            locale: None,
        };
        let cleanup_count = Arc::new(AtomicUsize::new(0));

        let error = run_after_store_hooks(&resolved, &app, &model, "main", &file, &attachment)
            .await
            .unwrap_err();
        let result: Result<()> = cleanup_after_store_failure(error, || {
            let cleanup_count = cleanup_count.clone();
            async move {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await;

        assert_eq!(cleanup_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            result.unwrap_err().to_string(),
            "attachment after_store hook for collection `main` panicked: after hook exploded"
        );
    }

    #[tokio::test]
    async fn cleanup_after_store_failure_reports_cleanup_panic() {
        let result: Result<()> =
            cleanup_after_store_failure(Error::message("after hook failed"), || async {
                panic!("cleanup future exploded")
            })
            .await;

        let error = result.unwrap_err().to_string();
        assert!(error.contains("after hook failed"));
        assert!(error.contains(
            "cleanup failed: attachment cleanup after after_store failure panicked: cleanup future exploded"
        ));
    }

    #[tokio::test]
    async fn cleanup_after_store_failure_catches_cleanup_factory_panic() {
        let result: Result<()> =
            cleanup_after_store_failure(Error::message("after hook failed"), || {
                panic!("cleanup factory exploded");
                #[allow(unreachable_code)]
                std::future::ready(Ok::<(), Error>(()))
            })
            .await;

        let error = result.unwrap_err().to_string();
        assert!(error.contains("after hook failed"));
        assert!(error.contains(
            "cleanup failed: attachment cleanup after after_store failure panicked: cleanup factory exploded"
        ));
    }

    #[tokio::test]
    async fn after_store_hook_error_remains_unchanged() {
        let app = test_app_context();
        let model = SpecAttachable {
            id: Uuid::now_v7().to_string(),
        };
        let file = test_uploaded_file("voucher.png");
        let attachment = test_attachment(&model, "main");
        let resolved = ResolvedAttachmentSpec {
            spec: AttachmentSpec::<SpecAttachable>::image("main").hook(FailingAfterStoreHook),
            locale: None,
        };

        let error = run_after_store_hooks(&resolved, &app, &model, "main", &file, &attachment)
            .await
            .expect_err("hook error should remain unchanged");

        assert_eq!(error.to_string(), "after hook failed");
    }

    #[tokio::test]
    async fn after_store_failure_runs_cleanup_and_returns_hook_error() {
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let result: Result<()> =
            cleanup_after_store_failure(Error::message("after hook failed"), || {
                let cleanup_count = cleanup_count.clone();
                async move {
                    cleanup_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .await;

        assert_eq!(cleanup_count.load(Ordering::SeqCst), 1);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("after hook failed"));
    }

    #[tokio::test]
    async fn after_store_failure_reports_cleanup_error() {
        let result: Result<()> =
            cleanup_after_store_failure(Error::message("after hook failed"), || async {
                Err(Error::message("cleanup failed"))
            })
            .await;

        let error = result.unwrap_err().to_string();
        assert!(error.contains("after hook failed"));
        assert!(error.contains("cleanup failed"));
    }

    #[test]
    fn image_policy_resizes_and_converts_output_format() {
        let input = test_image_bytes(640, 360, ImageFormat::Png);
        let transforms = [ImageTransform::ResizeToFill(1200, 630)];
        let options = image_processing_options(
            &transforms,
            Some(ImageFormat::WebP),
            None,
            true,
            true,
            ImageDecodeLimits::default(),
        );
        let processed = process_image_bytes(&input, Some("voucher.png"), &options)
            .unwrap()
            .unwrap();

        assert_eq!(processed.format, ImageFormat::WebP);
        let image = crate::imaging::ImageProcessor::from_bytes(&processed.bytes).unwrap();
        assert_eq!((image.width(), image.height()), (1200, 630));
    }

    #[test]
    fn image_policy_rejects_quality_for_webp_output() {
        let input = test_image_bytes(20, 10, ImageFormat::Png);
        let options = image_processing_options(
            &[],
            Some(ImageFormat::WebP),
            Some(80),
            true,
            true,
            ImageDecodeLimits::default(),
        );

        let error = process_image_bytes(&input, Some("image.png"), &options).unwrap_err();

        assert_eq!(
            error.to_string(),
            "image quality is only supported for JPEG output; WebP output is lossless"
        );
    }

    #[tokio::test]
    async fn image_policy_blocking_wrapper_resizes_and_converts_output_format() {
        let input = test_image_bytes(640, 360, ImageFormat::Png);
        let processed = process_image_bytes_blocking(
            input,
            Some("voucher.png".to_string()),
            OwnedImageProcessingOptions {
                transforms: vec![ImageTransform::ResizeToFill(1200, 630)],
                output_format: Some(ImageFormat::WebP),
                quality: None,
                allow_upscale: true,
                require_image: true,
                decode_limits: ImageDecodeLimits::default(),
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(processed.format, ImageFormat::WebP);
        let image = crate::imaging::ImageProcessor::from_bytes(&processed.bytes).unwrap();
        assert_eq!((image.width(), image.height()), (1200, 630));
    }

    #[test]
    fn image_policy_rejects_invalid_images() {
        let options =
            image_processing_options(&[], None, None, true, true, ImageDecodeLimits::default());
        let error = process_image_bytes(b"not an image", None, &options).unwrap_err();

        assert_error_code(error, "invalid_attachment_image");
    }

    #[test]
    fn image_policy_rejects_oversized_image_input_before_decode() {
        let input = test_image_bytes(10, 10, ImageFormat::Png);
        let limits = ImageDecodeLimits {
            max_input_bytes: (input.len() - 1) as u64,
            ..ImageDecodeLimits::default()
        };

        let options =
            image_processing_options(&[], Some(ImageFormat::Png), None, true, true, limits);
        let error = process_image_bytes(&input, Some("image.png"), &options).unwrap_err();

        assert_error_code(error, "attachment_image_input_too_large");
    }

    #[test]
    fn image_policy_rejects_oversized_image_dimensions() {
        let input = test_image_bytes(20, 10, ImageFormat::Png);
        let limits = ImageDecodeLimits {
            max_input_bytes: 0,
            max_pixels: 0,
            max_width: 10,
            max_height: 0,
        };

        let options =
            image_processing_options(&[], Some(ImageFormat::Png), None, true, true, limits);
        let error = process_image_bytes(&input, Some("image.png"), &options).unwrap_err();

        assert_error_code(error, "attachment_image_dimensions_too_large");
    }

    #[test]
    fn image_policy_rejects_too_small_fixed_resize_without_upscale() {
        let input = test_image_bytes(100, 100, ImageFormat::Png);
        let transforms = [ImageTransform::ResizeToFill(200, 200)];
        let options = image_processing_options(
            &transforms,
            Some(ImageFormat::Png),
            None,
            false,
            true,
            ImageDecodeLimits::default(),
        );
        let error = process_image_bytes(&input, Some("small.png"), &options).unwrap_err();

        assert_error_code(error, "attachment_image_too_small");
    }

    fn image_processing_options<'a>(
        transforms: &'a [ImageTransform],
        output_format: Option<ImageFormat>,
        quality: Option<u8>,
        allow_upscale: bool,
        require_image: bool,
        decode_limits: ImageDecodeLimits,
    ) -> ImageProcessingOptions<'a> {
        ImageProcessingOptions {
            transforms,
            output_format,
            quality,
            allow_upscale,
            require_image,
            decode_limits,
        }
    }

    fn test_app_context() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn test_uploaded_file(original_name: &str) -> UploadedFile {
        UploadedFile {
            field_name: "file".to_string(),
            original_name: Some(original_name.to_string()),
            content_type: Some("image/png".to_string()),
            size: 0,
            temp_path: PathBuf::from("/tmp/foundry-attachment-test-upload"),
        }
    }

    fn test_attachment(model: &SpecAttachable, collection: &str) -> Attachment {
        Attachment {
            id: Uuid::now_v7().to_string(),
            attachable_type: SpecAttachable::attachable_type().to_string(),
            attachable_id: model.attachable_id(),
            collection: collection.to_string(),
            disk: "local".to_string(),
            path: "attachments/spec_attachables/main/test.png".to_string(),
            name: "test.png".to_string(),
            original_name: Some("test.png".to_string()),
            mime_type: Some("image/png".to_string()),
            size: 0,
            sort_order: 0,
            custom_properties: serde_json::json!({}),
        }
    }

    fn test_image_bytes(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let image = DynamicImage::new_rgba8(width, height);
        let mut cursor = Cursor::new(Vec::new());
        let format: image::ImageFormat = format.into();
        image.write_to(&mut cursor, format).unwrap();
        cursor.into_inner()
    }

    fn assert_error_code(error: Error, expected: &str) {
        match error {
            Error::Http {
                error_code: Some(code),
                ..
            } => assert_eq!(code, expected),
            other => panic!("expected HTTP error code {expected}, got {other:?}"),
        }
    }

    fn attachment_record(attachable_type: &str, attachable_id: Uuid, collection: &str) -> DbRecord {
        let mut record = DbRecord::new();
        record.insert("id", DbValue::Uuid(Uuid::now_v7()));
        record.insert(
            "attachable_type",
            DbValue::Text(attachable_type.to_string()),
        );
        record.insert("attachable_id", DbValue::Uuid(attachable_id));
        record.insert("collection", DbValue::Text(collection.to_string()));
        record.insert("disk", DbValue::Text("local".to_string()));
        record.insert("path", DbValue::Text("attachments/test.png".to_string()));
        record.insert("name", DbValue::Text("test.png".to_string()));
        record.insert("original_name", DbValue::Text("test.png".to_string()));
        record.insert("mime_type", DbValue::Text("image/png".to_string()));
        record.insert("size", DbValue::Int64(128));
        record.insert("sort_order", DbValue::Int32(0));
        record.insert("custom_properties", DbValue::Json(serde_json::json!({})));
        record
    }
}
