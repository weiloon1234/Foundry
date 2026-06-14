use serde::{Deserialize, Serialize};

use crate::database::{DbType, DbValue, Expr, OrderBy, Query, Sql};
use crate::foundation::{AppContext, Result};

const SETTINGS_TABLE: &str = "settings";

/// The input type used to render a setting in admin forms.
///
/// Each variant maps to a specific form widget. The `parameters` field
/// on [`Setting`] provides additional constraints and options for the widget.
#[derive(Clone, Debug, Default, PartialEq, Eq, foundry_macros::AppEnum)]
pub enum SettingType {
    /// Single-line text input. Parameters: `max_length`, `placeholder`.
    #[default]
    Text,
    /// Multi-line text input. Parameters: `max_length`, `rows`.
    Textarea,
    /// Numeric input. Parameters: `min`, `max`, `step`.
    Number,
    /// Toggle or checkbox.
    Boolean,
    /// Single-select dropdown. Parameters: `options` array of `{value, label}`.
    Select,
    /// Multi-select input. Parameters: `options` array of `{value, label}`.
    Multiselect,
    /// Email address input.
    Email,
    /// URL input.
    Url,
    /// Color picker. Value stored as hex string (e.g., `"#FF5733"`).
    Color,
    /// Date picker. Value stored as ISO date string.
    Date,
    /// Datetime picker. Value stored as ISO 8601 string.
    Datetime,
    /// File upload. Parameters: `allowed_mimes`, `max_size_kb`.
    File,
    /// Image upload. Parameters: `allowed_mimes`, `max_size_kb`, `max_width`, `max_height`.
    Image,
    /// JSON editor for structured data.
    Json,
    /// Password field (masked input).
    Password,
    /// Code editor. Parameters: `language` (e.g., `"css"`, `"html"`, `"javascript"`).
    Code,
}

const SETTING_TYPE_VARIANTS: &[(&str, SettingType)] = &[
    ("text", SettingType::Text),
    ("textarea", SettingType::Textarea),
    ("number", SettingType::Number),
    ("boolean", SettingType::Boolean),
    ("select", SettingType::Select),
    ("multiselect", SettingType::Multiselect),
    ("email", SettingType::Email),
    ("url", SettingType::Url),
    ("color", SettingType::Color),
    ("date", SettingType::Date),
    ("datetime", SettingType::Datetime),
    ("file", SettingType::File),
    ("image", SettingType::Image),
    ("json", SettingType::Json),
    ("password", SettingType::Password),
    ("code", SettingType::Code),
];

impl SettingType {
    pub fn as_str(&self) -> &'static str {
        SETTING_TYPE_VARIANTS
            .iter()
            .find(|(_, v)| v == self)
            .expect("all variants covered")
            .0
    }

    pub fn parse(s: &str) -> Option<Self> {
        SETTING_TYPE_VARIANTS
            .iter()
            .find(|(k, _)| *k == s)
            .map(|(_, v)| v.clone())
    }

    /// All available setting types.
    pub fn all() -> &'static [(&'static str, SettingType)] {
        SETTING_TYPE_VARIANTS
    }
}

impl std::fmt::Display for SettingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A setting record from the `settings` table.
///
/// Settings are key-value pairs with type metadata for admin form rendering.
/// The `setting_type` and `parameters` fields define how the admin panel
/// should display and validate the input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Setting {
    pub id: String,
    pub key: String,
    pub value: Option<serde_json::Value>,
    pub setting_type: SettingType,
    pub parameters: serde_json::Value,
    pub group_name: String,
    pub label: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_public: bool,
}

/// Data required to create a new setting.
#[derive(Clone, Debug)]
pub struct NewSetting {
    pub key: String,
    pub value: Option<serde_json::Value>,
    pub setting_type: SettingType,
    pub parameters: serde_json::Value,
    pub group_name: String,
    pub label: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_public: bool,
}

impl NewSetting {
    /// Create a new setting definition with required fields.
    /// Defaults: `setting_type = Text`, `group_name = "general"`, `sort_order = 0`,
    /// `is_public = false`, `parameters = {}`.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: None,
            setting_type: SettingType::Text,
            parameters: serde_json::json!({}),
            group_name: "general".to_string(),
            label: label.into(),
            description: None,
            sort_order: 0,
            is_public: false,
        }
    }

    pub fn value(mut self, value: serde_json::Value) -> Self {
        self.value = Some(value);
        self
    }

    pub fn setting_type(mut self, setting_type: SettingType) -> Self {
        self.setting_type = setting_type;
        self
    }

    pub fn parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = parameters;
        self
    }

    pub fn group(mut self, group_name: impl Into<String>) -> Self {
        self.group_name = group_name.into();
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn sort_order(mut self, sort_order: i32) -> Self {
        self.sort_order = sort_order;
        self
    }

    pub fn is_public(mut self, is_public: bool) -> Self {
        self.is_public = is_public;
        self
    }
}

impl Setting {
    /// Get a setting value by key. Returns `None` if the key doesn't exist.
    pub async fn get(app: &AppContext, key: &str) -> Result<Option<serde_json::Value>> {
        let db = app.database()?;
        let rows = Query::table(SETTINGS_TABLE)
            .select(["value"])
            .where_eq("key", key.to_string())
            .get(db.as_ref())
            .await?;
        Ok(rows.first().and_then(|row| match row.get("value") {
            Some(DbValue::Json(v)) => Some(v.clone()),
            _ => None,
        }))
    }

    /// Get a setting as a typed value via serde deserialization.
    /// Returns `None` if the key doesn't exist or deserialization fails.
    pub async fn get_as<T: serde::de::DeserializeOwned>(
        app: &AppContext,
        key: &str,
    ) -> Result<Option<T>> {
        match Self::get(app, key).await? {
            Some(value) => Ok(serde_json::from_value(value).ok()),
            None => Ok(None),
        }
    }

    /// Get a setting value, returning a default if the key doesn't exist.
    pub async fn get_or(
        app: &AppContext,
        key: &str,
        default: serde_json::Value,
    ) -> Result<serde_json::Value> {
        Ok(Self::get(app, key).await?.unwrap_or(default))
    }

    /// Find a setting record by key (returns full Setting with metadata).
    pub async fn find(app: &AppContext, key: &str) -> Result<Option<Setting>> {
        let db = app.database()?;
        let rows = Query::table(SETTINGS_TABLE)
            .where_eq("key", key.to_string())
            .limit(1)
            .get(db.as_ref())
            .await?;
        rows.first().map(row_to_setting).transpose()
    }

    /// Update only the value of an existing setting.
    pub async fn set(app: &AppContext, key: &str, value: serde_json::Value) -> Result<()> {
        let db = app.database()?;
        Query::update_table(SETTINGS_TABLE)
            .value("value", DbValue::Json(value))
            .set_expr("updated_at", Sql::now())
            .where_eq("key", key.to_string())
            .execute(db.as_ref())
            .await?;
        Ok(())
    }

    /// Create a new setting from a [`NewSetting`] definition.
    /// Returns error if the key already exists.
    pub async fn create(app: &AppContext, new: NewSetting) -> Result<()> {
        let db = app.database()?;
        let value_param = match new.value {
            Some(v) => DbValue::Json(v),
            None => DbValue::Null(DbType::Json),
        };
        let desc_param = match new.description {
            Some(d) => DbValue::Text(d),
            None => DbValue::Null(DbType::Text),
        };
        Query::insert_into(SETTINGS_TABLE)
            .values([
                ("key", DbValue::Text(new.key)),
                ("value", value_param),
                (
                    "setting_type",
                    DbValue::Text(new.setting_type.as_str().to_string()),
                ),
                ("parameters", DbValue::Json(new.parameters)),
                ("group_name", DbValue::Text(new.group_name)),
                ("label", DbValue::Text(new.label)),
                ("description", desc_param),
                ("sort_order", DbValue::Int32(new.sort_order)),
                ("is_public", DbValue::Bool(new.is_public)),
            ])
            .execute(db.as_ref())
            .await?;
        Ok(())
    }

    /// Upsert an existing setting's value. Creates with defaults if the key doesn't exist.
    ///
    /// On conflict, only the `value` column is updated — existing metadata is preserved.
    /// On first insert, database defaults apply (`setting_type = "text"`, `group_name = "general"`,
    /// `label = ""`). Use [`Setting::create`] with [`NewSetting`] for full metadata control.
    pub async fn upsert(app: &AppContext, key: &str, value: serde_json::Value) -> Result<()> {
        let db = app.database()?;
        Query::insert_into(SETTINGS_TABLE)
            .values([
                ("key", DbValue::Text(key.to_string())),
                ("value", DbValue::Json(value)),
            ])
            .on_conflict_columns(["key"])
            .do_update()
            .set_excluded("value")
            .set_expr("updated_at", Sql::now())
            .execute(db.as_ref())
            .await?;
        Ok(())
    }

    /// Delete a setting by key. Returns `true` if the key existed.
    pub async fn remove(app: &AppContext, key: &str) -> Result<bool> {
        let db = app.database()?;
        let rows = Query::delete_from(SETTINGS_TABLE)
            .where_eq("key", key.to_string())
            .returning(["id"])
            .get(db.as_ref())
            .await?;
        Ok(!rows.is_empty())
    }

    /// Check if a setting key exists.
    pub async fn exists(app: &AppContext, key: &str) -> Result<bool> {
        let db = app.database()?;
        let rows = Query::table(SETTINGS_TABLE)
            .select(["id"])
            .where_eq("key", key.to_string())
            .limit(1)
            .get(db.as_ref())
            .await?;
        Ok(!rows.is_empty())
    }

    /// List all settings, ordered by group then sort_order.
    pub async fn all(app: &AppContext) -> Result<Vec<Setting>> {
        let db = app.database()?;
        let rows = Query::table(SETTINGS_TABLE)
            .order_by(OrderBy::asc("group_name"))
            .order_by(OrderBy::asc("sort_order"))
            .order_by(OrderBy::asc("key"))
            .get(db.as_ref())
            .await?;
        rows.iter().map(row_to_setting).collect()
    }

    /// List settings in a specific group, ordered by sort_order.
    pub async fn by_group(app: &AppContext, group: &str) -> Result<Vec<Setting>> {
        let db = app.database()?;
        let rows = Query::table(SETTINGS_TABLE)
            .where_eq("group_name", group.to_string())
            .order_by(OrderBy::asc("sort_order"))
            .order_by(OrderBy::asc("key"))
            .get(db.as_ref())
            .await?;
        rows.iter().map(row_to_setting).collect()
    }

    /// List settings whose keys start with a given prefix.
    pub async fn by_prefix(app: &AppContext, prefix: &str) -> Result<Vec<Setting>> {
        let db = app.database()?;
        let pattern = format!("{}%", prefix.replace('%', "\\%").replace('_', "\\_"));
        let rows = Query::table(SETTINGS_TABLE)
            .where_(Expr::column("key").like(pattern))
            .order_by(OrderBy::asc("group_name"))
            .order_by(OrderBy::asc("sort_order"))
            .order_by(OrderBy::asc("key"))
            .get(db.as_ref())
            .await?;
        rows.iter().map(row_to_setting).collect()
    }

    /// List only public settings (safe to expose to frontend/unauthenticated API).
    pub async fn public(app: &AppContext) -> Result<Vec<Setting>> {
        let db = app.database()?;
        let rows = Query::table(SETTINGS_TABLE)
            .where_eq("is_public", true)
            .order_by(OrderBy::asc("group_name"))
            .order_by(OrderBy::asc("sort_order"))
            .order_by(OrderBy::asc("key"))
            .get(db.as_ref())
            .await?;
        rows.iter().map(row_to_setting).collect()
    }

    /// List all distinct group names, ordered alphabetically.
    pub async fn groups(app: &AppContext) -> Result<Vec<String>> {
        let db = app.database()?;
        let rows = Query::table(SETTINGS_TABLE)
            .distinct()
            .select(["group_name"])
            .order_by(OrderBy::asc("group_name"))
            .get(db.as_ref())
            .await?;
        rows.iter().map(|r| r.try_text("group_name")).collect()
    }
}

fn row_to_setting(row: &crate::database::DbRecord) -> Result<Setting> {
    Ok(Setting {
        id: row.try_text_or_uuid("id")?,
        key: row.try_text("key")?,
        value: match row.get("value") {
            Some(DbValue::Json(v)) => Some(v.clone()),
            _ => None,
        },
        setting_type: SettingType::parse(&row.try_text("setting_type")?)
            .unwrap_or(SettingType::Text),
        parameters: match row.get("parameters") {
            Some(DbValue::Json(v)) => v.clone(),
            _ => serde_json::json!({}),
        },
        group_name: row.try_text("group_name")?,
        label: row.try_text("label")?,
        description: row.optional_text("description"),
        sort_order: match row.get("sort_order") {
            Some(DbValue::Int32(v)) => *v,
            _ => 0,
        },
        is_public: match row.get("is_public") {
            Some(DbValue::Bool(v)) => *v,
            _ => false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_type_roundtrip() {
        for (s, st) in SettingType::all() {
            assert_eq!(st.as_str(), *s);
            let parsed = SettingType::parse(s).unwrap();
            assert_eq!(&parsed, st);
        }
    }

    #[test]
    fn setting_type_serde_roundtrip() {
        let st = SettingType::Select;
        let json = serde_json::to_string(&st).unwrap();
        assert_eq!(json, "\"select\"");
        let parsed: SettingType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SettingType::Select);
    }

    #[test]
    fn new_setting_builder() {
        let s = NewSetting::new("app.name", "Application Name")
            .value(serde_json::json!("My App"))
            .setting_type(SettingType::Text)
            .group("general")
            .description("The name of your application")
            .sort_order(1)
            .is_public(true)
            .parameters(serde_json::json!({"max_length": 255}));

        assert_eq!(s.key, "app.name");
        assert_eq!(s.label, "Application Name");
        assert_eq!(s.setting_type, SettingType::Text);
        assert_eq!(s.group_name, "general");
        assert_eq!(s.sort_order, 1);
        assert!(s.is_public);
    }
}
