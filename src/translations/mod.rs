use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::database::extensions::{
    current_extension_scope, uuid_array_from_ids, AnyModelExtension, ModelExtensionLoader,
    TranslationCacheShape,
};
use crate::database::{
    ColumnRef, ComparisonOp, Condition, DbValue, Expr, OrderBy, Query, QueryExecutor, Sql, TableRef,
};
use crate::foundation::{AppContext, Error, Result};

tokio::task_local! {
    /// The current request's locale, set automatically by request middleware.
    pub static CURRENT_LOCALE: String;
}

/// Resolve the current locale: task_local request locale → i18n default → "en".
pub fn current_locale(app: &AppContext) -> String {
    CURRENT_LOCALE.try_with(|l| l.clone()).unwrap_or_else(|_| {
        app.i18n()
            .map(|m| m.default_locale().to_string())
            .unwrap_or_else(|_| "en".to_string())
    })
}

/// A single translation record from the `model_translations` table.
#[derive(Clone, Debug)]
pub struct ModelTranslation {
    pub id: String,
    pub translatable_type: String,
    pub translatable_id: String,
    pub locale: String,
    pub field: String,
    pub value: String,
}

/// A field's translations across all locales, with a resolved current-locale value.
///
/// ```ignore
/// let tf = product.translated_field(&app, "name").await?;
/// tf.translated            // "Red Shirt" (current locale)
/// tf.values["zh"]          // "红色衬衫"
/// tf.get("ms")             // Some("Baju Merah")
/// ```
#[derive(Clone, Debug)]
pub struct TranslatedFields {
    /// All locale values: `{"en": "Red Shirt", "zh": "红色衬衫"}`
    pub values: HashMap<String, String>,
    /// The resolved translation for the current request locale (with fallback).
    pub translated: String,
}

pub const MODEL_TRANSLATIONS_TABLE: &str = "model_translations";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslationJoin {
    alias: String,
}

impl TranslationJoin {
    pub fn new(alias: impl Into<String>) -> Self {
        Self {
            alias: alias.into(),
        }
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn table(&self) -> TableRef {
        TableRef::new(MODEL_TRANSLATIONS_TABLE).aliased(self.alias.clone())
    }

    pub fn column(&self, name: impl Into<String>) -> Expr {
        Expr::column(ColumnRef::new(self.alias.clone(), name.into()))
    }

    pub fn value(&self) -> Expr {
        self.column("value")
    }

    pub fn on<M>(
        &self,
        translatable_id: impl Into<Expr>,
        field: impl Into<String>,
        locale: impl Into<String>,
    ) -> Condition
    where
        M: HasTranslations,
    {
        Condition::and([
            self.column("translatable_id")
                .compare(ComparisonOp::Eq, translatable_id.into()),
            self.column("translatable_type")
                .eq_value(M::translatable_type()),
            self.column("field").eq_value(field.into()),
            self.column("locale").eq_value(locale.into()),
        ])
    }
}

pub fn translation_join(alias: impl Into<String>) -> TranslationJoin {
    TranslationJoin::new(alias)
}

pub(crate) fn translated_field_extension_loader<M>(field: String) -> AnyModelExtension<M>
where
    M: HasTranslations + Send + Sync + 'static,
{
    Arc::new(TranslationExtensionLoader {
        shape: TranslationCacheShape::Field { field },
        _model: PhantomData,
    })
}

pub(crate) fn translations_for_extension_loader<M>(locale: String) -> AnyModelExtension<M>
where
    M: HasTranslations + Send + Sync + 'static,
{
    Arc::new(TranslationExtensionLoader {
        shape: TranslationCacheShape::Locale { locale },
        _model: PhantomData,
    })
}

pub(crate) fn all_translations_extension_loader<M>() -> AnyModelExtension<M>
where
    M: HasTranslations + Send + Sync + 'static,
{
    Arc::new(TranslationExtensionLoader {
        shape: TranslationCacheShape::All,
        _model: PhantomData,
    })
}

struct TranslationExtensionLoader<M> {
    shape: TranslationCacheShape,
    _model: PhantomData<fn() -> M>,
}

#[async_trait]
impl<M> ModelExtensionLoader<M> for TranslationExtensionLoader<M>
where
    M: HasTranslations + Send + Sync + 'static,
{
    async fn load(&self, executor: &dyn QueryExecutor, models: &[M]) -> Result<()> {
        let Some(scope) = current_extension_scope() else {
            return Ok(());
        };

        let ids = collect_unique_ids(models.iter().map(|model| model.translatable_id()));
        if ids.is_empty() {
            return Ok(());
        }

        let translatable_type = M::translatable_type();
        let missing_ids = scope.missing_translation_ids(translatable_type, &self.shape, &ids);
        if missing_ids.is_empty() {
            return Ok(());
        }

        let rows =
            load_translation_rows(executor, translatable_type, &self.shape, &missing_ids).await?;
        scope.store_translations(translatable_type, self.shape.clone(), &missing_ids, rows);
        Ok(())
    }
}

impl TranslatedFields {
    /// Build from a list of (locale, value) pairs, resolving `translated`.
    pub fn from_entries(
        entries: Vec<(String, String)>,
        current_locale: &str,
        default_locale: &str,
    ) -> Self {
        let values: HashMap<String, String> = entries.into_iter().collect();
        let translated = values
            .get(current_locale)
            .or_else(|| values.get(default_locale))
            .or_else(|| values.values().next())
            .cloned()
            .unwrap_or_default();
        Self { values, translated }
    }

    /// Get a specific locale's value.
    pub fn get(&self, locale: &str) -> Option<&str> {
        self.values.get(locale).map(|s| s.as_str())
    }
}

/// Trait for models with translatable fields stored in the `model_translations` table.
///
/// ```ignore
/// impl HasTranslations for Product {
///     fn translatable_type() -> &'static str { "products" }
///     fn translatable_id(&self) -> String { self.id.to_string() }
/// }
///
/// product.set_translation(&app, "zh", "name", "红色衬衫").await?;
/// let name = product.translated_field(&app, "name").await?;
/// name.translated  // current locale value
/// ```
#[async_trait::async_trait]
pub trait HasTranslations: Send + Sync {
    fn translatable_type() -> &'static str;
    fn translatable_id(&self) -> String;

    async fn set_translation(
        &self,
        app: &AppContext,
        locale: &str,
        field: &str,
        value: &str,
    ) -> Result<()> {
        let db = app.database()?;
        self.set_translation_with(&*db, locale, field, value).await
    }

    async fn set_translation_with<E>(
        &self,
        executor: &E,
        locale: &str,
        field: &str,
        value: &str,
    ) -> Result<()>
    where
        E: QueryExecutor,
    {
        let translatable_id = self.translatable_id();
        let translatable_uuid = parse_translatable_uuid(&translatable_id)?;

        Query::insert_into(MODEL_TRANSLATIONS_TABLE)
            .values([
                (
                    "translatable_type",
                    DbValue::Text(Self::translatable_type().to_string()),
                ),
                ("translatable_id", DbValue::Uuid(translatable_uuid)),
                ("locale", DbValue::Text(locale.to_string())),
                ("field", DbValue::Text(field.to_string())),
                ("value", DbValue::Text(value.to_string())),
            ])
            .on_conflict_columns(["translatable_type", "translatable_id", "locale", "field"])
            .do_update()
            .set_excluded("value")
            .set_expr("updated_at", Sql::now())
            .execute(executor)
            .await?;

        invalidate_translation_cache(Self::translatable_type(), &translatable_id);
        Ok(())
    }

    async fn set_translations(
        &self,
        app: &AppContext,
        locale: &str,
        values: &[(&str, &str)],
    ) -> Result<()> {
        for (field, value) in values {
            self.set_translation(app, locale, field, value).await?;
        }
        Ok(())
    }

    async fn set_translations_with<E>(
        &self,
        executor: &E,
        locale: &str,
        values: &[(&str, &str)],
    ) -> Result<()>
    where
        E: QueryExecutor,
    {
        for (field, value) in values {
            self.set_translation_with(executor, locale, field, value)
                .await?;
        }
        Ok(())
    }

    async fn translation(
        &self,
        app: &AppContext,
        locale: &str,
        field: &str,
    ) -> Result<Option<String>> {
        let shape = TranslationCacheShape::Single {
            locale: locale.to_string(),
            field: field.to_string(),
        };
        if let Some(rows) = cached_translations_for_id(
            app,
            Self::translatable_type(),
            &self.translatable_id(),
            &shape,
        )
        .await?
        {
            return Ok(rows.into_iter().next().map(|row| row.value));
        }

        let translatable_id = self.translatable_id();
        let rows = translation_select_query()
            .where_eq("translatable_type", Self::translatable_type())
            .where_eq(
                "translatable_id",
                parse_translatable_uuid(&translatable_id)?,
            )
            .where_eq("locale", locale.to_string())
            .where_eq("field", field.to_string())
            .get(&*app.database()?)
            .await?;
        match rows.first() {
            Some(row) => match row.get("value") {
                Some(DbValue::Text(s)) => Ok(Some(s.clone())),
                _ => Ok(None),
            },
            None => Ok(None),
        }
    }

    async fn translations_for(
        &self,
        app: &AppContext,
        locale: &str,
    ) -> Result<HashMap<String, String>> {
        let shape = TranslationCacheShape::Locale {
            locale: locale.to_string(),
        };
        if let Some(rows) = cached_translations_for_id(
            app,
            Self::translatable_type(),
            &self.translatable_id(),
            &shape,
        )
        .await?
        {
            return Ok(rows.into_iter().map(|row| (row.field, row.value)).collect());
        }

        let translatable_id = self.translatable_id();
        let rows = translation_select_query()
            .where_eq("translatable_type", Self::translatable_type())
            .where_eq(
                "translatable_id",
                parse_translatable_uuid(&translatable_id)?,
            )
            .where_eq("locale", locale.to_string())
            .get(&*app.database()?)
            .await?;
        let mut map = HashMap::new();
        for row in rows.iter() {
            if let (Some(DbValue::Text(field)), Some(DbValue::Text(value))) =
                (row.get("field"), row.get("value"))
            {
                map.insert(field.clone(), value.clone());
            }
        }
        Ok(map)
    }

    /// Get a `TranslatedFields` for a specific field across all locales.
    ///
    /// The `translated` value is resolved using the current request locale
    /// (via `CURRENT_LOCALE` task_local), falling back to the i18n default locale.
    async fn translated_field(&self, app: &AppContext, field: &str) -> Result<TranslatedFields> {
        let shape = TranslationCacheShape::Field {
            field: field.to_string(),
        };
        if let Some(rows) = cached_translations_for_id(
            app,
            Self::translatable_type(),
            &self.translatable_id(),
            &shape,
        )
        .await?
        {
            return Ok(translated_fields_from_rows(app, rows));
        }

        let translatable_id = self.translatable_id();
        let rows = translation_select_query()
            .where_eq("translatable_type", Self::translatable_type())
            .where_eq(
                "translatable_id",
                parse_translatable_uuid(&translatable_id)?,
            )
            .where_eq("field", field.to_string())
            .get(&*app.database()?)
            .await?;
        let entries: Vec<(String, String)> = rows
            .iter()
            .filter_map(|row| match (row.get("locale"), row.get("value")) {
                (Some(DbValue::Text(locale)), Some(DbValue::Text(value))) => {
                    Some((locale.clone(), value.clone()))
                }
                _ => None,
            })
            .collect();
        let cur = current_locale(app);
        let default = app
            .i18n()
            .map(|m| m.default_locale().to_string())
            .unwrap_or_else(|_| "en".to_string());
        Ok(TranslatedFields::from_entries(entries, &cur, &default))
    }

    async fn all_translations(&self, app: &AppContext) -> Result<Vec<ModelTranslation>> {
        let shape = TranslationCacheShape::All;
        if let Some(rows) = cached_translations_for_id(
            app,
            Self::translatable_type(),
            &self.translatable_id(),
            &shape,
        )
        .await?
        {
            return Ok(rows);
        }

        let translatable_id = self.translatable_id();
        let rows = order_translation_rows(
            translation_select_query()
                .where_eq("translatable_type", Self::translatable_type())
                .where_eq(
                    "translatable_id",
                    parse_translatable_uuid(&translatable_id)?,
                ),
        )
        .get(&*app.database()?)
        .await?;
        rows.iter().map(row_to_model_translation).collect()
    }

    async fn delete_translations(&self, app: &AppContext, locale: &str) -> Result<u64> {
        let db = app.database()?;
        self.delete_translations_with(&*db, locale).await
    }

    async fn delete_translations_with<E>(&self, executor: &E, locale: &str) -> Result<u64>
    where
        E: QueryExecutor,
    {
        let translatable_id = self.translatable_id();
        let affected = Query::delete_from(MODEL_TRANSLATIONS_TABLE)
            .where_eq("translatable_type", Self::translatable_type())
            .where_eq(
                "translatable_id",
                parse_translatable_uuid(&translatable_id)?,
            )
            .where_eq("locale", locale.to_string())
            .execute(executor)
            .await?;
        invalidate_translation_cache(Self::translatable_type(), &translatable_id);
        Ok(affected)
    }

    async fn delete_translation_field(&self, app: &AppContext, field: &str) -> Result<u64> {
        let db = app.database()?;
        self.delete_translation_field_with(&*db, field).await
    }

    async fn delete_translation_field_with<E>(&self, executor: &E, field: &str) -> Result<u64>
    where
        E: QueryExecutor,
    {
        let translatable_id = self.translatable_id();
        let affected = Query::delete_from(MODEL_TRANSLATIONS_TABLE)
            .where_eq("translatable_type", Self::translatable_type())
            .where_eq(
                "translatable_id",
                parse_translatable_uuid(&translatable_id)?,
            )
            .where_eq("field", field.to_string())
            .execute(executor)
            .await?;
        invalidate_translation_cache(Self::translatable_type(), &translatable_id);
        Ok(affected)
    }

    async fn delete_all_translations(&self, app: &AppContext) -> Result<u64> {
        let db = app.database()?;
        self.delete_all_translations_with(&*db).await
    }

    async fn delete_all_translations_with<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor,
    {
        let translatable_id = self.translatable_id();
        let affected = Query::delete_from(MODEL_TRANSLATIONS_TABLE)
            .where_eq("translatable_type", Self::translatable_type())
            .where_eq(
                "translatable_id",
                parse_translatable_uuid(&translatable_id)?,
            )
            .execute(executor)
            .await?;
        invalidate_translation_cache(Self::translatable_type(), &translatable_id);
        Ok(affected)
    }
}

async fn cached_translations_for_id(
    executor: &dyn QueryExecutor,
    translatable_type: &str,
    translatable_id: &str,
    shape: &TranslationCacheShape,
) -> Result<Option<Vec<ModelTranslation>>> {
    let Some(scope) = current_extension_scope() else {
        return Ok(None);
    };

    if let Some(rows) = scope.cached_translations(translatable_type, shape, translatable_id) {
        return Ok(Some(rows));
    }

    let missing_ids =
        scope.missing_translation_ids_for_known(translatable_type, shape, translatable_id);
    if !missing_ids.is_empty() {
        let rows = load_translation_rows(executor, translatable_type, shape, &missing_ids).await?;
        scope.store_translations(translatable_type, shape.clone(), &missing_ids, rows);
    }

    Ok(Some(
        scope
            .cached_translations(translatable_type, shape, translatable_id)
            .unwrap_or_default(),
    ))
}

async fn load_translation_rows(
    executor: &dyn QueryExecutor,
    translatable_type: &str,
    shape: &TranslationCacheShape,
    translatable_ids: &[String],
) -> Result<Vec<ModelTranslation>> {
    if translatable_ids.is_empty() {
        return Ok(Vec::new());
    }

    let ids = uuid_array_from_ids(translatable_ids)?;
    let base = translation_select_query()
        .where_eq("translatable_type", translatable_type.to_string())
        .where_in("translatable_id", ids);
    let query = match shape {
        TranslationCacheShape::Single { locale, field } => base
            .where_eq("locale", locale.clone())
            .where_eq("field", field.clone()),
        TranslationCacheShape::Locale { locale } => base.where_eq("locale", locale.clone()),
        TranslationCacheShape::Field { field } => base.where_eq("field", field.clone()),
        TranslationCacheShape::All => base,
    };
    let rows = order_translation_rows(query).get(executor).await?;

    rows.iter().map(row_to_model_translation).collect()
}

fn parse_translatable_uuid(id: &str) -> Result<Uuid> {
    Uuid::parse_str(id).map_err(|error| {
        Error::message(format!(
            "model translation expected UUID translatable_id `{id}`: {error}"
        ))
    })
}

fn translation_select_query() -> Query {
    Query::table(MODEL_TRANSLATIONS_TABLE).select([
        "id",
        "translatable_type",
        "translatable_id",
        "locale",
        "field",
        "value",
    ])
}

fn order_translation_rows(query: Query) -> Query {
    query
        .order_by(OrderBy::asc("translatable_id"))
        .order_by(OrderBy::asc("field"))
        .order_by(OrderBy::asc("locale"))
}

fn row_to_model_translation(row: &crate::database::DbRecord) -> Result<ModelTranslation> {
    Ok(ModelTranslation {
        id: row.try_text_or_uuid("id")?,
        translatable_type: row.try_text("translatable_type")?,
        translatable_id: row.try_text_or_uuid("translatable_id")?,
        locale: row.try_text("locale")?,
        field: row.try_text("field")?,
        value: row.try_text("value")?,
    })
}

fn translated_fields_from_rows(app: &AppContext, rows: Vec<ModelTranslation>) -> TranslatedFields {
    let entries = rows
        .into_iter()
        .map(|row| (row.locale, row.value))
        .collect();
    let cur = current_locale(app);
    let default = app
        .i18n()
        .map(|m| m.default_locale().to_string())
        .unwrap_or_else(|_| "en".to_string());
    TranslatedFields::from_entries(entries, &cur, &default)
}

fn invalidate_translation_cache(translatable_type: &str, translatable_id: &str) {
    if let Some(scope) = current_extension_scope() {
        scope.invalidate_translations(translatable_type, translatable_id);
    }
}

fn collect_unique_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    ids.into_iter()
        .filter(|id| !id.trim().is_empty())
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use uuid::Uuid;

    use crate::database::{
        scope_model_extensions, DbRecord, DbValue, PostgresCompiler, Query, QueryExecutionOptions,
        QueryExecutor, Sql,
    };

    use super::*;

    #[derive(Default)]
    struct CountingTranslationExecutor {
        query_count: AtomicUsize,
    }

    #[async_trait]
    impl QueryExecutor for CountingTranslationExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            self.query_count.fetch_add(1, Ordering::SeqCst);
            let translatable_type = match &bindings[0] {
                DbValue::Text(value) => value.clone(),
                _ => panic!("expected translatable_type binding"),
            };
            let ids: Vec<Uuid> = bindings
                .iter()
                .filter_map(|binding| match binding {
                    DbValue::Uuid(value) => Some(*value),
                    _ => None,
                })
                .collect();
            assert!(!ids.is_empty(), "expected translatable_id uuid bindings");
            let field = bindings
                .iter()
                .rev()
                .find_map(|binding| match binding {
                    DbValue::Text(value) if value != &translatable_type => Some(value.clone()),
                    _ => None,
                })
                .expect("expected field binding");

            Ok(ids
                .into_iter()
                .map(|id| translation_record(&translatable_type, id, &field))
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

    #[tokio::test]
    async fn lazy_translation_cache_batches_known_scope_ids() {
        let executor = CountingTranslationExecutor::default();
        let first_id = Uuid::now_v7().to_string();
        let second_id = Uuid::now_v7().to_string();
        let shape = TranslationCacheShape::Field {
            field: "name".to_string(),
        };

        scope_model_extensions(async {
            current_extension_scope()
                .unwrap()
                .register_model_ids("test_translatables", [first_id.clone(), second_id.clone()]);

            let first =
                cached_translations_for_id(&executor, "test_translatables", &first_id, &shape)
                    .await
                    .unwrap()
                    .unwrap();
            let second =
                cached_translations_for_id(&executor, "test_translatables", &second_id, &shape)
                    .await
                    .unwrap()
                    .unwrap();

            assert_eq!(executor.query_count.load(Ordering::SeqCst), 1);
            assert_eq!(first[0].translatable_id, first_id);
            assert_eq!(second[0].translatable_id, second_id);
        })
        .await;
    }

    #[tokio::test]
    async fn translation_write_helpers_use_executor_and_invalidate_scope_cache() {
        let executor = RecordingTranslationExecutor::default();
        let translatable = TestTranslatable {
            id: Uuid::now_v7().to_string(),
        };
        let shape = TranslationCacheShape::All;

        scope_model_extensions(async {
            current_extension_scope().unwrap().store_translations(
                TestTranslatable::translatable_type(),
                shape.clone(),
                &[translatable.translatable_id()],
                vec![ModelTranslation {
                    id: Uuid::now_v7().to_string(),
                    translatable_type: TestTranslatable::translatable_type().to_string(),
                    translatable_id: translatable.translatable_id(),
                    locale: "en".to_string(),
                    field: "name".to_string(),
                    value: "Cached".to_string(),
                }],
            );

            assert!(current_extension_scope()
                .unwrap()
                .cached_translations(
                    TestTranslatable::translatable_type(),
                    &shape,
                    &translatable.translatable_id()
                )
                .is_some());

            translatable
                .set_translation_with(&executor, "en", "name", "Desk")
                .await
                .unwrap();
            translatable
                .delete_translation_field_with(&executor, "name")
                .await
                .unwrap();
            translatable
                .delete_all_translations_with(&executor)
                .await
                .unwrap();

            assert!(current_extension_scope()
                .unwrap()
                .cached_translations(
                    TestTranslatable::translatable_type(),
                    &shape,
                    &translatable.translatable_id()
                )
                .is_none());
        })
        .await;

        let calls = executor.execute_calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls[0].0.contains("ON CONFLICT"));
        assert_eq!(
            calls[0].1,
            vec![
                DbValue::Text("test_translatables".to_string()),
                DbValue::Uuid(Uuid::parse_str(&translatable.id).unwrap()),
                DbValue::Text("en".to_string()),
                DbValue::Text("name".to_string()),
                DbValue::Text("Desk".to_string()),
            ]
        );
        assert!(calls[1].0.contains("\"field\" = $3::text"));
        assert!(calls[2].0.contains("\"translatable_type\" = $1::text"));
    }

    #[test]
    fn translation_join_builds_table_value_and_on_condition() {
        let join = translation_join("product_names");
        let query = Query::table("products")
            .left_join(
                join.table(),
                join.on::<TestTranslatable>(ColumnRef::new("products", "id"), "name", "en"),
            )
            .select_expr(Sql::coalesce([join.value(), Expr::text("")]), "name");

        let compiled = PostgresCompiler::compile(query.ast()).unwrap();

        assert!(compiled
            .sql
            .contains("LEFT JOIN \"model_translations\" AS \"product_names\" ON"));
        assert!(compiled
            .sql
            .contains("\"product_names\".\"translatable_id\" = \"products\".\"id\""));
        assert!(compiled
            .sql
            .contains("\"product_names\".\"translatable_type\" = $2::text"));
        assert!(compiled
            .sql
            .contains("\"product_names\".\"field\" = $3::text"));
        assert!(compiled
            .sql
            .contains("\"product_names\".\"locale\" = $4::text"));
        assert!(compiled
            .sql
            .contains("COALESCE(\"product_names\".\"value\", $1::text) AS \"name\""));
    }

    fn translation_record(translatable_type: &str, translatable_id: Uuid, field: &str) -> DbRecord {
        let mut record = DbRecord::new();
        record.insert("id", DbValue::Uuid(Uuid::now_v7()));
        record.insert(
            "translatable_type",
            DbValue::Text(translatable_type.to_string()),
        );
        record.insert("translatable_id", DbValue::Uuid(translatable_id));
        record.insert("locale", DbValue::Text("en".to_string()));
        record.insert("field", DbValue::Text(field.to_string()));
        record.insert("value", DbValue::Text("Translated".to_string()));
        record
    }

    struct TestTranslatable {
        id: String,
    }

    impl HasTranslations for TestTranslatable {
        fn translatable_type() -> &'static str {
            "test_translatables"
        }

        fn translatable_id(&self) -> String {
            self.id.clone()
        }
    }

    #[derive(Default)]
    struct RecordingTranslationExecutor {
        execute_calls: Mutex<Vec<(String, Vec<DbValue>)>>,
    }

    #[async_trait]
    impl QueryExecutor for RecordingTranslationExecutor {
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
}
