use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::database::{DbValue, OrderBy, Query, Sql};
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
        let rows = Query::table(METADATA_TABLE)
            .select(["value"])
            .where_eq("metadatable_type", Self::metadatable_type())
            .where_eq("metadatable_id", parse_metadatable_uuid(&metadatable_id)?)
            .where_eq("key", key.to_string())
            .get(&*app.database()?)
            .await?;
        match rows.first() {
            Some(row) => match row.get("value") {
                Some(DbValue::Json(v)) => Ok(Some(v.clone())),
                _ => Ok(None),
            },
            None => Ok(None),
        }
    }

    async fn forget_meta(&self, app: &AppContext, key: &str) -> Result<bool> {
        let metadatable_id = self.metadatable_id();
        let affected = Query::delete_from(METADATA_TABLE)
            .where_eq("metadatable_type", Self::metadatable_type())
            .where_eq("metadatable_id", parse_metadatable_uuid(&metadatable_id)?)
            .where_eq("key", key.to_string())
            .execute(&*app.database()?)
            .await?;
        Ok(affected > 0)
    }

    async fn has_meta(&self, app: &AppContext, key: &str) -> Result<bool> {
        let metadatable_id = self.metadatable_id();
        let row = Query::table(METADATA_TABLE)
            .select(["id"])
            .where_eq("metadatable_type", Self::metadatable_type())
            .where_eq("metadatable_id", parse_metadatable_uuid(&metadatable_id)?)
            .where_eq("key", key.to_string())
            .first(&*app.database()?)
            .await?;
        Ok(row.is_some())
    }

    async fn all_meta(&self, app: &AppContext) -> Result<Vec<ModelMeta>> {
        let metadatable_id = self.metadatable_id();
        let rows = Query::table(METADATA_TABLE)
            .select(["id", "metadatable_type", "metadatable_id", "key", "value"])
            .where_eq("metadatable_type", Self::metadatable_type())
            .where_eq("metadatable_id", parse_metadatable_uuid(&metadatable_id)?)
            .order_by(OrderBy::asc("key"))
            .get(&*app.database()?)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(ModelMeta {
                    id: row.try_text_or_uuid("id")?,
                    metadatable_type: row.try_text("metadatable_type")?,
                    metadatable_id: row.try_text_or_uuid("metadatable_id")?,
                    key: row.try_text("key")?,
                    value: match row.get("value") {
                        Some(DbValue::Json(v)) => Some(v.clone()),
                        _ => None,
                    },
                })
            })
            .collect()
    }
}

fn parse_metadatable_uuid(id: &str) -> Result<Uuid> {
    Uuid::parse_str(id).map_err(|error| {
        Error::message(format!(
            "metadata expected UUID metadatable_id `{id}`: {error}"
        ))
    })
}
