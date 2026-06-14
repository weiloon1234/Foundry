use serde::{Deserialize, Serialize};

use crate::database::{DbType, DbValue, OrderBy, Query, QueryExecutor, Sql};
use crate::foundation::{AppContext, Error, Result};

const BUILTIN_SEED: &str = include_str!("seed.json");
const COUNTRIES_TABLE: &str = "countries";

/// Country activation status.
#[derive(Clone, Debug, Default, PartialEq, Eq, foundry_macros::AppEnum)]
pub enum CountryStatus {
    Enabled,
    #[default]
    Disabled,
}

impl CountryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "enabled" => Self::Enabled,
            _ => Self::Disabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }
}

impl std::fmt::Display for CountryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A country record from the `countries` table.
///
/// Primary key is `iso2` (2-letter ISO 3166-1 alpha-2 code), not a UUID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Country {
    pub iso2: String,
    pub iso3: String,
    pub iso_numeric: Option<String>,
    pub name: String,
    pub official_name: Option<String>,
    pub capital: Option<String>,
    pub region: Option<String>,
    pub subregion: Option<String>,
    pub currencies: serde_json::Value,
    pub primary_currency_code: Option<String>,
    pub calling_code: Option<String>,
    pub calling_root: Option<String>,
    pub calling_suffixes: serde_json::Value,
    pub tlds: serde_json::Value,
    pub timezones: serde_json::Value,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub independent: Option<bool>,
    pub un_member: Option<bool>,
    pub flag_emoji: Option<String>,
    pub status: CountryStatus,
    pub conversion_rate: Option<f64>,
    pub is_default: bool,
}

impl Country {
    /// Find a country by ISO2 code.
    pub async fn find(app: &AppContext, iso2: &str) -> Result<Option<Country>> {
        let db = app.database()?;
        let rows = Query::table(COUNTRIES_TABLE)
            .where_eq("iso2", iso2.to_ascii_uppercase())
            .limit(1)
            .get(db.as_ref())
            .await?;
        rows.first().map(row_to_country).transpose()
    }

    /// List all countries, ordered by name.
    pub async fn all(app: &AppContext) -> Result<Vec<Country>> {
        let db = app.database()?;
        let rows = Query::table(COUNTRIES_TABLE)
            .order_by(OrderBy::asc("name"))
            .get(db.as_ref())
            .await?;
        rows.iter().map(row_to_country).collect()
    }

    /// List countries filtered by status.
    pub async fn by_status(app: &AppContext, status: CountryStatus) -> Result<Vec<Country>> {
        let db = app.database()?;
        let rows = Query::table(COUNTRIES_TABLE)
            .where_eq("status", status.as_str().to_string())
            .order_by(OrderBy::asc("name"))
            .get(db.as_ref())
            .await?;
        rows.iter().map(row_to_country).collect()
    }

    /// List only enabled countries.
    pub async fn enabled(app: &AppContext) -> Result<Vec<Country>> {
        Self::by_status(app, CountryStatus::Enabled).await
    }

    /// List only disabled countries.
    pub async fn disabled(app: &AppContext) -> Result<Vec<Country>> {
        Self::by_status(app, CountryStatus::Disabled).await
    }

    /// Check if an ISO2 code is valid (exists in the table).
    pub async fn exists(app: &AppContext, iso2: &str) -> Result<bool> {
        let db = app.database()?;
        let rows = Query::table(COUNTRIES_TABLE)
            .select(["iso2"])
            .where_eq("iso2", iso2.to_ascii_uppercase())
            .limit(1)
            .get(db.as_ref())
            .await?;
        Ok(!rows.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Seed data
// ---------------------------------------------------------------------------

/// A country seed record from the built-in JSON data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountrySeed {
    pub iso2: String,
    pub iso3: String,
    #[serde(default)]
    pub iso_numeric: Option<String>,
    pub name: String,
    #[serde(default)]
    pub official_name: Option<String>,
    #[serde(default)]
    pub capital: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub subregion: Option<String>,
    #[serde(default)]
    pub currencies: Vec<CountryCurrency>,
    #[serde(default)]
    pub primary_currency_code: Option<String>,
    #[serde(default)]
    pub calling_code: Option<String>,
    #[serde(default)]
    pub calling_root: Option<String>,
    #[serde(default)]
    pub calling_suffixes: Vec<String>,
    #[serde(default)]
    pub tlds: Vec<String>,
    #[serde(default)]
    pub timezones: Vec<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub independent: Option<bool>,
    #[serde(default)]
    pub un_member: Option<bool>,
    #[serde(default)]
    pub flag_emoji: Option<String>,
    #[serde(default)]
    pub conversion_rate: Option<f64>,
    #[serde(default)]
    pub is_default: Option<bool>,
    #[serde(default, alias = "status")]
    pub assignment_status: Option<String>,
    #[serde(default)]
    pub capitals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryCurrency {
    pub code: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub minor_units: Option<i16>,
}

/// Load the built-in 250 country seed records.
pub fn load_seed() -> Result<Vec<CountrySeed>> {
    serde_json::from_str(BUILTIN_SEED)
        .map_err(|e| Error::message(format!("failed to parse built-in countries seed: {e}")))
}

/// Seed the countries table from built-in data.
///
/// Uses upsert (ON CONFLICT iso2 DO UPDATE) so it's safe to run multiple times.
pub async fn seed_countries(app: &AppContext) -> Result<u64> {
    let db = app.database()?;
    seed_countries_with(db.as_ref()).await
}

/// Seed the countries table using any database executor.
///
/// This lets published seeders reuse the same upsert logic while still running
/// inside the active seeder transaction.
pub async fn seed_countries_with(executor: &dyn QueryExecutor) -> Result<u64> {
    let seeds = load_seed()?;
    let mut count = 0u64;

    for seed in seeds {
        upsert_country_seed(executor, &seed).await?;
        count += 1;
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn opt_text(value: &Option<String>) -> DbValue {
    match value {
        Some(s) if !s.trim().is_empty() => DbValue::Text(s.trim().to_string()),
        _ => DbValue::Null(DbType::Text),
    }
}

fn opt_f64(value: Option<f64>) -> DbValue {
    match value {
        Some(v) => DbValue::Float64(v),
        None => DbValue::Null(DbType::Float64),
    }
}

fn opt_bool(value: Option<bool>) -> DbValue {
    match value {
        Some(v) => DbValue::Bool(v),
        None => DbValue::Null(DbType::Bool),
    }
}

async fn upsert_country_seed(executor: &dyn QueryExecutor, seed: &CountrySeed) -> Result<()> {
    let iso2 = seed.iso2.trim().to_ascii_uppercase();
    let iso3 = seed.iso3.trim().to_ascii_uppercase();
    let currencies = serde_json::to_value(&seed.currencies).unwrap_or_default();
    let calling_suffixes = serde_json::to_value(&seed.calling_suffixes).unwrap_or_default();
    let tlds = serde_json::to_value(&seed.tlds).unwrap_or_default();
    let timezones = serde_json::to_value(&seed.timezones).unwrap_or_default();

    let mut query = Query::insert_into(COUNTRIES_TABLE)
        .values([
            ("iso2", DbValue::Text(iso2)),
            ("iso3", DbValue::Text(iso3)),
            ("iso_numeric", opt_text(&seed.iso_numeric)),
            ("name", DbValue::Text(seed.name.trim().to_string())),
            ("official_name", opt_text(&seed.official_name)),
            ("capital", opt_text(&seed.capital)),
            ("region", opt_text(&seed.region)),
            ("subregion", opt_text(&seed.subregion)),
            ("currencies", DbValue::Json(currencies)),
            (
                "primary_currency_code",
                opt_text(&seed.primary_currency_code),
            ),
            ("calling_code", opt_text(&seed.calling_code)),
            ("calling_root", opt_text(&seed.calling_root)),
            ("calling_suffixes", DbValue::Json(calling_suffixes)),
            ("tlds", DbValue::Json(tlds)),
            ("timezones", DbValue::Json(timezones)),
            ("latitude", opt_f64(seed.latitude)),
            ("longitude", opt_f64(seed.longitude)),
            ("independent", opt_bool(seed.independent)),
            ("un_member", opt_bool(seed.un_member)),
            ("flag_emoji", opt_text(&seed.flag_emoji)),
            ("conversion_rate", opt_f64(seed.conversion_rate)),
            (
                "is_default",
                DbValue::Bool(seed.is_default.unwrap_or(false)),
            ),
            (
                "status",
                DbValue::Text(CountryStatus::Disabled.as_str().to_string()),
            ),
        ])
        .on_conflict_columns(["iso2"])
        .do_update()
        .set_excluded("iso3")
        .set_excluded("iso_numeric")
        .set_excluded("name")
        .set_excluded("official_name")
        .set_excluded("capital")
        .set_excluded("region")
        .set_excluded("subregion")
        .set_excluded("currencies")
        .set_excluded("primary_currency_code")
        .set_excluded("calling_code")
        .set_excluded("calling_root")
        .set_excluded("calling_suffixes")
        .set_excluded("tlds")
        .set_excluded("timezones")
        .set_excluded("latitude")
        .set_excluded("longitude")
        .set_excluded("independent")
        .set_excluded("un_member")
        .set_excluded("flag_emoji")
        .set_expr("updated_at", Sql::now());

    if seed.conversion_rate.is_some() {
        query = query.set_excluded("conversion_rate");
    }

    if seed.is_default.is_some() {
        query = query.set_excluded("is_default");
    }

    query.execute(executor).await?;
    Ok(())
}

fn row_to_country(row: &crate::database::DbRecord) -> Result<Country> {
    Ok(Country {
        iso2: row.try_text("iso2")?,
        iso3: row.try_text("iso3")?,
        iso_numeric: row.optional_text("iso_numeric"),
        name: row.try_text("name")?,
        official_name: row.optional_text("official_name"),
        capital: row.optional_text("capital"),
        region: row.optional_text("region"),
        subregion: row.optional_text("subregion"),
        currencies: match row.get("currencies") {
            Some(DbValue::Json(v)) => v.clone(),
            _ => serde_json::json!([]),
        },
        primary_currency_code: row.optional_text("primary_currency_code"),
        calling_code: row.optional_text("calling_code"),
        calling_root: row.optional_text("calling_root"),
        calling_suffixes: match row.get("calling_suffixes") {
            Some(DbValue::Json(v)) => v.clone(),
            _ => serde_json::json!([]),
        },
        tlds: match row.get("tlds") {
            Some(DbValue::Json(v)) => v.clone(),
            _ => serde_json::json!([]),
        },
        timezones: match row.get("timezones") {
            Some(DbValue::Json(v)) => v.clone(),
            _ => serde_json::json!([]),
        },
        latitude: match row.get("latitude") {
            Some(DbValue::Float64(v)) => Some(*v),
            _ => None,
        },
        longitude: match row.get("longitude") {
            Some(DbValue::Float64(v)) => Some(*v),
            _ => None,
        },
        independent: match row.get("independent") {
            Some(DbValue::Bool(v)) => Some(*v),
            _ => None,
        },
        un_member: match row.get("un_member") {
            Some(DbValue::Bool(v)) => Some(*v),
            _ => None,
        },
        flag_emoji: row.optional_text("flag_emoji"),
        status: CountryStatus::parse(&row.try_text("status")?),
        conversion_rate: match row.get("conversion_rate") {
            Some(DbValue::Float64(v)) => Some(*v),
            _ => None,
        },
        is_default: match row.get("is_default") {
            Some(DbValue::Bool(v)) => *v,
            _ => false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_seed_parses_all_250_countries() {
        let countries = load_seed().unwrap();
        assert_eq!(countries.len(), 250);
    }

    #[test]
    fn seed_data_has_expected_countries() {
        let countries = load_seed().unwrap();
        let my = countries.iter().find(|c| c.iso2 == "MY").unwrap();
        assert_eq!(my.name, "Malaysia");
        assert_eq!(my.iso3, "MYS");
        assert!(my.flag_emoji.is_some());
    }

    #[test]
    fn seed_data_has_currencies() {
        let countries = load_seed().unwrap();
        let us = countries.iter().find(|c| c.iso2 == "US").unwrap();
        assert!(!us.currencies.is_empty());
        assert_eq!(us.currencies[0].code, "USD");
    }

    #[test]
    fn seed_data_defaults_optional_country_flags() {
        let countries = load_seed().unwrap();
        let my = countries.iter().find(|c| c.iso2 == "MY").unwrap();
        assert_eq!(my.conversion_rate, None);
        assert_eq!(my.is_default, None);
    }
}
