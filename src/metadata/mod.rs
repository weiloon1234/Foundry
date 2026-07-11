use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::sync::Arc;

use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::database::extensions::{
    current_extension_scope, uuid_array_from_ids, AnyModelExtension, MetadataCacheShape,
    ModelExtensionLoader,
};
use crate::database::{DbType, DbValue, Model, OrderBy, Query, QueryExecutor, Sql};
use crate::foundation::{AppContext, Error, Result};

const METADATA_TABLE: &str = "metadata";

/// A metadata record — polymorphic key-value store.
#[derive(Clone, Debug)]
pub struct ModelMeta {
    pub id: String,
    pub metadatable_type: String,
    pub metadatable_id: String,
    pub key: String,
    pub value: Option<serde_json::Value>,
}

pub(crate) fn metadata_extension_loader<M>(shape: MetadataCacheShape) -> AnyModelExtension<M>
where
    M: HasMetadata + Send + Sync + 'static,
{
    Arc::new(MetadataExtensionLoader {
        shape,
        _model: PhantomData,
    })
}

struct MetadataExtensionLoader<M> {
    shape: MetadataCacheShape,
    _model: PhantomData<fn() -> M>,
}

#[async_trait::async_trait]
impl<M> ModelExtensionLoader<M> for MetadataExtensionLoader<M>
where
    M: HasMetadata + Send + Sync + 'static,
{
    async fn load(&self, executor: &dyn QueryExecutor, models: &[M]) -> Result<()> {
        let Some(scope) = current_extension_scope() else {
            return Ok(());
        };
        let ids = collect_unique_ids(models.iter().map(HasMetadata::metadatable_id));
        if ids.is_empty() {
            return Ok(());
        }
        let metadatable_type = M::metadatable_type();
        let missing_ids = scope.missing_metadata_ids(metadatable_type, &self.shape, &ids);
        if missing_ids.is_empty() {
            return Ok(());
        }
        let rows =
            load_metadata_rows(executor, metadatable_type, &self.shape, &missing_ids).await?;
        scope.store_metadata(metadatable_type, self.shape.clone(), &missing_ids, rows);
        Ok(())
    }
}

/// Trait for models that can have arbitrary key-value metadata.
///
/// ```ignore
/// impl HasMetadata for User {
///     fn metadatable_type() -> &'static str { "users" }
///     fn metadatable_id(&self) -> String { self.id.to_string() }
/// }
///
/// user.set_meta(&app, "theme", json!("dark")).await?;
/// let theme: String = user.get_meta(&app, "theme").await?.unwrap();
/// ```
#[async_trait::async_trait]
pub trait HasMetadata: Send + Sync {
    fn metadatable_type() -> &'static str;
    fn metadatable_id(&self) -> String;

    async fn set_meta(
        &self,
        app: &AppContext,
        key: &str,
        value: impl Serialize + Send,
    ) -> Result<()> {
        let db = app.database()?;
        let json_val = serde_json::to_value(value).map_err(Error::other)?;
        let metadatable_id = self.metadatable_id();
        Query::insert_into(METADATA_TABLE)
            .values([
                (
                    "metadatable_type",
                    DbValue::Text(Self::metadatable_type().to_string()),
                ),
                (
                    "metadatable_id",
                    DbValue::Uuid(parse_metadatable_uuid(&metadatable_id)?),
                ),
                ("key", DbValue::Text(key.to_string())),
                ("value", DbValue::Json(json_val)),
            ])
            .on_conflict_columns(["metadatable_type", "metadatable_id", "key"])
            .do_update()
            .set_excluded("value")
            .set_expr("updated_at", Sql::now())
            .execute(&*db)
            .await?;
        invalidate_metadata_cache(Self::metadatable_type(), &metadatable_id);
        Ok(())
    }

    async fn get_meta<T: DeserializeOwned>(
        &self,
        app: &AppContext,
        key: &str,
    ) -> Result<Option<T>> {
        match self.get_meta_raw(app, key).await? {
            Some(v) => Ok(Some(serde_json::from_value(v).map_err(Error::other)?)),
            None => Ok(None),
        }
    }

    async fn get_meta_raw(&self, app: &AppContext, key: &str) -> Result<Option<serde_json::Value>> {
        let metadatable_id = self.metadatable_id();
        let database = app.database()?;
        let shape = MetadataCacheShape::Key(key.to_string());
        let rows = match cached_metadata_for_id(
            database.as_ref(),
            Self::metadatable_type(),
            &metadatable_id,
            &shape,
        )
        .await?
        {
            Some(rows) => rows,
            None => {
                load_metadata_rows(
                    database.as_ref(),
                    Self::metadatable_type(),
                    &shape,
                    std::slice::from_ref(&metadatable_id),
                )
                .await?
            }
        };
        Ok(rows.into_iter().next().and_then(|row| row.value))
    }

    async fn forget_meta(&self, app: &AppContext, key: &str) -> Result<bool> {
        let metadatable_id = self.metadatable_id();
        let affected = Query::delete_from(METADATA_TABLE)
            .where_eq("metadatable_type", Self::metadatable_type())
            .where_eq("metadatable_id", parse_metadatable_uuid(&metadatable_id)?)
            .where_eq("key", key.to_string())
            .execute(&*app.database()?)
            .await?;
        invalidate_metadata_cache(Self::metadatable_type(), &metadatable_id);
        Ok(affected > 0)
    }

    async fn has_meta(&self, app: &AppContext, key: &str) -> Result<bool> {
        let metadatable_id = self.metadatable_id();
        let database = app.database()?;
        let shape = MetadataCacheShape::Key(key.to_string());
        let rows = match cached_metadata_for_id(
            database.as_ref(),
            Self::metadatable_type(),
            &metadatable_id,
            &shape,
        )
        .await?
        {
            Some(rows) => rows,
            None => {
                load_metadata_rows(
                    database.as_ref(),
                    Self::metadatable_type(),
                    &shape,
                    std::slice::from_ref(&metadatable_id),
                )
                .await?
            }
        };
        Ok(!rows.is_empty())
    }

    async fn all_meta(&self, app: &AppContext) -> Result<Vec<ModelMeta>> {
        let metadatable_id = self.metadatable_id();
        let database = app.database()?;
        let shape = MetadataCacheShape::All;
        match cached_metadata_for_id(
            database.as_ref(),
            Self::metadatable_type(),
            &metadatable_id,
            &shape,
        )
        .await?
        {
            Some(rows) => Ok(rows),
            None => {
                load_metadata_rows(
                    database.as_ref(),
                    Self::metadatable_type(),
                    &shape,
                    std::slice::from_ref(&metadatable_id),
                )
                .await
            }
        }
    }

    async fn delete_all_meta(&self, app: &AppContext) -> Result<u64> {
        let database = app.database()?;
        self.delete_all_meta_with(database.as_ref()).await
    }

    async fn delete_all_meta_with<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor,
    {
        let metadatable_id = self.metadatable_id();
        let affected = Query::delete_from(METADATA_TABLE)
            .where_eq("metadatable_type", Self::metadatable_type())
            .where_eq("metadatable_id", parse_metadatable_uuid(&metadatable_id)?)
            .execute(executor)
            .await?;
        invalidate_metadata_cache(Self::metadatable_type(), &metadatable_id);
        Ok(affected)
    }
}

async fn cached_metadata_for_id(
    executor: &dyn QueryExecutor,
    metadatable_type: &str,
    metadatable_id: &str,
    shape: &MetadataCacheShape,
) -> Result<Option<Vec<ModelMeta>>> {
    let Some(scope) = current_extension_scope() else {
        return Ok(None);
    };
    if let Some(rows) = scope.cached_metadata(metadatable_type, shape, metadatable_id) {
        return Ok(Some(rows));
    }
    let missing_ids = scope.missing_metadata_ids_for_known(metadatable_type, shape, metadatable_id);
    if !missing_ids.is_empty() {
        let rows = load_metadata_rows(executor, metadatable_type, shape, &missing_ids).await?;
        scope.store_metadata(metadatable_type, shape.clone(), &missing_ids, rows);
    }
    Ok(Some(
        scope
            .cached_metadata(metadatable_type, shape, metadatable_id)
            .unwrap_or_default(),
    ))
}

async fn load_metadata_rows(
    executor: &dyn QueryExecutor,
    metadatable_type: &str,
    shape: &MetadataCacheShape,
    metadatable_ids: &[String],
) -> Result<Vec<ModelMeta>> {
    if metadatable_ids.is_empty() {
        return Ok(Vec::new());
    }
    let base = Query::table(METADATA_TABLE)
        .select(["id", "metadatable_type", "metadatable_id", "key", "value"])
        .where_eq("metadatable_type", metadatable_type.to_string())
        .where_in("metadatable_id", uuid_array_from_ids(metadatable_ids)?);
    let query = match shape {
        MetadataCacheShape::Key(key) => base.where_eq("key", key.clone()),
        MetadataCacheShape::All => base,
    };
    query
        .order_by(OrderBy::asc("metadatable_id"))
        .order_by(OrderBy::asc("key"))
        .get(executor)
        .await?
        .iter()
        .map(row_to_model_meta)
        .collect()
}

fn row_to_model_meta(row: &crate::database::DbRecord) -> Result<ModelMeta> {
    Ok(ModelMeta {
        id: row.try_text_or_uuid("id")?,
        metadatable_type: row.try_text("metadatable_type")?,
        metadatable_id: row.try_text_or_uuid("metadatable_id")?,
        key: row.try_text("key")?,
        value: match row.get("value") {
            Some(DbValue::Json(value)) => Some(value.clone()),
            Some(DbValue::Null(_)) | None => None,
            Some(_) => return Err(Error::message("metadata value is not JSON")),
        },
    })
}

fn invalidate_metadata_cache(metadatable_type: &str, metadatable_id: &str) {
    if let Some(scope) = current_extension_scope() {
        scope.invalidate_metadata(metadatable_type, metadatable_id);
    }
}

fn collect_unique_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| !id.trim().is_empty())
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// Declares the owner table used for metadata orphan audits and pruning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataOwner {
    metadatable_type: String,
    table: String,
    primary_key: String,
}

impl MetadataOwner {
    pub fn new(
        metadatable_type: impl Into<String>,
        table: impl Into<String>,
        primary_key: impl Into<String>,
    ) -> Result<Self> {
        let metadatable_type = metadatable_type.into();
        let table = table.into();
        let primary_key = primary_key.into();
        if metadatable_type.trim().is_empty() {
            return Err(Error::message("metadata owner type cannot be empty"));
        }
        validate_identifier("table", &table)?;
        validate_identifier("primary key", &primary_key)?;
        Ok(Self {
            metadatable_type,
            table,
            primary_key,
        })
    }

    pub fn for_model<M>() -> Result<Self>
    where
        M: Model + HasMetadata,
    {
        let table = M::table_meta();
        let primary_key = table
            .primary_key_column_info()
            .ok_or_else(|| Error::message("metadata owner model has no primary key"))?;
        if primary_key.db_type != DbType::Uuid {
            return Err(Error::message(format!(
                "metadata owner `{}` must use a UUID primary key",
                table.name()
            )));
        }
        Self::new(M::metadatable_type(), table.name(), primary_key.name)
    }

    pub fn metadatable_type(&self) -> &str {
        &self.metadatable_type
    }

    pub fn table(&self) -> &str {
        &self.table
    }

    pub fn primary_key(&self) -> &str {
        &self.primary_key
    }
}

/// Count metadata rows whose declared owner row no longer exists.
pub async fn audit_metadata_orphans<E>(executor: &E, owner: &MetadataOwner) -> Result<u64>
where
    E: QueryExecutor + ?Sized,
{
    let sql = metadata_orphan_sql(owner, false);
    let rows = executor
        .raw_query(&sql, &[DbValue::Text(owner.metadatable_type.clone())])
        .await?;
    let count: i64 = rows
        .first()
        .ok_or_else(|| Error::message("metadata orphan audit returned no count row"))?
        .decode("orphan_count")?;
    u64::try_from(count).map_err(Error::other)
}

/// Delete metadata rows whose declared owner row no longer exists.
pub async fn prune_metadata_orphans<E>(executor: &E, owner: &MetadataOwner) -> Result<u64>
where
    E: QueryExecutor + ?Sized,
{
    executor
        .raw_execute(
            &metadata_orphan_sql(owner, true),
            &[DbValue::Text(owner.metadatable_type.clone())],
        )
        .await
}

fn metadata_orphan_sql(owner: &MetadataOwner, delete: bool) -> String {
    let action = if delete {
        "DELETE FROM metadata AS metadata_row"
    } else {
        "SELECT COUNT(*)::BIGINT AS orphan_count FROM metadata AS metadata_row"
    };
    format!(
        "{action} WHERE metadata_row.metadatable_type = $1 AND NOT EXISTS (SELECT 1 FROM \"{}\" AS metadata_owner WHERE metadata_owner.\"{}\" = metadata_row.metadatable_id)",
        owner.table, owner.primary_key
    )
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    let mut characters = value.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    let valid_rest =
        characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if !valid_first || !valid_rest {
        return Err(Error::message(format!(
            "metadata owner {label} `{value}` is not a valid PostgreSQL identifier"
        )));
    }
    Ok(())
}

fn parse_metadatable_uuid(id: &str) -> Result<Uuid> {
    Uuid::parse_str(id).map_err(|error| {
        Error::message(format!(
            "metadata expected UUID metadatable_id `{id}`: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use crate::database::{scope_model_extensions, DbRecord, QueryExecutionOptions, QueryExecutor};

    use super::*;

    #[derive(Default)]
    struct CountingMetadataExecutor {
        query_count: AtomicUsize,
    }

    #[async_trait]
    impl QueryExecutor for CountingMetadataExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            self.query_count.fetch_add(1, Ordering::SeqCst);
            let metadatable_type = match bindings.first() {
                Some(DbValue::Text(value)) => value.clone(),
                _ => panic!("expected metadatable_type binding"),
            };
            let ids = bindings
                .iter()
                .filter_map(|binding| match binding {
                    DbValue::Uuid(value) => Some(*value),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(!ids.is_empty(), "expected metadatable_id UUID bindings");
            let key = bindings.iter().rev().find_map(|binding| match binding {
                DbValue::Text(value) if value != &metadatable_type => Some(value.clone()),
                _ => None,
            });

            Ok(ids
                .into_iter()
                .map(|id| metadata_record(&metadatable_type, id, key.as_deref().unwrap_or("theme")))
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

    fn metadata_record(metadatable_type: &str, metadatable_id: Uuid, key: &str) -> DbRecord {
        let mut record = DbRecord::new();
        record.insert("id", DbValue::Uuid(Uuid::now_v7()));
        record.insert(
            "metadatable_type",
            DbValue::Text(metadatable_type.to_string()),
        );
        record.insert("metadatable_id", DbValue::Uuid(metadatable_id));
        record.insert("key", DbValue::Text(key.to_string()));
        record.insert("value", DbValue::Json(serde_json::json!("dark")));
        record
    }

    #[tokio::test]
    async fn lazy_metadata_cache_batches_known_scope_ids() {
        let executor = CountingMetadataExecutor::default();
        let first_id = Uuid::now_v7().to_string();
        let second_id = Uuid::now_v7().to_string();
        let shape = MetadataCacheShape::Key("theme".to_string());

        scope_model_extensions(async {
            current_extension_scope()
                .unwrap()
                .register_model_ids("test_metadatable", [first_id.clone(), second_id.clone()]);

            let first = cached_metadata_for_id(&executor, "test_metadatable", &first_id, &shape)
                .await
                .unwrap()
                .unwrap();
            let second = cached_metadata_for_id(&executor, "test_metadatable", &second_id, &shape)
                .await
                .unwrap()
                .unwrap();

            assert_eq!(executor.query_count.load(Ordering::SeqCst), 1);
            assert_eq!(first[0].metadatable_id, first_id);
            assert_eq!(second[0].metadatable_id, second_id);
        })
        .await;
    }

    #[test]
    fn metadata_owner_rejects_unsafe_identifiers() {
        let error = MetadataOwner::new("users", "users; DROP TABLE metadata", "id")
            .expect_err("unsafe owner table should be rejected");
        assert!(error.to_string().contains("PostgreSQL identifier"));
        assert!(MetadataOwner::new("users", "account_users", "user_id").is_ok());
    }
}
