use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use uuid::Uuid;

use crate::attachments::Attachment;
use crate::foundation::{Error, Result};
use crate::metadata::ModelMeta;
use crate::support::sync::lock_unpoisoned;
use crate::translations::ModelTranslation;

use super::runtime::{DbRecord, QueryExecutor};
use super::TableMeta;

#[async_trait]
pub(crate) trait ModelExtensionLoader<M>: Send + Sync {
    async fn load(&self, executor: &dyn QueryExecutor, models: &[M]) -> Result<()>;
}

pub(crate) type AnyModelExtension<M> = Arc<dyn ModelExtensionLoader<M>>;

#[derive(Clone)]
pub(crate) struct ModelExtensionScope {
    inner: Arc<Mutex<ModelExtensionCache>>,
}

#[derive(Default)]
struct ModelExtensionCache {
    model_ids: HashMap<String, BTreeSet<String>>,
    attachments: HashMap<AttachmentCacheKey, AttachmentCacheEntry>,
    translations: HashMap<TranslationCacheKey, TranslationCacheEntry>,
    metadata: HashMap<MetadataCacheKey, MetadataCacheEntry>,
}

#[derive(Clone, Debug, Eq)]
struct AttachmentCacheKey {
    attachable_type: String,
    collection: String,
}

#[derive(Default)]
struct AttachmentCacheEntry {
    loaded_ids: BTreeSet<String>,
    rows_by_id: HashMap<String, Vec<Attachment>>,
}

#[derive(Clone, Debug, Eq)]
struct TranslationCacheKey {
    translatable_type: String,
    shape: TranslationCacheShape,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TranslationCacheShape {
    Single { locale: String, field: String },
    Locale { locale: String },
    Field { field: String },
    All,
}

#[derive(Default)]
struct TranslationCacheEntry {
    loaded_ids: BTreeSet<String>,
    rows_by_id: HashMap<String, Vec<ModelTranslation>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MetadataCacheShape {
    Key(String),
    All,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MetadataCacheKey {
    metadatable_type: String,
    shape: MetadataCacheShape,
}

#[derive(Default)]
struct MetadataCacheEntry {
    loaded_ids: BTreeSet<String>,
    rows_by_id: HashMap<String, Vec<ModelMeta>>,
}

tokio::task_local! {
    static CURRENT_MODEL_EXTENSION_SCOPE: ModelExtensionScope;
}

pub async fn scope_model_extensions<F, T>(future: F) -> T
where
    F: Future<Output = T>,
{
    if CURRENT_MODEL_EXTENSION_SCOPE.try_with(|_| ()).is_ok() {
        return future.await;
    }

    CURRENT_MODEL_EXTENSION_SCOPE
        .scope(ModelExtensionScope::new(), future)
        .await
}

pub(crate) fn current_extension_scope() -> Option<ModelExtensionScope> {
    CURRENT_MODEL_EXTENSION_SCOPE.try_with(Clone::clone).ok()
}

pub(crate) fn register_model_records<M>(table: &'static TableMeta<M>, records: &[DbRecord]) {
    let Some(scope) = current_extension_scope() else {
        return;
    };

    let ids = records
        .iter()
        .filter_map(|record| id_from_record(record, table.primary_key_name()))
        .collect::<Vec<_>>();
    scope.register_model_ids(table.name(), ids);
}

pub(crate) fn uuid_array_from_ids(ids: &[String]) -> Result<Vec<Uuid>> {
    ids.iter()
        .map(|id| {
            Uuid::parse_str(id).map_err(|error| {
                Error::message(format!(
                    "model extension batching expected UUID id `{id}`: {error}"
                ))
            })
        })
        .collect()
}

impl ModelExtensionScope {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ModelExtensionCache::default())),
        }
    }

    pub(crate) fn register_model_ids(
        &self,
        model_type: &str,
        ids: impl IntoIterator<Item = String>,
    ) {
        let ids = ids
            .into_iter()
            .filter(|id| !id.trim().is_empty())
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return;
        }

        let mut cache = self.lock();
        cache.insert_model_ids(model_type, ids.clone());

        let basename = model_type_basename(model_type);
        if basename != model_type {
            cache.insert_model_ids(basename, ids);
        }
    }

    pub(crate) fn cached_attachments(
        &self,
        attachable_type: &str,
        collection: &str,
        attachable_id: &str,
    ) -> Option<Vec<Attachment>> {
        let cache = self.lock();
        let key = AttachmentCacheKey::new(attachable_type, collection);
        let entry = cache.attachments.get(&key)?;
        if !entry.loaded_ids.contains(attachable_id) {
            return None;
        }
        Some(
            entry
                .rows_by_id
                .get(attachable_id)
                .cloned()
                .unwrap_or_default(),
        )
    }

    pub(crate) fn missing_attachment_ids_for_known(
        &self,
        attachable_type: &str,
        collection: &str,
        current_id: &str,
    ) -> Vec<String> {
        let mut cache = self.lock();
        cache.insert_model_ids(attachable_type, [current_id.to_string()]);
        let known_ids = cache.known_ids(attachable_type);
        cache.missing_attachment_ids(attachable_type, collection, known_ids)
    }

    pub(crate) fn missing_attachment_ids(
        &self,
        attachable_type: &str,
        collection: &str,
        ids: &[String],
    ) -> Vec<String> {
        let mut cache = self.lock();
        cache.insert_model_ids(attachable_type, ids.iter().cloned());
        cache.missing_attachment_ids(attachable_type, collection, ids.to_vec())
    }

    pub(crate) fn store_attachments(
        &self,
        attachable_type: &str,
        collection: &str,
        ids: &[String],
        rows: Vec<Attachment>,
    ) {
        let mut cache = self.lock();
        let key = AttachmentCacheKey::new(attachable_type, collection);
        let entry = cache.attachments.entry(key).or_default();
        let mut grouped: HashMap<String, Vec<Attachment>> = HashMap::new();
        for row in rows {
            grouped
                .entry(row.attachable_id.clone())
                .or_default()
                .push(row);
        }

        for id in ids {
            entry.loaded_ids.insert(id.clone());
            entry
                .rows_by_id
                .insert(id.clone(), grouped.remove(id).unwrap_or_default());
        }
    }

    pub(crate) fn invalidate_attachments(&self, attachable_type: &str, attachable_id: &str) {
        let mut cache = self.lock();
        for (key, entry) in cache.attachments.iter_mut() {
            if key.attachable_type == attachable_type {
                entry.loaded_ids.remove(attachable_id);
                entry.rows_by_id.remove(attachable_id);
            }
        }
    }

    pub(crate) fn invalidate_attachment_collection(
        &self,
        attachable_type: &str,
        attachable_id: &str,
        collection: &str,
    ) {
        let mut cache = self.lock();
        let key = AttachmentCacheKey::new(attachable_type, collection);
        if let Some(entry) = cache.attachments.get_mut(&key) {
            entry.loaded_ids.remove(attachable_id);
            entry.rows_by_id.remove(attachable_id);
        }
    }

    pub(crate) fn cached_translations(
        &self,
        translatable_type: &str,
        shape: &TranslationCacheShape,
        translatable_id: &str,
    ) -> Option<Vec<ModelTranslation>> {
        let cache = self.lock();
        let key = TranslationCacheKey::new(translatable_type, shape.clone());
        let entry = cache.translations.get(&key)?;
        if !entry.loaded_ids.contains(translatable_id) {
            return None;
        }
        Some(
            entry
                .rows_by_id
                .get(translatable_id)
                .cloned()
                .unwrap_or_default(),
        )
    }

    pub(crate) fn missing_translation_ids_for_known(
        &self,
        translatable_type: &str,
        shape: &TranslationCacheShape,
        current_id: &str,
    ) -> Vec<String> {
        let mut cache = self.lock();
        cache.insert_model_ids(translatable_type, [current_id.to_string()]);
        let known_ids = cache.known_ids(translatable_type);
        cache.missing_translation_ids(translatable_type, shape, known_ids)
    }

    pub(crate) fn missing_translation_ids(
        &self,
        translatable_type: &str,
        shape: &TranslationCacheShape,
        ids: &[String],
    ) -> Vec<String> {
        let mut cache = self.lock();
        cache.insert_model_ids(translatable_type, ids.iter().cloned());
        cache.missing_translation_ids(translatable_type, shape, ids.to_vec())
    }

    pub(crate) fn store_translations(
        &self,
        translatable_type: &str,
        shape: TranslationCacheShape,
        ids: &[String],
        rows: Vec<ModelTranslation>,
    ) {
        let mut cache = self.lock();
        let key = TranslationCacheKey::new(translatable_type, shape);
        let entry = cache.translations.entry(key).or_default();
        let mut grouped: HashMap<String, Vec<ModelTranslation>> = HashMap::new();
        for row in rows {
            grouped
                .entry(row.translatable_id.clone())
                .or_default()
                .push(row);
        }

        for id in ids {
            entry.loaded_ids.insert(id.clone());
            entry
                .rows_by_id
                .insert(id.clone(), grouped.remove(id).unwrap_or_default());
        }
    }

    pub(crate) fn invalidate_translations(&self, translatable_type: &str, translatable_id: &str) {
        let mut cache = self.lock();
        for (key, entry) in cache.translations.iter_mut() {
            if key.translatable_type == translatable_type {
                entry.loaded_ids.remove(translatable_id);
                entry.rows_by_id.remove(translatable_id);
            }
        }
    }

    pub(crate) fn cached_metadata(
        &self,
        metadatable_type: &str,
        shape: &MetadataCacheShape,
        metadatable_id: &str,
    ) -> Option<Vec<ModelMeta>> {
        let cache = self.lock();
        let key = MetadataCacheKey {
            metadatable_type: metadatable_type.to_string(),
            shape: shape.clone(),
        };
        if let Some(entry) = cache.metadata.get(&key) {
            if entry.loaded_ids.contains(metadatable_id) {
                return Some(
                    entry
                        .rows_by_id
                        .get(metadatable_id)
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }

        let MetadataCacheShape::Key(requested_key) = shape else {
            return None;
        };
        let all_key = MetadataCacheKey {
            metadatable_type: metadatable_type.to_string(),
            shape: MetadataCacheShape::All,
        };
        let all_entry = cache.metadata.get(&all_key)?;
        if !all_entry.loaded_ids.contains(metadatable_id) {
            return None;
        }
        Some(
            all_entry
                .rows_by_id
                .get(metadatable_id)
                .into_iter()
                .flatten()
                .filter(|row| row.key == *requested_key)
                .cloned()
                .collect(),
        )
    }

    pub(crate) fn missing_metadata_ids_for_known(
        &self,
        metadatable_type: &str,
        shape: &MetadataCacheShape,
        current_id: &str,
    ) -> Vec<String> {
        let mut cache = self.lock();
        cache.insert_model_ids(metadatable_type, [current_id.to_string()]);
        let known_ids = cache.known_ids(metadatable_type);
        cache.missing_metadata_ids(metadatable_type, shape, known_ids)
    }

    pub(crate) fn missing_metadata_ids(
        &self,
        metadatable_type: &str,
        shape: &MetadataCacheShape,
        ids: &[String],
    ) -> Vec<String> {
        let mut cache = self.lock();
        cache.insert_model_ids(metadatable_type, ids.iter().cloned());
        cache.missing_metadata_ids(metadatable_type, shape, ids.to_vec())
    }

    pub(crate) fn store_metadata(
        &self,
        metadatable_type: &str,
        shape: MetadataCacheShape,
        ids: &[String],
        rows: Vec<ModelMeta>,
    ) {
        let mut cache = self.lock();
        let key = MetadataCacheKey {
            metadatable_type: metadatable_type.to_string(),
            shape,
        };
        let entry = cache.metadata.entry(key).or_default();
        let mut grouped: HashMap<String, Vec<ModelMeta>> = HashMap::new();
        for row in rows {
            grouped
                .entry(row.metadatable_id.clone())
                .or_default()
                .push(row);
        }
        for id in ids {
            entry.loaded_ids.insert(id.clone());
            entry
                .rows_by_id
                .insert(id.clone(), grouped.remove(id).unwrap_or_default());
        }
    }

    pub(crate) fn invalidate_metadata(&self, metadatable_type: &str, metadatable_id: &str) {
        let mut cache = self.lock();
        for (key, entry) in &mut cache.metadata {
            if key.metadatable_type == metadatable_type {
                entry.loaded_ids.remove(metadatable_id);
                entry.rows_by_id.remove(metadatable_id);
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ModelExtensionCache> {
        lock_unpoisoned(&self.inner, "model extension cache")
    }
}

impl ModelExtensionCache {
    fn insert_model_ids(&mut self, model_type: &str, ids: impl IntoIterator<Item = String>) {
        self.model_ids
            .entry(model_type.to_string())
            .or_default()
            .extend(ids.into_iter().filter(|id| !id.trim().is_empty()));
    }

    fn known_ids(&self, model_type: &str) -> Vec<String> {
        self.model_ids
            .get(model_type)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn missing_attachment_ids(
        &mut self,
        attachable_type: &str,
        collection: &str,
        ids: Vec<String>,
    ) -> Vec<String> {
        let key = AttachmentCacheKey::new(attachable_type, collection);
        let entry = self.attachments.entry(key).or_default();
        ids.into_iter()
            .filter(|id| !entry.loaded_ids.contains(id))
            .collect()
    }

    fn missing_translation_ids(
        &mut self,
        translatable_type: &str,
        shape: &TranslationCacheShape,
        ids: Vec<String>,
    ) -> Vec<String> {
        let key = TranslationCacheKey::new(translatable_type, shape.clone());
        let entry = self.translations.entry(key).or_default();
        ids.into_iter()
            .filter(|id| !entry.loaded_ids.contains(id))
            .collect()
    }

    fn missing_metadata_ids(
        &mut self,
        metadatable_type: &str,
        shape: &MetadataCacheShape,
        ids: Vec<String>,
    ) -> Vec<String> {
        let key = MetadataCacheKey {
            metadatable_type: metadatable_type.to_string(),
            shape: shape.clone(),
        };
        let exact_loaded = self.metadata.entry(key).or_default().loaded_ids.clone();
        let all_loaded = if matches!(shape, MetadataCacheShape::Key(_)) {
            self.metadata
                .get(&MetadataCacheKey {
                    metadatable_type: metadatable_type.to_string(),
                    shape: MetadataCacheShape::All,
                })
                .map(|entry| entry.loaded_ids.clone())
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        ids.into_iter()
            .filter(|id| !exact_loaded.contains(id) && !all_loaded.contains(id))
            .collect()
    }
}

impl AttachmentCacheKey {
    fn new(attachable_type: &str, collection: &str) -> Self {
        Self {
            attachable_type: attachable_type.to_string(),
            collection: collection.to_string(),
        }
    }
}

impl PartialEq for AttachmentCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.attachable_type == other.attachable_type && self.collection == other.collection
    }
}

impl Hash for AttachmentCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.attachable_type.hash(state);
        self.collection.hash(state);
    }
}

impl TranslationCacheKey {
    fn new(translatable_type: &str, shape: TranslationCacheShape) -> Self {
        Self {
            translatable_type: translatable_type.to_string(),
            shape,
        }
    }
}

impl PartialEq for TranslationCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.translatable_type == other.translatable_type && self.shape == other.shape
    }
}

impl Hash for TranslationCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.translatable_type.hash(state);
        self.shape.hash(state);
    }
}

fn id_from_record(record: &DbRecord, primary_key: &str) -> Option<String> {
    let id = record.try_text_or_uuid(primary_key).ok()?;
    (!id.is_empty()).then_some(id)
}

fn model_type_basename(model_type: &str) -> &str {
    model_type.rsplit('.').next().unwrap_or(model_type)
}
