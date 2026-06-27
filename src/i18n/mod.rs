pub mod extractor;

// Re-export the primary extractor type at module root for convenience.
pub use extractor::I18n;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::config::I18nConfig;
use crate::foundation::{Error, Result};

const MAX_ACCEPT_LANGUAGE_CANDIDATES: usize = 32;
const MAX_LANGUAGE_TAG_BYTES: usize = 64;

/// Translate a key using the [`I18n`] extractor with named parameters.
///
/// ```
/// use foundry::prelude::*;
/// use foundry::t;
///
/// async fn handler(i18n: I18n) -> String {
///     // No parameters
///     t!(i18n, "Something went wrong")
/// }
///
/// async fn greeting(i18n: I18n) -> String {
///     // Named parameters — order doesn't matter
///     t!(i18n, "Hello {{name2}} and {{name}}", name2 = "Alice", name = "Bob")
/// }
/// ```
#[macro_export]
macro_rules! t {
    ($i18n:expr, $key:expr) => {
        $i18n.t($key)
    };
    ($i18n:expr, $key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $i18n.t_with($key, &[$((stringify!($name), $value)),+])
    };
}

type Catalog = HashMap<String, String>;

/// Manages translation catalogs loaded at startup.
///
/// Scans `{resource_path}/{locale}/*.json`, merges all files per locale into
/// a single catalog, and provides O(1) translation lookups with a three-tier
/// fallback chain: requested locale → fallback locale → key itself.
///
/// Thread-safe by design — loaded once, never mutated.
pub struct I18nManager {
    default_locale: String,
    fallback_locale: String,
    catalogs: HashMap<String, Catalog>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I18nLocaleDescriptor {
    pub locale: String,
    pub default: bool,
    pub fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I18nManifestDescriptor {
    pub default_locale: String,
    pub fallback_locale: String,
    pub locales: Vec<I18nLocaleDescriptor>,
}

impl Default for I18nManifestDescriptor {
    fn default() -> Self {
        Self::from_config(&I18nConfig::default())
    }
}

impl I18nManifestDescriptor {
    pub fn from_config(config: &I18nConfig) -> Self {
        Self {
            default_locale: config.default_locale.clone(),
            fallback_locale: config.fallback_locale.clone(),
            locales: Vec::new(),
        }
    }
}

impl I18nManager {
    /// Load all translation catalogs from the configured resource path.
    ///
    /// Scans `{resource_path}/*/` for locale directories, reads all `*.json`
    /// files in each, and merges them into per-locale catalogs. Warns on
    /// duplicate keys (last file wins).
    pub fn load(config: &I18nConfig) -> Result<Self> {
        let resource_path = Path::new(&config.resource_path);

        if !resource_path.exists() {
            tracing::info!(
                "foundry: i18n resource path not found, skipping: {}",
                config.resource_path
            );
            return Ok(Self {
                default_locale: config.default_locale.clone(),
                fallback_locale: config.fallback_locale.clone(),
                catalogs: HashMap::new(),
            });
        }

        let mut catalogs: HashMap<String, Catalog> = HashMap::new();

        let locale_dirs = fs::read_dir(resource_path)
            .map_err(Error::other)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir());

        for locale_dir in locale_dirs {
            let locale_name = match locale_dir.file_name().to_str() {
                Some(name) => name.to_string(),
                None => continue,
            };

            let mut catalog: Catalog = HashMap::new();

            let json_files = fs::read_dir(locale_dir.path())
                .map_err(Error::other)?
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("json"))
                        .unwrap_or(false)
                });

            for json_file in json_files {
                let content = fs::read_to_string(json_file.path()).map_err(Error::other)?;
                let value: Value = serde_json::from_str(&content).map_err(Error::other)?;

                if let Value::Object(map) = value {
                    merge_json_into_catalog(&mut catalog, &map, &locale_name);
                }
            }

            if !catalog.is_empty() {
                tracing::debug!(
                    "foundry: i18n loaded {} keys for locale '{}'",
                    catalog.len(),
                    locale_name
                );
                catalogs.insert(locale_name, catalog);
            }
        }

        let loaded_locales: Vec<&str> = catalogs.keys().map(|s| s.as_str()).collect();
        tracing::info!("foundry: i18n loaded locales: {:?}", loaded_locales);

        Ok(Self {
            default_locale: config.default_locale.clone(),
            fallback_locale: config.fallback_locale.clone(),
            catalogs,
        })
    }

    /// Translate a key in the given locale, interpolating values.
    ///
    /// Fallback chain:
    /// 1. `catalogs[locale][key]`
    /// 2. `catalogs[fallback_locale][key]`
    /// 3. `key` itself (the English string is the key)
    pub fn translate(&self, locale: &str, key: &str, values: &[(&str, &str)]) -> String {
        let template = self
            .catalogs
            .get(locale)
            .and_then(|cat| cat.get(key))
            .or_else(|| {
                self.catalogs
                    .get(&self.fallback_locale)
                    .and_then(|cat| cat.get(key))
            })
            .map(|s| s.as_str())
            .unwrap_or(key);

        if values.is_empty() {
            template.to_string()
        } else {
            interpolate(template, values)
        }
    }

    /// Resolve the best matching locale from an `Accept-Language` header value.
    ///
    /// Parses the header, finds the first locale that matches a loaded catalog,
    /// or falls back to the default locale.
    pub fn resolve_locale(&self, accept_language: &str) -> String {
        for tag in parse_accept_language(accept_language) {
            if tag == "*" {
                if self.has_locale(&self.default_locale) {
                    return self.default_locale.clone();
                }
                continue;
            }
            if let Some(locale) = self.match_locale(&tag) {
                return locale;
            }
        }
        self.default_locale.clone()
    }

    /// The configured default locale.
    pub fn default_locale(&self) -> &str {
        &self.default_locale
    }

    /// The configured fallback locale.
    pub fn fallback_locale(&self) -> &str {
        &self.fallback_locale
    }

    /// Whether a catalog exists for the given locale.
    pub fn has_locale(&self, locale: &str) -> bool {
        self.catalogs.contains_key(locale)
    }

    fn match_locale(&self, tag: &str) -> Option<String> {
        if self.has_locale(tag) {
            return Some(tag.to_string());
        }

        let mut candidate = tag;
        while let Some((base, _rest)) = candidate.rsplit_once('-') {
            if self.has_locale(base) {
                return Some(base.to_string());
            }
            candidate = base;
        }

        None
    }

    /// List of all loaded locale names.
    pub fn locale_list(&self) -> Vec<&str> {
        let mut locales = self.catalogs.keys().map(|s| s.as_str()).collect::<Vec<_>>();
        locales.sort();
        locales
    }

    pub fn descriptor(&self) -> I18nManifestDescriptor {
        I18nManifestDescriptor {
            default_locale: self.default_locale.clone(),
            fallback_locale: self.fallback_locale.clone(),
            locales: self
                .locale_list()
                .into_iter()
                .map(|locale| I18nLocaleDescriptor {
                    locale: locale.to_string(),
                    default: locale == self.default_locale,
                    fallback: locale == self.fallback_locale,
                })
                .collect(),
        }
    }
}

/// Per-request locale stored in request extensions.
///
/// Can be set by custom middleware (e.g., from a cookie or user preference)
/// and is read by the `I18n` extractor.
#[derive(Clone, Debug)]
pub struct Locale(pub String);

/// Merge a JSON object (potentially nested) into a flat catalog.
///
/// Nested keys are flattened by joining with `.`:
/// `{"errors": {"not_found": "Not found"}}` → `"errors.not_found" → "Not found"`
///
/// Top-level string values are merged directly. Non-string leaf values are skipped.
fn merge_json_into_catalog(
    catalog: &mut Catalog,
    map: &serde_json::Map<String, Value>,
    locale: &str,
) {
    for (key, value) in map {
        match value {
            Value::String(s) => {
                if let Some(existing) = catalog.get(key) {
                    tracing::warn!(
                        "foundry: i18n duplicate key '{}' in locale '{}', overwriting '{}' with '{}'",
                        key,
                        locale,
                        existing,
                        s
                    );
                }
                catalog.insert(key.clone(), s.clone());
            }
            Value::Object(nested) => {
                merge_json_nested(catalog, nested, key, locale);
            }
            _ => {}
        }
    }
}

fn merge_json_nested(
    catalog: &mut Catalog,
    map: &serde_json::Map<String, Value>,
    prefix: &str,
    locale: &str,
) {
    for (key, value) in map {
        let full_key = format!("{}.{}", prefix, key);
        match value {
            Value::String(s) => {
                if let Some(existing) = catalog.get(&full_key) {
                    tracing::warn!(
                        "foundry: i18n duplicate key '{}' in locale '{}', overwriting '{}' with '{}'",
                        full_key,
                        locale,
                        existing,
                        s
                    );
                }
                catalog.insert(full_key, s.clone());
            }
            Value::Object(deeper) => {
                merge_json_nested(catalog, deeper, &full_key, locale);
            }
            _ => {}
        }
    }
}

/// Replace `{{var}}` placeholders with values.
fn interpolate(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in values {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

#[derive(Debug, PartialEq, Eq)]
struct LanguagePreference {
    order: usize,
    quality: u16,
    tag: String,
}

/// Parse an `Accept-Language` header value into quality-sorted locale tags.
fn parse_accept_language(header: &str) -> Vec<String> {
    let mut preferences = header
        .split(',')
        .take(MAX_ACCEPT_LANGUAGE_CANDIDATES)
        .enumerate()
        .filter_map(|(order, value)| {
            let mut parts = value.split(';');
            let tag = normalize_language_tag(parts.next()?.trim())?;
            let mut quality = 1000;
            for parameter in parts {
                let parameter = parameter.trim();
                let Some((name, value)) = parameter.split_once('=') else {
                    continue;
                };
                if name.trim().eq_ignore_ascii_case("q") {
                    quality = parse_quality(value.trim())?;
                    break;
                }
            }
            (quality > 0).then_some(LanguagePreference {
                order,
                quality,
                tag,
            })
        })
        .collect::<Vec<_>>();

    preferences.sort_by(|left, right| {
        right
            .quality
            .cmp(&left.quality)
            .then_with(|| left.order.cmp(&right.order))
    });
    preferences
        .into_iter()
        .map(|preference| preference.tag)
        .collect()
}

fn normalize_language_tag(tag: &str) -> Option<String> {
    let tag = tag.trim();
    if tag == "*" {
        return Some(tag.to_string());
    }
    if tag.is_empty() || tag.len() > MAX_LANGUAGE_TAG_BYTES {
        return None;
    }
    if tag
        .split('-')
        .any(|segment| segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return None;
    }
    Some(tag.to_string())
}

fn parse_quality(value: &str) -> Option<u16> {
    let value = value.trim();
    if value == "1" || value == "1.0" || value == "1.00" || value == "1.000" {
        return Some(1000);
    }
    if value == "0" {
        return Some(0);
    }
    let fraction = value.strip_prefix("0.")?;
    if fraction.is_empty() || fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut quality = fraction.parse::<u16>().ok()?;
    for _ in fraction.len()..3 {
        quality *= 10;
    }
    Some(quality)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::config::I18nConfig;

    fn make_config(dir: &tempfile::TempDir) -> I18nConfig {
        I18nConfig {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            resource_path: dir.path().to_str().unwrap().to_string(),
        }
    }

    #[test]
    fn loads_catalogs_from_filesystem() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("en")).unwrap();
        fs::write(dir.path().join("en/common.json"), r#"{ "Hello": "Hello" }"#).unwrap();
        fs::create_dir(dir.path().join("ms")).unwrap();
        fs::write(dir.path().join("ms/common.json"), r#"{ "Hello": "Helo" }"#).unwrap();

        let manager = I18nManager::load(&make_config(&dir)).unwrap();

        assert_eq!(manager.translate("en", "Hello", &[]), "Hello");
        assert_eq!(manager.translate("ms", "Hello", &[]), "Helo");
    }

    #[test]
    fn descriptor_exposes_sorted_locale_metadata() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("ms")).unwrap();
        fs::write(dir.path().join("ms/common.json"), r#"{ "Hello": "Helo" }"#).unwrap();
        fs::create_dir(dir.path().join("en")).unwrap();
        fs::write(dir.path().join("en/common.json"), r#"{ "Hello": "Hello" }"#).unwrap();

        let manager = I18nManager::load(&make_config(&dir)).unwrap();

        assert_eq!(manager.locale_list(), vec!["en", "ms"]);
        assert_eq!(
            manager.descriptor(),
            I18nManifestDescriptor {
                default_locale: "en".to_string(),
                fallback_locale: "en".to_string(),
                locales: vec![
                    I18nLocaleDescriptor {
                        locale: "en".to_string(),
                        default: true,
                        fallback: true,
                    },
                    I18nLocaleDescriptor {
                        locale: "ms".to_string(),
                        default: false,
                        fallback: false,
                    },
                ],
            }
        );
    }

    #[test]
    fn merges_multiple_files_per_locale() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("en")).unwrap();
        fs::write(dir.path().join("en/common.json"), r#"{ "Hello": "Hello" }"#).unwrap();
        fs::write(
            dir.path().join("en/validation.json"),
            r#"{ "Required": "This field is required" }"#,
        )
        .unwrap();

        let manager = I18nManager::load(&make_config(&dir)).unwrap();

        assert_eq!(manager.translate("en", "Hello", &[]), "Hello");
        assert_eq!(
            manager.translate("en", "Required", &[]),
            "This field is required"
        );
    }

    #[test]
    fn falls_back_to_fallback_locale() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("en")).unwrap();
        fs::write(dir.path().join("en/common.json"), r#"{ "Hello": "Hello" }"#).unwrap();
        fs::create_dir(dir.path().join("ms")).unwrap();
        fs::write(dir.path().join("ms/common.json"), "{}").unwrap();

        let config = I18nConfig {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            resource_path: dir.path().to_str().unwrap().to_string(),
        };
        let manager = I18nManager::load(&config).unwrap();

        // "ms" locale doesn't have "Hello", falls back to "en"
        assert_eq!(manager.translate("ms", "Hello", &[]), "Hello");
    }

    #[test]
    fn returns_key_when_not_found_anywhere() {
        let manager = I18nManager {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            catalogs: HashMap::new(),
        };

        assert_eq!(manager.translate("en", "Missing key", &[]), "Missing key");
    }

    #[test]
    fn interpolates_values() {
        let manager = I18nManager {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            catalogs: {
                let mut m = HashMap::new();
                m.insert(
                    "en".to_string(),
                    HashMap::from([
                        ("Hello, {{name}}".to_string(), "Hello, {{name}}".to_string()),
                        ("{{count}} items".to_string(), "{{count}} items".to_string()),
                    ]),
                );
                m
            },
        };

        assert_eq!(
            manager.translate("en", "Hello, {{name}}", &[("name", "WeiLoon")]),
            "Hello, WeiLoon"
        );
        assert_eq!(
            manager.translate("en", "{{count}} items", &[("count", "5")]),
            "5 items"
        );
    }

    #[test]
    fn interpolates_translated_template() {
        let manager = I18nManager {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            catalogs: {
                let mut m = HashMap::new();
                m.insert(
                    "en".to_string(),
                    HashMap::from([("Hello, {{name}}".to_string(), "Hello, {{name}}".to_string())]),
                );
                m.insert(
                    "ms".to_string(),
                    HashMap::from([("Hello, {{name}}".to_string(), "Helo, {{name}}".to_string())]),
                );
                m
            },
        };

        assert_eq!(
            manager.translate("ms", "Hello, {{name}}", &[("name", "WeiLoon")]),
            "Helo, WeiLoon"
        );
    }

    #[test]
    fn resolves_locale_from_accept_language() {
        let manager = I18nManager {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            catalogs: {
                let mut m = HashMap::new();
                m.insert("en".to_string(), HashMap::new());
                m.insert("ms".to_string(), HashMap::new());
                m.insert("zh-CN".to_string(), HashMap::new());
                m
            },
        };

        assert_eq!(manager.resolve_locale("ms"), "ms");
        assert_eq!(manager.resolve_locale("ms,en-US;q=0.9"), "ms");
        assert_eq!(manager.resolve_locale("fr"), "en"); // not loaded, falls back
        assert_eq!(manager.resolve_locale("zh-CN,en;q=0.9"), "zh-CN");
    }

    #[test]
    fn resolves_locale_by_accept_language_quality_and_base_tag() {
        let manager = I18nManager {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            catalogs: {
                let mut m = HashMap::new();
                m.insert("en".to_string(), HashMap::new());
                m.insert("ms".to_string(), HashMap::new());
                m.insert("zh".to_string(), HashMap::new());
                m
            },
        };

        assert_eq!(manager.resolve_locale("ms;q=0.4,en;q=0.8"), "en");
        assert_eq!(manager.resolve_locale("zh-CN;q=0.9,en;q=0.8"), "zh");
        assert_eq!(manager.resolve_locale("fr;q=0.9,*;q=0.5"), "en");
        assert_eq!(manager.resolve_locale("ms;q=0,en;q=0.8"), "en");
    }

    #[test]
    fn flattens_nested_json() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("en")).unwrap();
        fs::write(
            dir.path().join("en/common.json"),
            r#"{
                "Something went wrong": "Something went wrong",
                "errors": {
                    "not_found": "Not found",
                    "validation": {
                        "required": "This field is required"
                    }
                }
            }"#,
        )
        .unwrap();

        let manager = I18nManager::load(&make_config(&dir)).unwrap();

        assert_eq!(
            manager.translate("en", "Something went wrong", &[]),
            "Something went wrong"
        );
        assert_eq!(
            manager.translate("en", "errors.not_found", &[]),
            "Not found"
        );
        assert_eq!(
            manager.translate("en", "errors.validation.required", &[]),
            "This field is required"
        );
    }

    #[test]
    fn handles_missing_resource_path_gracefully() {
        let config = I18nConfig {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            resource_path: "/nonexistent/path".to_string(),
        };

        let manager = I18nManager::load(&config).unwrap();
        assert_eq!(manager.translate("en", "Hello", &[]), "Hello");
    }

    #[test]
    fn parse_accept_language_basic() {
        let tags = parse_accept_language("en-US,en;q=0.9,ms;q=0.8");
        assert_eq!(tags, vec!["en-US", "en", "ms"]);
    }

    #[test]
    fn parse_accept_language_sorts_by_quality_and_preserves_ties() {
        let tags = parse_accept_language("ms;q=0.4,zh-CN;q=0.9,en;q=0.9,fr;q=0.1");
        assert_eq!(tags, vec!["zh-CN", "en", "ms", "fr"]);
    }

    #[test]
    fn parse_accept_language_ignores_invalid_or_zero_quality_entries() {
        let tags = parse_accept_language("ms;q=0,bad tag,en;q=wat,zh;q=0.5,*;q=0.1");
        assert_eq!(tags, vec!["zh", "*"]);
    }

    #[test]
    fn parse_accept_language_single() {
        let tags = parse_accept_language("ms");
        assert_eq!(tags, vec!["ms"]);
    }

    #[test]
    fn parse_accept_language_empty() {
        let tags = parse_accept_language("");
        assert!(tags.is_empty());
    }

    #[test]
    fn t_macro_no_params() {
        let manager = I18nManager {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            catalogs: {
                let mut m = HashMap::new();
                m.insert(
                    "en".to_string(),
                    HashMap::from([("Hello".to_string(), "Hello there".to_string())]),
                );
                m
            },
        };
        let i18n = crate::i18n::I18n::from_parts_for_test(
            "en".to_string(),
            Some(std::sync::Arc::new(manager)),
        );

        assert_eq!(t!(i18n, "Hello"), "Hello there");
    }

    #[test]
    fn t_macro_with_named_params() {
        let manager = I18nManager {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            catalogs: {
                let mut m = HashMap::new();
                m.insert(
                    "en".to_string(),
                    HashMap::from([(
                        "Hello {{name2}} and {{name}}".to_string(),
                        "Hello {{name2}} and {{name}}".to_string(),
                    )]),
                );
                m
            },
        };
        let i18n = crate::i18n::I18n::from_parts_for_test(
            "en".to_string(),
            Some(std::sync::Arc::new(manager)),
        );

        // Order doesn't matter — named params
        assert_eq!(
            t!(
                i18n,
                "Hello {{name2}} and {{name}}",
                name2 = "Alice",
                name = "Bob"
            ),
            "Hello Alice and Bob"
        );
    }

    #[test]
    fn t_macro_noop_when_no_manager() {
        let i18n = crate::i18n::I18n::from_parts_for_test("en".to_string(), None);

        assert_eq!(t!(i18n, "Missing key"), "Missing key");
        assert_eq!(t!(i18n, "Hello {{name}}", name = "World"), "Hello {{name}}");
    }
}
