pub mod spec;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

/// Trait implemented by types that can generate an OpenAPI JSON Schema.
/// Derive with `#[derive(ApiSchema)]`.
pub trait ApiSchema {
    fn schema() -> Value;
    fn schema_name() -> &'static str;
}

#[doc(hidden)]
pub fn insert_json_schema_property(
    properties: &mut serde_json::Map<String, Value>,
    schema_name: &str,
    property_name: impl Into<String>,
    property_schema: Value,
) {
    let property_name = property_name.into();
    if properties
        .insert(property_name.clone(), property_schema)
        .is_some()
    {
        panic!(
            "OpenAPI schema `{schema_name}` contains duplicate property `{property_name}` after applying serde field names and flattening; use unique serde rename values or split the DTO"
        );
    }
}

#[doc(hidden)]
pub fn escape_json_schema_pattern_literal(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        if matches!(
            ch,
            '\\' | '^' | '$' | '.' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{' | '}'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[doc(hidden)]
pub fn insert_json_schema_pattern(
    obj: &mut serde_json::Map<String, Value>,
    pattern: impl Into<String>,
) {
    let pattern = Value::String(pattern.into());
    if !obj.contains_key("pattern") && !obj.contains_key("allOf") {
        obj.insert("pattern".into(), pattern);
        return;
    }

    if let Some(existing_pattern) = obj.remove("pattern") {
        let mut schema = serde_json::Map::new();
        schema.insert("pattern".into(), existing_pattern);
        push_all_of_schema(obj, Value::Object(schema));
    }

    let mut schema = serde_json::Map::new();
    schema.insert("pattern".into(), pattern);
    push_all_of_schema(obj, Value::Object(schema));
}

#[doc(hidden)]
pub fn insert_json_schema_any_pattern(
    obj: &mut serde_json::Map<String, Value>,
    patterns: impl IntoIterator<Item = impl Into<String>>,
) {
    let patterns = patterns
        .into_iter()
        .map(Into::into)
        .collect::<Vec<String>>();
    match patterns.as_slice() {
        [] => {}
        [pattern] => insert_json_schema_pattern(obj, pattern.clone()),
        _ => {
            if let Some(existing_pattern) = obj.remove("pattern") {
                let mut schema = serde_json::Map::new();
                schema.insert("pattern".into(), existing_pattern);
                push_all_of_schema(obj, Value::Object(schema));
            }

            let schemas = patterns
                .into_iter()
                .map(|pattern| {
                    let mut schema = serde_json::Map::new();
                    schema.insert("pattern".into(), Value::String(pattern));
                    Value::Object(schema)
                })
                .collect::<Vec<_>>();

            let mut any_of = serde_json::Map::new();
            any_of.insert("anyOf".into(), Value::Array(schemas));
            push_all_of_schema(obj, Value::Object(any_of));
        }
    }
}

#[doc(hidden)]
pub fn insert_json_schema_not_any_pattern(
    obj: &mut serde_json::Map<String, Value>,
    patterns: impl IntoIterator<Item = impl Into<String>>,
) {
    let patterns = patterns
        .into_iter()
        .map(Into::into)
        .collect::<Vec<String>>();
    let not_schema = match patterns.as_slice() {
        [] => return,
        [pattern] => {
            let mut schema = serde_json::Map::new();
            schema.insert("pattern".into(), Value::String(pattern.clone()));
            Value::Object(schema)
        }
        _ => {
            let schemas = patterns
                .into_iter()
                .map(|pattern| {
                    let mut schema = serde_json::Map::new();
                    schema.insert("pattern".into(), Value::String(pattern));
                    Value::Object(schema)
                })
                .collect::<Vec<_>>();
            let mut any_of = serde_json::Map::new();
            any_of.insert("anyOf".into(), Value::Array(schemas));
            Value::Object(any_of)
        }
    };

    insert_json_schema_not(obj, not_schema);
}

#[doc(hidden)]
pub fn insert_json_schema_array_contains_all(
    obj: &mut serde_json::Map<String, Value>,
    values: impl IntoIterator<Item = impl Into<String>>,
) {
    let mut values = values.into_iter().map(Into::into).collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }

    if values.len() == 1 && !obj.contains_key("contains") && !obj.contains_key("allOf") {
        let value = values.pop().expect("single value exists");
        obj.insert("contains".into(), serde_json::json!({ "const": value }));
        return;
    }

    if let Some(existing_contains) = obj.remove("contains") {
        let mut schema = serde_json::Map::new();
        schema.insert("contains".into(), existing_contains);
        push_all_of_schema(obj, Value::Object(schema));
    }

    for value in values {
        let mut schema = serde_json::Map::new();
        schema.insert("contains".into(), serde_json::json!({ "const": value }));
        push_all_of_schema(obj, Value::Object(schema));
    }
}

#[doc(hidden)]
pub fn insert_json_schema_array_not_contains_any(
    obj: &mut serde_json::Map<String, Value>,
    values: impl IntoIterator<Item = impl Into<String>>,
) {
    let values = values.into_iter().map(Into::into).collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }

    insert_json_schema_not(
        obj,
        serde_json::json!({
            "contains": {
                "enum": values,
            },
        }),
    );
}

#[doc(hidden)]
pub fn insert_json_schema_required_properties(
    obj: &mut serde_json::Map<String, Value>,
    keys: impl IntoIterator<Item = impl Into<String>>,
) {
    if obj.get("type").and_then(Value::as_str) != Some("object") {
        return;
    }

    let required = obj
        .entry("required".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(required) = required.as_array_mut() else {
        return;
    };

    for key in keys {
        let key = key.into();
        if !required
            .iter()
            .any(|value| value.as_str() == Some(key.as_str()))
        {
            required.push(Value::String(key));
        }
    }
}

#[doc(hidden)]
pub fn json_schema_enum_values_for_schema(
    obj: &serde_json::Map<String, Value>,
    values: impl IntoIterator<Item = impl ToString>,
) -> Value {
    let values = values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    match obj.get("type").and_then(Value::as_str) {
        Some("integer") => json_schema_integer_enum_values(&values)
            .map(Value::Array)
            .unwrap_or_else(|| serde_json::json!(values)),
        Some("number") => json_schema_number_enum_values(&values)
            .map(Value::Array)
            .unwrap_or_else(|| serde_json::json!(values)),
        _ => serde_json::json!(values),
    }
}

fn json_schema_integer_enum_values(values: &[String]) -> Option<Vec<Value>> {
    values
        .iter()
        .map(|value| {
            value
                .parse::<i64>()
                .map(|value| serde_json::json!(value))
                .or_else(|_| value.parse::<u64>().map(|value| serde_json::json!(value)))
                .ok()
        })
        .collect()
}

fn json_schema_number_enum_values(values: &[String]) -> Option<Vec<Value>> {
    values
        .iter()
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .and_then(|value| serde_json::Number::from_f64(value).map(Value::Number))
        })
        .collect()
}

#[doc(hidden)]
pub fn insert_foundry_server_only_validation(
    obj: &mut serde_json::Map<String, Value>,
    code: impl Into<String>,
) {
    let code = Value::String(code.into());
    let entry = obj
        .entry("x-foundry-server-only-validation")
        .or_insert_with(|| Value::Array(Vec::new()));

    if let Value::Array(codes) = entry {
        if !codes.iter().any(|existing| existing == &code) {
            codes.push(code);
        }
    } else {
        *entry = Value::Array(vec![code]);
    }
}

#[doc(hidden)]
pub fn insert_foundry_validation_rule(obj: &mut serde_json::Map<String, Value>, rule: Value) {
    let entry = obj
        .entry("x-foundry-validation")
        .or_insert_with(|| Value::Array(Vec::new()));

    if let Value::Array(rules) = entry {
        if !rules.iter().any(|existing| existing == &rule) {
            rules.push(rule);
        }
    } else {
        *entry = Value::Array(vec![rule]);
    }
}

fn insert_json_schema_not(obj: &mut serde_json::Map<String, Value>, schema: Value) {
    if let Some(existing_not) = obj.remove("not") {
        let mut existing = serde_json::Map::new();
        existing.insert("not".into(), existing_not);
        push_all_of_schema(obj, Value::Object(existing));
    }

    if !obj.contains_key("allOf") {
        obj.insert("not".into(), schema);
        return;
    }

    let mut not = serde_json::Map::new();
    not.insert("not".into(), schema);
    push_all_of_schema(obj, Value::Object(not));
}

fn push_all_of_schema(obj: &mut serde_json::Map<String, Value>, schema: Value) {
    if let Some(all_of) = obj.get_mut("allOf").and_then(Value::as_array_mut) {
        all_of.push(schema);
        return;
    }

    obj.insert("allOf".into(), Value::Array(vec![schema]));
}

// Built-in impls for common types

impl ApiSchema for String {
    fn schema() -> Value {
        serde_json::json!({"type": "string"})
    }
    fn schema_name() -> &'static str {
        "String"
    }
}

macro_rules! integer_schema {
    ($ty:ty, $schema_name:literal, $format:literal) => {
        impl ApiSchema for $ty {
            fn schema() -> Value {
                serde_json::json!({"type": "integer", "format": $format})
            }

            fn schema_name() -> &'static str {
                $schema_name
            }
        }
    };
    ($ty:ty, $schema_name:literal) => {
        impl ApiSchema for $ty {
            fn schema() -> Value {
                serde_json::json!({"type": "integer"})
            }

            fn schema_name() -> &'static str {
                $schema_name
            }
        }
    };
}

integer_schema!(i8, "i8", "int32");
integer_schema!(i16, "i16", "int32");
integer_schema!(i32, "i32", "int32");
integer_schema!(i64, "i64", "int64");
integer_schema!(i128, "i128");
integer_schema!(isize, "isize", "int64");
integer_schema!(u8, "u8", "int32");
integer_schema!(u16, "u16", "int32");
integer_schema!(u32, "u32");
integer_schema!(u64, "u64");
integer_schema!(u128, "u128");
integer_schema!(usize, "usize");

impl ApiSchema for f32 {
    fn schema() -> Value {
        serde_json::json!({"type": "number", "format": "float"})
    }
    fn schema_name() -> &'static str {
        "f32"
    }
}

impl ApiSchema for f64 {
    fn schema() -> Value {
        serde_json::json!({"type": "number", "format": "double"})
    }
    fn schema_name() -> &'static str {
        "f64"
    }
}

impl ApiSchema for bool {
    fn schema() -> Value {
        serde_json::json!({"type": "boolean"})
    }
    fn schema_name() -> &'static str {
        "bool"
    }
}

impl ApiSchema for () {
    fn schema() -> Value {
        serde_json::json!({"type": "null"})
    }

    fn schema_name() -> &'static str {
        "Unit"
    }
}

impl ApiSchema for serde_json::Value {
    fn schema() -> Value {
        serde_json::json!({})
    }
    fn schema_name() -> &'static str {
        "JsonValue"
    }
}

impl ApiSchema for uuid::Uuid {
    fn schema() -> Value {
        serde_json::json!({"type": "string", "format": "uuid"})
    }

    fn schema_name() -> &'static str {
        "Uuid"
    }
}

impl<T: ApiSchema> ApiSchema for Option<T> {
    fn schema() -> Value {
        let mut s = T::schema();
        if let Some(obj) = s.as_object_mut() {
            obj.insert("nullable".into(), true.into());
        }
        s
    }
    fn schema_name() -> &'static str {
        T::schema_name()
    }
}

impl<T: ApiSchema> ApiSchema for Vec<T> {
    fn schema() -> Value {
        serde_json::json!({
            "type": "array",
            "items": T::schema(),
            "x-foundry-item-schema": T::schema_name(),
        })
    }
    fn schema_name() -> &'static str {
        "Array"
    }
}

impl<T: ApiSchema> ApiSchema for crate::support::Collection<T> {
    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "x-foundry-wrapper-schema": "Collection",
            "x-foundry-data-schema": T::schema_name(),
            "properties": {
                "items": {
                    "type": "array",
                    "items": T::schema(),
                    "x-foundry-item-schema": T::schema_name(),
                }
            },
            "required": ["items"]
        })
    }

    fn schema_name() -> &'static str {
        "Collection"
    }
}

impl<T: ApiSchema> ApiSchema for BTreeSet<T> {
    fn schema() -> Value {
        serde_json::json!({
            "type": "array",
            "items": T::schema(),
            "x-foundry-item-schema": T::schema_name(),
        })
    }
    fn schema_name() -> &'static str {
        "Array"
    }
}

impl<T: ApiSchema> ApiSchema for HashSet<T> {
    fn schema() -> Value {
        serde_json::json!({
            "type": "array",
            "items": T::schema(),
            "x-foundry-item-schema": T::schema_name(),
        })
    }
    fn schema_name() -> &'static str {
        "Array"
    }
}

impl<T: ApiSchema> ApiSchema for BTreeMap<String, T> {
    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": T::schema(),
            "x-foundry-additional-schema": T::schema_name(),
        })
    }
    fn schema_name() -> &'static str {
        "Map"
    }
}

impl<T: ApiSchema> ApiSchema for HashMap<String, T> {
    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": T::schema(),
            "x-foundry-additional-schema": T::schema_name(),
        })
    }
    fn schema_name() -> &'static str {
        "Map"
    }
}

/// Type-erased schema reference for route documentation.
#[derive(Clone)]
pub struct SchemaRef {
    pub name: &'static str,
    pub schema_fn: fn() -> Value,
}

impl SchemaRef {
    pub fn of<T: ApiSchema>() -> Self {
        Self {
            name: T::schema_name(),
            schema_fn: T::schema,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RouteDocRateLimit {
    pub(crate) max_requests: u32,
    pub(crate) window_seconds: u64,
    pub(crate) by: String,
}

/// Documentation for a single route.
#[derive(Clone, Default)]
pub struct RouteDoc {
    pub(crate) method: Option<String>,
    pub(crate) operation_id: Option<String>,
    pub(crate) route_id: Option<String>,
    pub(crate) middleware_group: Option<String>,
    pub(crate) audit_area: Option<String>,
    pub(crate) rate_limits: Vec<RouteDocRateLimit>,
    pub(crate) auth_required: bool,
    pub(crate) auth_guard: Option<String>,
    pub(crate) auth_permissions: Vec<String>,
    pub(crate) auth_allows_mfa_pending_token: bool,
    pub(crate) auth_has_authorize_callback: bool,
    pub(crate) summary: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) request: Option<SchemaRef>,
    pub(crate) responses: Vec<(u16, SchemaRef)>,
    pub(crate) deprecated: bool,
}

impl RouteDoc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn method(mut self, m: &str) -> Self {
        if let Some(method) = route_doc_metadata_value(m) {
            self.method = Some(method.to_lowercase());
        }
        self
    }

    pub fn operation_id(mut self, id: &str) -> Self {
        self.operation_id = route_doc_metadata_value(id);
        self
    }

    pub fn get(self) -> Self {
        self.method("get")
    }

    pub fn head(self) -> Self {
        self.method("head")
    }

    pub fn post(self) -> Self {
        self.method("post")
    }

    pub fn put(self) -> Self {
        self.method("put")
    }

    pub fn patch(self) -> Self {
        self.method("patch")
    }

    pub fn delete(self) -> Self {
        self.method("delete")
    }

    pub fn options(self) -> Self {
        self.method("options")
    }

    pub fn summary(mut self, s: &str) -> Self {
        self.summary = route_doc_metadata_value(s);
        self
    }

    pub fn description(mut self, d: &str) -> Self {
        self.description = route_doc_metadata_value(d);
        self
    }

    pub fn tag(mut self, t: &str) -> Self {
        let Some(tag) = route_doc_metadata_value(t) else {
            return self;
        };
        if !self.tags.iter().any(|existing| existing == &tag) {
            self.tags.push(tag);
        }
        self
    }

    pub fn request<T: ApiSchema>(mut self) -> Self {
        self.request = Some(SchemaRef::of::<T>());
        self
    }

    pub fn response<T: ApiSchema>(mut self, status: u16) -> Self {
        self.responses.push((status, SchemaRef::of::<T>()));
        self
    }

    /// Document Foundry's standard validation error envelope for `422` responses.
    ///
    /// This is an opt-in shortcut for routes that use `Validated<T>` or
    /// `JsonValidated<T>` extractors. It leaves any explicit `422` response
    /// untouched.
    pub fn validation_errors(mut self) -> Self {
        if !self.responses.iter().any(|(status, _)| *status == 422) {
            self.responses.push((
                422,
                SchemaRef::of::<crate::validation::ValidationErrorResponse>(),
            ));
        }
        self
    }

    pub fn deprecated(mut self) -> Self {
        self.deprecated = true;
        self
    }

    pub fn merge_defaults(mut self, defaults: &Self) -> Self {
        if self.method.is_none() {
            self.method = defaults.method.clone();
        }
        if self.summary.is_none() {
            self.summary = defaults.summary.clone();
        }
        if self.operation_id.is_none() {
            self.operation_id = defaults.operation_id.clone();
        }
        if !self.auth_required {
            self.auth_required = defaults.auth_required;
        }
        if self.auth_guard.is_none() {
            self.auth_guard = defaults.auth_guard.clone();
        }
        if self.auth_permissions.is_empty() {
            self.auth_permissions = defaults.auth_permissions.clone();
        }
        if defaults.auth_allows_mfa_pending_token {
            self.auth_allows_mfa_pending_token = true;
        }
        if defaults.auth_has_authorize_callback {
            self.auth_has_authorize_callback = true;
        }
        if self.description.is_none() {
            self.description = defaults.description.clone();
        }
        if self.request.is_none() {
            self.request = defaults.request.clone();
        }
        if self.responses.is_empty() {
            self.responses = defaults.responses.clone();
        }
        if defaults.deprecated {
            self.deprecated = true;
        }

        for tag in &defaults.tags {
            self = self.tag(tag);
        }

        self
    }
}

fn route_doc_metadata_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct SkippedFieldSchema {
        visible: String,
        #[serde(skip)]
        #[ts(skip)]
        internal: Vec<String>,
    }

    #[allow(dead_code)]
    #[derive(serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    #[serde(deny_unknown_fields)]
    struct StrictObjectSchema {
        id: String,
        name: String,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum ValidationStatus {
        Draft,
        #[foundry(aliases = ["live"])]
        Published,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum ValidationPriority {
        Low = 1,
        High = 2,
    }

    #[test]
    fn route_doc_tags_are_deduplicated_in_order() {
        let doc = RouteDoc::new()
            .tag("admin")
            .tag("admin")
            .tag("users")
            .tag("admin");

        assert_eq!(doc.tags, vec!["admin".to_string(), "users".to_string()]);
    }

    #[test]
    fn route_doc_metadata_is_trimmed_and_blank_values_are_ignored() {
        let doc = RouteDoc::new()
            .method(" GET ")
            .operation_id(" admin.users.index ")
            .summary(" List users ")
            .description("  Shows users.  ")
            .tag(" admin ")
            .tag("admin")
            .tag("   ");

        assert_eq!(doc.method.as_deref(), Some("get"));
        assert_eq!(doc.operation_id.as_deref(), Some("admin.users.index"));
        assert_eq!(doc.summary.as_deref(), Some("List users"));
        assert_eq!(doc.description.as_deref(), Some("Shows users."));
        assert_eq!(doc.tags, vec!["admin".to_string()]);

        let blank = RouteDoc::new()
            .method(" ")
            .operation_id("")
            .summary("   ")
            .description("\n\t")
            .tag("");

        assert!(blank.method.is_none());
        assert!(blank.operation_id.is_none());
        assert!(blank.summary.is_none());
        assert!(blank.description.is_none());
        assert!(blank.tags.is_empty());
    }

    #[test]
    fn route_doc_default_tags_reuse_deduplication_order() {
        let defaults = RouteDoc::new().tag("admin").tag("shared");
        let doc = RouteDoc::new()
            .tag("users")
            .tag("admin")
            .merge_defaults(&defaults);

        assert_eq!(
            doc.tags,
            vec![
                "users".to_string(),
                "admin".to_string(),
                "shared".to_string()
            ]
        );
    }

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    struct ValidationConstraintSchema {
        #[validate(required, min(3), max(32))]
        name: String,
        #[validate(
            required(message = "Email is required."),
            email(message = "Invalid email.")
        )]
        email: String,
        #[validate(min_numeric(1), max_numeric(10))]
        score: u32,
        #[validate(size(4))]
        exact_code: String,
        #[validate(size(10))]
        exact_seats: u32,
        #[validate(size(2))]
        exact_tags: Vec<String>,
        #[validate(
            min_items(1),
            max_items(5),
            contains("rust", "foundry"),
            doesnt_contain("legacy"),
            distinct,
            each(min(2, message = "Tag is too short."), max(50))
        )]
        tags: Vec<String>,
        #[validate(filled)]
        filled_tags: Vec<String>,
        #[validate(in_list("draft", "published"))]
        status: String,
        #[validate(in_list(1, 2))]
        status_code: i32,
        #[validate(in_list(1.5, 2.5))]
        status_ratio: f64,
        #[validate(not_in("root", "admin"))]
        username: String,
        #[validate(not_in(0, -1))]
        blocked_status_code: i32,
        #[validate(each(in_list("red", "green", "blue")))]
        colors: Vec<String>,
        #[validate(each(in_list(1, 2)))]
        color_codes: Vec<i32>,
        #[validate(
            regex("^[a-z0-9_-]+$"),
            starts_with("usr.", "acct."),
            ends_with(".id", ".key")
        )]
        slug: String,
        #[validate(regex("(?x)^ legacy-[0-9]+ $"))]
        legacy_code: String,
        #[validate(not_regex("^admin"))]
        public_username: String,
        #[validate(doesnt_start_with("admin.", "root."))]
        public_handle: String,
        #[validate(doesnt_end_with(".internal", ".local"))]
        public_domain: String,
        #[validate(contains("sku."))]
        sku: String,
        #[validate(doesnt_contain("legacy."))]
        safe_sku: String,
        #[validate(filled)]
        nickname: String,
        #[validate(alpha)]
        display_name: String,
        #[validate(alpha_num)]
        account_name: String,
        #[validate(alpha_dash)]
        username_slug: String,
        #[validate(ascii)]
        ascii_key: String,
        #[validate(ulid)]
        ulid_key: String,
        #[validate(uuid(4))]
        uuid_v4_key: String,
        #[validate(hex_color)]
        brand_color: String,
        #[validate(mac_address)]
        device_mac: String,
        #[validate(decimal(2, 4))]
        price: String,
        #[validate(multiple_of(0.25))]
        increment: f64,
        #[validate(digits)]
        pin: String,
        #[validate(min_digits(4))]
        min_pin: String,
        #[validate(max_digits(6))]
        max_pin: String,
        #[validate(digits_between(4, 6))]
        ranged_pin: String,
        #[validate(between(1.5, 9.5))]
        ratio: f64,
        #[validate(gt(0.0))]
        discount_rate: f64,
        #[validate(gte(1.0))]
        minimum_quantity: u32,
        #[validate(lt(100.0))]
        tax_rate: f64,
        #[validate(lte(10.0))]
        max_attempts: u32,
        #[validate(ipv4)]
        ipv4_address: String,
        #[validate(ipv6)]
        ipv6_address: String,
        #[validate(date)]
        starts_on: String,
        #[validate(time)]
        starts_at: String,
        #[validate(datetime)]
        published_at: String,
        #[validate(local_datetime)]
        local_publish_at: String,
        #[validate(timezone)]
        publish_timezone: String,
        #[validate(boolean)]
        enabled: String,
        #[validate(accepted)]
        terms: String,
        #[validate(accepted)]
        terms_bool: bool,
        #[validate(accepted_if("enabled", "true"))]
        conditional_terms: bool,
        #[validate(declined)]
        marketing_opt_out: String,
        #[validate(declined)]
        marketing_opt_out_bool: bool,
        #[validate(declined_if("enabled", "true"))]
        conditional_marketing_opt_out: bool,
        #[validate(app_enum(ValidationStatus))]
        workflow_status: ValidationStatus,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct AppEnumStringConstraintSchema {
        #[validate(app_enum(ValidationStatus))]
        workflow_status: String,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct AppEnumNumericConstraintSchema {
        #[validate(app_enum(ValidationPriority))]
        priority: i32,
        #[validate(app_enum(ValidationPriority))]
        priority_code: String,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, ts_rs::TS, crate::ApiSchema, crate::Validate)]
    struct FileValidationSchema {
        #[validate(
            max_file_size(2048),
            allowed_extensions("jpg", "png", "webp"),
            image,
            allowed_mimes("image/png", "image/jpeg", "image/webp"),
            max_dimensions(1024, 768),
            min_dimensions(128, 128)
        )]
        avatar: crate::storage::UploadedFile,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, ts_rs::TS, crate::ApiSchema, crate::Validate)]
    struct MultiFileValidationSchema {
        #[validate(
            min_items(1),
            max_items(4),
            max_file_size(4096),
            allowed_extensions("jpg", "png", "webp"),
            image,
            allowed_mimes("image/png", "image/jpeg", "image/webp")
        )]
        photos: Vec<crate::storage::UploadedFile>,
        #[validate(max_items(2), allowed_extensions("jpg", "png"))]
        optional_photos: Option<Vec<crate::storage::UploadedFile>>,
    }

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    struct PortableValidationConstraintSchema {
        #[validate(numeric)]
        amount_text: String,
        #[validate(integer)]
        whole_text: String,
        #[validate(integer)]
        whole_number: f64,
        #[validate(ip)]
        ip_address: String,
        #[validate(json)]
        json_text: String,
        #[validate(lowercase)]
        lowercase_slug: String,
        #[validate(uppercase)]
        uppercase_code: String,
    }

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    #[serde(rename_all = "camelCase")]
    struct ValidationMetadataSchema {
        enabled: bool,
        reviewer: String,
        second_reviewer: String,
        legacy_token: String,
        override_token: String,
        ends_at: String,
        #[validate(
            required_if("enabled", "true"),
            required_with_all("reviewer", "second_reviewer")
        )]
        audit_note: String,
        #[validate(confirmed)]
        password: String,
        password_confirmation: String,
        #[validate(same("password"))]
        repeated_password: String,
        #[validate(before_or_equal("ends_at"))]
        starts_at: String,
        #[validate(unique("users", "email"))]
        email: String,
        #[validate(prohibits("legacy_token", "override_token"))]
        exclusive_token: String,
        #[validate(required_keys("timezone", "locale"))]
        settings: std::collections::BTreeMap<String, String>,
        #[validate(nullable, bail, email)]
        optional_contact: String,
    }

    const MOBILE_RULE_ID: crate::support::ValidationRuleId =
        crate::support::ValidationRuleId::new("mobile");

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    #[serde(rename_all = "camelCase")]
    #[validate(after(validate_custom_rule_schema))]
    struct CustomRuleSchema {
        #[validate(required, rule("mobile"))]
        phone: String,
        #[validate(rule(MOBILE_RULE_ID))]
        typed_phone: String,
    }

    async fn validate_custom_rule_schema(
        _input: &CustomRuleSchema,
        _validator: &mut crate::validation::Validator,
    ) -> crate::foundation::Result<()> {
        Ok(())
    }

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    #[serde(rename_all = "camelCase")]
    struct NestedValidationChildSchema {
        #[validate(required)]
        street_name: String,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate)]
    #[serde(rename_all = "camelCase")]
    struct NestedValidationParentSchema {
        #[validate(nested)]
        primary_address: NestedValidationChildSchema,
        #[validate(each(nested))]
        previous_addresses: Vec<NestedValidationChildSchema>,
        #[validate(min_items(1), each(nested))]
        optional_previous_addresses: Option<Vec<NestedValidationChildSchema>>,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct JsonValueFieldSchema {
        payload: serde_json::Value,
        metadata: Option<serde_json::Value>,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate)]
    struct JsonValueValidationSchema {
        #[validate(json)]
        payload: serde_json::Value,
        #[validate(json)]
        metadata: Option<serde_json::Value>,
        #[validate(json)]
        raw: String,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct UnitFieldSchema {
        marker: (),
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct PrimitiveIntegerSchema {
        tiny: u8,
        port: u16,
        attempts: usize,
        signed_offset: isize,
    }

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    #[serde(rename_all = "camelCase")]
    struct RenamedFieldSchema {
        first_name: String,
        #[serde(rename = "emailAddress")]
        email: String,
        #[validate(required)]
        display_name: Option<String>,
        r#type: String,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    #[serde(rename_all = "snake_case")]
    enum RenamedEnumSchema {
        PendingReview,
        #[serde(rename = "done")]
        Completed,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    #[serde(rename_all = "camelCase")]
    struct FlattenedProfileSchema {
        first_name: String,
        display_name: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    #[serde(rename_all = "camelCase")]
    struct FlattenedOptionalSchema {
        audit_note: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct FlattenedEnvelopeSchema {
        id: String,
        #[serde(flatten)]
        profile: FlattenedProfileSchema,
        #[serde(flatten)]
        audit: FlattenedOptionalSchema,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct FlattenedDuplicateProfileSchema {
        id: String,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct FlattenedDuplicateEnvelopeSchema {
        id: String,
        #[serde(flatten)]
        profile: FlattenedDuplicateProfileSchema,
    }

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    struct DefaultedFieldSchema {
        #[serde(default)]
        #[ts(optional, as = "Option<_>")]
        page: u64,
        #[serde(default)]
        #[ts(optional, as = "Option<_>")]
        tags: Vec<String>,
        #[serde(default)]
        #[validate(required)]
        label: String,
        title: String,
    }

    #[allow(dead_code)]
    #[derive(Default, serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    #[serde(default)]
    struct DefaultedContainerSchema {
        #[ts(optional, as = "Option<_>")]
        page: u64,
        #[ts(optional, as = "Option<_>")]
        tags: Vec<String>,
        #[ts(optional, as = "Option<_>")]
        title: String,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, crate::ApiSchema)]
    struct SparseSerializedSchema {
        id: String,
        #[serde(skip_serializing_if = "String::is_empty")]
        #[ts(optional)]
        subtitle: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        note: Option<String>,
    }

    impl ts_rs::TS for SparseSerializedSchema {
        type WithoutGenerics = Self;

        fn name() -> String {
            "SparseSerializedSchema".to_string()
        }

        fn decl() -> String {
            concat!(
                "type SparseSerializedSchema = { ",
                "id: string, ",
                "subtitle?: string, ",
                "note?: string | null, ",
                "};",
            )
            .to_string()
        }

        fn decl_concrete() -> String {
            Self::decl()
        }

        fn inline() -> String {
            concat!(
                "{ ",
                "id: string, ",
                "subtitle?: string, ",
                "note?: string | null, ",
                "}",
            )
            .to_string()
        }

        fn inline_flattened() -> String {
            Self::inline()
        }

        fn output_path() -> Option<&'static std::path::Path> {
            Some(std::path::Path::new("SparseSerializedSchema.ts"))
        }
    }

    #[test]
    fn route_doc_validation_errors_documents_standard_422_once() {
        let doc = RouteDoc::new().validation_errors().validation_errors();

        assert_eq!(doc.responses.len(), 1);
        assert_eq!(doc.responses[0].0, 422);
        assert_eq!(doc.responses[0].1.name, "ValidationErrorResponse");

        let explicit = RouteDoc::new().response::<String>(422).validation_errors();

        assert_eq!(explicit.responses.len(), 1);
        assert_eq!(explicit.responses[0].0, 422);
        assert_eq!(explicit.responses[0].1.name, "String");
    }

    #[test]
    fn string_keyed_maps_expose_additional_property_schema() {
        let schema = <BTreeMap<String, u64> as ApiSchema>::schema();

        assert_eq!(schema["type"], serde_json::json!("object"));
        assert_eq!(
            schema["additionalProperties"],
            serde_json::json!({"type": "integer"})
        );
        assert_eq!(
            schema["x-foundry-additional-schema"],
            serde_json::json!("u64")
        );
    }

    #[test]
    fn set_schemas_match_serialized_arrays() {
        let btree_schema = <BTreeSet<String> as ApiSchema>::schema();
        let hash_schema = <HashSet<String> as ApiSchema>::schema();

        for schema in [&btree_schema, &hash_schema] {
            assert_eq!(schema["type"], serde_json::json!("array"));
            assert_eq!(schema["items"], serde_json::json!({"type": "string"}));
            assert_eq!(schema["x-foundry-item-schema"], serde_json::json!("String"));
        }
        assert_eq!(<HashSet<String> as ApiSchema>::schema_name(), "Array");
    }

    #[test]
    fn collection_schema_matches_serialized_items_object() {
        let schema = <crate::support::Collection<String> as ApiSchema>::schema();

        assert_eq!(schema["type"], serde_json::json!("object"));
        assert_eq!(
            schema["x-foundry-wrapper-schema"],
            serde_json::json!("Collection")
        );
        assert_eq!(schema["x-foundry-data-schema"], serde_json::json!("String"));
        assert_eq!(
            schema["properties"]["items"],
            serde_json::json!({
                "type": "array",
                "items": { "type": "string" },
                "x-foundry-item-schema": "String",
            })
        );
        assert_eq!(schema["required"], serde_json::json!(["items"]));
    }

    #[test]
    fn json_value_schema_is_unconstrained() {
        let schema = <serde_json::Value as ApiSchema>::schema();

        assert_eq!(schema, serde_json::json!({}));
    }

    #[test]
    fn unit_schema_documents_null_value() {
        assert_eq!(
            <() as ApiSchema>::schema(),
            serde_json::json!({"type": "null"})
        );
        assert_eq!(<() as ApiSchema>::schema_name(), "Unit");
    }

    #[test]
    fn uuid_schema_is_documented_as_uuid_string() {
        assert_eq!(
            <uuid::Uuid as ApiSchema>::schema(),
            serde_json::json!({"type": "string", "format": "uuid"})
        );
        assert_eq!(<uuid::Uuid as ApiSchema>::schema_name(), "Uuid");
    }

    #[test]
    fn timezone_schema_is_documented_as_timezone_string() {
        assert_eq!(
            <crate::support::Timezone as ApiSchema>::schema(),
            serde_json::json!({"type": "string", "format": "timezone"})
        );
        assert_eq!(
            <crate::support::Timezone as ApiSchema>::schema_name(),
            "Timezone"
        );
    }

    #[test]
    fn string_keyed_json_maps_allow_unknown_values() {
        let schema = <BTreeMap<String, serde_json::Value> as ApiSchema>::schema();

        assert_eq!(schema["type"], serde_json::json!("object"));
        assert_eq!(schema["additionalProperties"], serde_json::json!({}));
        assert_eq!(
            schema["x-foundry-additional-schema"],
            serde_json::json!("JsonValue")
        );
    }

    #[test]
    fn primitive_integer_schemas_cover_common_rust_widths() {
        assert_eq!(
            <u8 as ApiSchema>::schema(),
            serde_json::json!({"type": "integer", "format": "int32"})
        );
        assert_eq!(
            <u16 as ApiSchema>::schema(),
            serde_json::json!({"type": "integer", "format": "int32"})
        );
        assert_eq!(
            <usize as ApiSchema>::schema(),
            serde_json::json!({"type": "integer"})
        );
        assert_eq!(
            <isize as ApiSchema>::schema(),
            serde_json::json!({"type": "integer", "format": "int64"})
        );

        let schema = PrimitiveIntegerSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        assert_eq!(
            properties.get("tiny"),
            Some(&serde_json::json!({"type": "integer", "format": "int32"}))
        );
        assert_eq!(
            properties.get("port"),
            Some(&serde_json::json!({"type": "integer", "format": "int32"}))
        );
        assert_eq!(
            properties.get("attempts"),
            Some(&serde_json::json!({"type": "integer"}))
        );
        assert_eq!(
            properties.get("signed_offset"),
            Some(&serde_json::json!({"type": "integer", "format": "int64"}))
        );
    }

    #[test]
    fn derived_json_value_fields_are_unconstrained() {
        let schema = JsonValueFieldSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");

        assert_eq!(properties.get("payload"), Some(&serde_json::json!({})));
        assert_eq!(
            properties.get("metadata"),
            Some(&serde_json::json!({ "nullable": true }))
        );
    }

    #[test]
    fn json_value_validation_metadata_is_server_only_for_openapi_schema() {
        let schema = JsonValueValidationSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");

        let payload = properties.get("payload").expect("payload property");
        let metadata = properties.get("metadata").expect("metadata property");
        let raw = properties.get("raw").expect("raw property");

        assert_eq!(
            payload["x-foundry-validation"],
            serde_json::json!([{ "code": "json", "serverOnly": true }])
        );
        assert_eq!(metadata["nullable"], serde_json::json!(true));
        assert_eq!(
            metadata["x-foundry-validation"],
            serde_json::json!([{ "code": "json", "serverOnly": true }])
        );
        assert_eq!(raw["format"], serde_json::json!("json-string"));
        assert!(raw.get("x-foundry-validation").is_none());
    }

    #[test]
    fn derived_unit_fields_are_null_schemas() {
        let schema = UnitFieldSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");

        assert_eq!(
            properties.get("marker"),
            Some(&serde_json::json!({"type": "null"}))
        );
    }

    #[test]
    fn derived_schema_uses_serde_field_names() {
        let schema = RenamedFieldSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");

        assert!(properties.contains_key("firstName"));
        assert!(properties.contains_key("emailAddress"));
        assert!(properties.contains_key("displayName"));
        assert!(properties.contains_key("type"));
        assert!(!properties.contains_key("first_name"));
        assert!(!properties.contains_key("email"));
        assert!(!properties.contains_key("r#type"));
        assert_eq!(
            schema.get("required"),
            Some(&serde_json::json!([
                "firstName",
                "emailAddress",
                "displayName",
                "type"
            ]))
        );
    }

    #[test]
    fn derived_enum_schema_uses_serde_variant_names() {
        let schema = RenamedEnumSchema::schema();

        assert_eq!(schema["type"], serde_json::json!("string"));
        assert_eq!(
            schema["enum"],
            serde_json::json!(["pending_review", "done"])
        );
    }

    #[test]
    fn derived_schema_flattens_serde_flatten_fields() {
        let schema = FlattenedEnvelopeSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");

        assert!(properties.contains_key("id"));
        assert!(properties.contains_key("firstName"));
        assert!(properties.contains_key("displayName"));
        assert!(properties.contains_key("auditNote"));
        assert!(!properties.contains_key("profile"));
        assert!(!properties.contains_key("audit"));

        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("required fields");
        assert!(required.contains(&serde_json::json!("id")));
        assert!(required.contains(&serde_json::json!("firstName")));
        assert!(!required.contains(&serde_json::json!("displayName")));
        assert!(!required.contains(&serde_json::json!("auditNote")));
    }

    #[test]
    #[should_panic(
        expected = "OpenAPI schema `FlattenedDuplicateEnvelopeSchema` contains duplicate property `id` after applying serde field names and flattening"
    )]
    fn derived_schema_rejects_duplicate_flattened_properties() {
        let _ = FlattenedDuplicateEnvelopeSchema::schema();
    }

    #[test]
    fn derived_schema_treats_serde_default_fields_as_optional() {
        let schema = DefaultedFieldSchema::schema();
        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("required fields");

        assert!(!required.contains(&serde_json::json!("page")));
        assert!(!required.contains(&serde_json::json!("tags")));
        assert!(required.contains(&serde_json::json!("label")));
        assert!(required.contains(&serde_json::json!("title")));

        let container_schema = DefaultedContainerSchema::schema();
        assert!(container_schema.get("required").is_none());
    }

    #[test]
    fn derived_schema_treats_sparse_serialized_fields_as_optional() {
        let schema = SparseSerializedSchema::schema();
        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("required fields");

        assert!(required.contains(&serde_json::json!("id")));
        assert!(!required.contains(&serde_json::json!("subtitle")));
        assert!(!required.contains(&serde_json::json!("note")));
    }

    #[test]
    fn skipped_fields_are_excluded_from_derived_schema() {
        let schema = SkippedFieldSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");

        assert!(properties.contains_key("visible"));
        assert!(!properties.contains_key("internal"));
        assert_eq!(
            schema.get("required"),
            Some(&serde_json::json!(["visible"]))
        );
    }

    #[test]
    fn derived_schema_honors_serde_deny_unknown_fields() {
        let schema = StrictObjectSchema::schema();

        assert_eq!(
            schema.get("additionalProperties"),
            Some(&serde_json::Value::Bool(false))
        );
    }

    #[test]
    fn validate_min_max_aliases_are_exposed_as_json_schema_lengths() {
        let schema = ValidationConstraintSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        let name = properties.get("name").expect("name property");
        let email = properties.get("email").expect("email property");
        let score = properties.get("score").expect("score property");
        let exact_code = properties.get("exact_code").expect("exact_code property");
        let exact_seats = properties.get("exact_seats").expect("exact_seats property");
        let exact_tags = properties.get("exact_tags").expect("exact_tags property");
        let tags = properties.get("tags").expect("tags property");
        let tag_items = tags.get("items").expect("tags item schema");
        let filled_tags = properties.get("filled_tags").expect("filled_tags property");
        let status = properties.get("status").expect("status property");
        let status_code = properties.get("status_code").expect("status_code property");
        let status_ratio = properties
            .get("status_ratio")
            .expect("status_ratio property");
        let username = properties.get("username").expect("username property");
        let blocked_status_code = properties
            .get("blocked_status_code")
            .expect("blocked_status_code property");
        let color_items = properties
            .get("colors")
            .and_then(|schema| schema.get("items"))
            .expect("colors item schema");
        let color_code_items = properties
            .get("color_codes")
            .and_then(|schema| schema.get("items"))
            .expect("color_codes item schema");
        let slug = properties.get("slug").expect("slug property");
        let legacy_code = properties.get("legacy_code").expect("legacy_code property");
        let public_username = properties
            .get("public_username")
            .expect("public_username property");
        let public_handle = properties
            .get("public_handle")
            .expect("public_handle property");
        let public_domain = properties
            .get("public_domain")
            .expect("public_domain property");
        let sku = properties.get("sku").expect("sku property");
        let safe_sku = properties.get("safe_sku").expect("safe_sku property");
        let nickname = properties.get("nickname").expect("nickname property");
        let display_name = properties
            .get("display_name")
            .expect("display_name property");
        let account_name = properties
            .get("account_name")
            .expect("account_name property");
        let username_slug = properties
            .get("username_slug")
            .expect("username_slug property");
        let ascii_key = properties.get("ascii_key").expect("ascii_key property");
        let ulid_key = properties.get("ulid_key").expect("ulid_key property");
        let uuid_v4_key = properties.get("uuid_v4_key").expect("uuid_v4_key property");
        let brand_color = properties.get("brand_color").expect("brand_color property");
        let device_mac = properties.get("device_mac").expect("device_mac property");
        let price = properties.get("price").expect("price property");
        let increment = properties.get("increment").expect("increment property");
        let pin = properties.get("pin").expect("pin property");
        let min_pin = properties.get("min_pin").expect("min_pin property");
        let max_pin = properties.get("max_pin").expect("max_pin property");
        let ranged_pin = properties.get("ranged_pin").expect("ranged_pin property");
        let ratio = properties.get("ratio").expect("ratio property");
        let discount_rate = properties
            .get("discount_rate")
            .expect("discount_rate property");
        let minimum_quantity = properties
            .get("minimum_quantity")
            .expect("minimum_quantity property");
        let tax_rate = properties.get("tax_rate").expect("tax_rate property");
        let max_attempts = properties
            .get("max_attempts")
            .expect("max_attempts property");
        let ipv4_address = properties
            .get("ipv4_address")
            .expect("ipv4_address property");
        let ipv6_address = properties
            .get("ipv6_address")
            .expect("ipv6_address property");
        let starts_on = properties.get("starts_on").expect("starts_on property");
        let starts_at = properties.get("starts_at").expect("starts_at property");
        let published_at = properties
            .get("published_at")
            .expect("published_at property");
        let local_publish_at = properties
            .get("local_publish_at")
            .expect("local_publish_at property");
        let publish_timezone = properties
            .get("publish_timezone")
            .expect("publish_timezone property");
        let enabled = properties.get("enabled").expect("enabled property");
        let terms = properties.get("terms").expect("terms property");
        let terms_bool = properties.get("terms_bool").expect("terms_bool property");
        let conditional_terms = properties
            .get("conditional_terms")
            .expect("conditional_terms property");
        let marketing_opt_out = properties
            .get("marketing_opt_out")
            .expect("marketing_opt_out property");
        let marketing_opt_out_bool = properties
            .get("marketing_opt_out_bool")
            .expect("marketing_opt_out_bool property");
        let conditional_marketing_opt_out = properties
            .get("conditional_marketing_opt_out")
            .expect("conditional_marketing_opt_out property");
        let workflow_status = properties
            .get("workflow_status")
            .expect("workflow_status property");

        assert_eq!(name["minLength"], serde_json::json!(3));
        assert_eq!(name["maxLength"], serde_json::json!(32));
        assert_eq!(email["format"], serde_json::json!("email"));
        assert_eq!(score["minimum"], serde_json::json!(1));
        assert_eq!(score["maximum"], serde_json::json!(10));
        assert_eq!(exact_code["minLength"], serde_json::json!(4));
        assert_eq!(exact_code["maxLength"], serde_json::json!(4));
        assert_eq!(exact_seats["minimum"], serde_json::json!(10.0));
        assert_eq!(exact_seats["maximum"], serde_json::json!(10.0));
        assert_eq!(exact_tags["minItems"], serde_json::json!(2));
        assert_eq!(exact_tags["maxItems"], serde_json::json!(2));
        assert_eq!(
            exact_tags["x-foundry-item-schema"],
            serde_json::json!("String")
        );
        assert_eq!(tags["minItems"], serde_json::json!(1));
        assert_eq!(tags["maxItems"], serde_json::json!(5));
        assert_eq!(tags["uniqueItems"], serde_json::json!(true));
        assert_eq!(
            tags["allOf"],
            serde_json::json!([
                { "contains": { "const": "rust" } },
                { "contains": { "const": "foundry" } },
                { "not": { "contains": { "enum": ["legacy"] } } },
            ])
        );
        assert_eq!(tag_items["minLength"], serde_json::json!(2));
        assert_eq!(tag_items["maxLength"], serde_json::json!(50));
        assert_eq!(filled_tags["minItems"], serde_json::json!(1));
        assert_eq!(status["enum"], serde_json::json!(["draft", "published"]));
        assert_eq!(status_code["enum"], serde_json::json!([1, 2]));
        assert_eq!(status_ratio["enum"], serde_json::json!([1.5, 2.5]));
        assert_eq!(
            username["not"],
            serde_json::json!({ "enum": ["root", "admin"] })
        );
        assert_eq!(
            blocked_status_code["not"],
            serde_json::json!({ "enum": [0, -1] })
        );
        assert_eq!(
            color_items["enum"],
            serde_json::json!(["red", "green", "blue"])
        );
        assert_eq!(color_code_items["enum"], serde_json::json!([1, 2]));
        assert_eq!(
            slug["allOf"],
            serde_json::json!([
                { "pattern": "^[a-z0-9_-]+$" },
                {
                    "anyOf": [
                        { "pattern": "^usr\\." },
                        { "pattern": "^acct\\." },
                    ]
                },
                {
                    "anyOf": [
                        { "pattern": "\\.id$" },
                        { "pattern": "\\.key$" },
                    ]
                },
            ])
        );
        assert!(legacy_code.get("pattern").is_none());
        assert_eq!(
            legacy_code["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "regex",
                    "params": { "pattern": "(?x)^ legacy-[0-9]+ $" },
                    "serverOnly": true,
                }
            ])
        );
        assert_eq!(
            public_username["not"],
            serde_json::json!({ "pattern": "^admin" })
        );
        assert_eq!(
            public_handle["not"],
            serde_json::json!({
                "anyOf": [
                    { "pattern": "^admin\\." },
                    { "pattern": "^root\\." },
                ]
            })
        );
        assert_eq!(
            public_domain["not"],
            serde_json::json!({
                "anyOf": [
                    { "pattern": "\\.internal$" },
                    { "pattern": "\\.local$" },
                ]
            })
        );
        assert_eq!(sku["pattern"], serde_json::json!("sku\\."));
        assert_eq!(
            safe_sku["not"],
            serde_json::json!({ "pattern": "legacy\\." })
        );
        assert_eq!(nickname["pattern"], serde_json::json!(r"\S"));
        assert_eq!(
            display_name["pattern"],
            serde_json::json!(r"^[\p{L}\p{M}]*$")
        );
        assert_eq!(
            account_name["pattern"],
            serde_json::json!(r"^[\p{L}\p{M}\p{N}]*$")
        );
        assert_eq!(
            username_slug["pattern"],
            serde_json::json!(r"^[\p{L}\p{M}\p{N}_-]*$")
        );
        assert_eq!(ascii_key["pattern"], serde_json::json!(r"^[\x00-\x7F]*$"));
        assert_eq!(
            ulid_key["pattern"],
            serde_json::json!("^[0-7][0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{25}$")
        );
        assert_eq!(uuid_v4_key["format"], serde_json::json!("uuid"));
        assert_eq!(
            uuid_v4_key["pattern"],
            serde_json::json!(
                "^(?:[0-9a-fA-F]{12}4[0-9a-fA-F]{19}|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-4[0-9a-fA-F]{3}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}|\\{[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-4[0-9a-fA-F]{3}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\\}|urn:uuid:[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-4[0-9a-fA-F]{3}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})$"
            )
        );
        assert_eq!(
            brand_color["pattern"],
            serde_json::json!("^#(?:[0-9A-Fa-f]{3}|[0-9A-Fa-f]{4}|[0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$")
        );
        assert_eq!(
            device_mac["pattern"],
            serde_json::json!(
                "^(?:(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}|(?:[0-9A-Fa-f]{2}-){5}[0-9A-Fa-f]{2})$"
            )
        );
        assert_eq!(
            price["pattern"],
            serde_json::json!("^[+-]?(?:[0-9]+\\.[0-9]{2,4}|\\.[0-9]{2,4})$")
        );
        assert_eq!(increment["multipleOf"], serde_json::json!(0.25));
        assert_eq!(pin["pattern"], serde_json::json!("^[0-9]*$"));
        assert_eq!(min_pin["pattern"], serde_json::json!("^[0-9]*$"));
        assert_eq!(min_pin["minLength"], serde_json::json!(4));
        assert_eq!(max_pin["pattern"], serde_json::json!("^[0-9]*$"));
        assert_eq!(max_pin["maxLength"], serde_json::json!(6));
        assert_eq!(ranged_pin["pattern"], serde_json::json!("^[0-9]*$"));
        assert_eq!(ranged_pin["minLength"], serde_json::json!(4));
        assert_eq!(ranged_pin["maxLength"], serde_json::json!(6));
        assert_eq!(ratio["minimum"], serde_json::json!(1.5));
        assert_eq!(ratio["maximum"], serde_json::json!(9.5));
        assert_eq!(discount_rate["exclusiveMinimum"], serde_json::json!(0.0));
        assert_eq!(minimum_quantity["minimum"], serde_json::json!(1.0));
        assert_eq!(tax_rate["exclusiveMaximum"], serde_json::json!(100.0));
        assert_eq!(max_attempts["maximum"], serde_json::json!(10.0));
        assert_eq!(ipv4_address["format"], serde_json::json!("ipv4"));
        assert_eq!(ipv6_address["format"], serde_json::json!("ipv6"));
        assert_eq!(starts_on["format"], serde_json::json!("date"));
        assert_eq!(starts_at["format"], serde_json::json!("time"));
        assert_eq!(published_at["format"], serde_json::json!("date-time"));
        assert_eq!(local_publish_at["format"], serde_json::json!("date-time"));
        assert_eq!(publish_timezone["format"], serde_json::json!("timezone"));
        assert_eq!(
            enabled["enum"],
            serde_json::json!(["true", "false", "1", "0"])
        );
        assert_eq!(terms["enum"], serde_json::json!(["yes", "on", "1", "true"]));
        assert_eq!(terms_bool["enum"], serde_json::json!([true]));
        assert_eq!(conditional_terms["type"], serde_json::json!("boolean"));
        assert!(conditional_terms.get("enum").is_none());
        assert_eq!(
            marketing_opt_out["enum"],
            serde_json::json!(["no", "off", "0", "false"])
        );
        assert_eq!(marketing_opt_out_bool["enum"], serde_json::json!([false]));
        assert_eq!(
            conditional_marketing_opt_out["type"],
            serde_json::json!("boolean")
        );
        assert!(conditional_marketing_opt_out.get("enum").is_none());
        assert_eq!(
            workflow_status["enum"],
            serde_json::json!(["draft", "published"])
        );

        let schema = AppEnumStringConstraintSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        assert_eq!(
            properties
                .get("workflow_status")
                .expect("workflow_status property")["enum"],
            serde_json::json!(["draft", "published", "live"])
        );

        let schema = AppEnumNumericConstraintSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        assert_eq!(
            properties.get("priority").expect("priority property")["enum"],
            serde_json::json!([1, 2])
        );
        assert_eq!(
            properties
                .get("priority_code")
                .expect("priority_code property")["enum"],
            serde_json::json!(["1", "2"])
        );
    }

    #[test]
    fn file_validation_metadata_is_exposed_as_openapi_vendor_extensions() {
        let schema = FileValidationSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        let avatar = properties.get("avatar").expect("avatar property");

        assert_eq!(avatar["type"], serde_json::json!("string"));
        assert_eq!(avatar["format"], serde_json::json!("binary"));
        assert_eq!(
            avatar["x-foundry-max-file-size-kb"],
            serde_json::json!(2048)
        );
        assert_eq!(
            avatar["x-foundry-allowed-extensions"],
            serde_json::json!(["jpg", "png", "webp"])
        );
        assert_eq!(
            avatar["x-foundry-server-only-validation"],
            serde_json::json!(["image", "allowed_mimes", "max_dimensions", "min_dimensions"])
        );
        assert_eq!(
            avatar["x-foundry-allowed-mimes"],
            serde_json::json!(["image/png", "image/jpeg", "image/webp"])
        );
        assert_eq!(
            avatar["x-foundry-max-dimensions"],
            serde_json::json!({"width": 1024, "height": 768})
        );
        assert_eq!(
            avatar["x-foundry-min-dimensions"],
            serde_json::json!({"width": 128, "height": 128})
        );
    }

    #[test]
    fn multi_file_validation_metadata_is_exposed_on_openapi_array_items() {
        let schema = MultiFileValidationSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        let photos = properties.get("photos").expect("photos property");
        let photo_items = photos.get("items").expect("photos item schema");
        let optional_photos = properties
            .get("optional_photos")
            .expect("optional_photos property");
        let optional_photo_items = optional_photos
            .get("items")
            .expect("optional_photos item schema");

        assert_eq!(photos["type"], serde_json::json!("array"));
        assert_eq!(photos["minItems"], serde_json::json!(1));
        assert_eq!(photos["maxItems"], serde_json::json!(4));
        assert!(photos.get("x-foundry-max-file-size-kb").is_none());
        assert!(photos.get("x-foundry-allowed-extensions").is_none());
        assert_eq!(photo_items["type"], serde_json::json!("string"));
        assert_eq!(photo_items["format"], serde_json::json!("binary"));
        assert_eq!(
            photo_items["x-foundry-max-file-size-kb"],
            serde_json::json!(4096)
        );
        assert_eq!(
            photo_items["x-foundry-allowed-extensions"],
            serde_json::json!(["jpg", "png", "webp"])
        );
        assert_eq!(
            photo_items["x-foundry-server-only-validation"],
            serde_json::json!(["image", "allowed_mimes"])
        );
        assert_eq!(
            photo_items["x-foundry-allowed-mimes"],
            serde_json::json!(["image/png", "image/jpeg", "image/webp"])
        );

        assert_eq!(optional_photos["nullable"], serde_json::json!(true));
        assert_eq!(optional_photos["maxItems"], serde_json::json!(2));
        assert!(optional_photos
            .get("x-foundry-allowed-extensions")
            .is_none());
        assert_eq!(
            optional_photo_items["x-foundry-allowed-extensions"],
            serde_json::json!(["jpg", "png"])
        );
    }

    #[test]
    fn portable_validation_rules_are_exposed_as_openapi_constraints() {
        let schema = PortableValidationConstraintSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        let amount_text = properties.get("amount_text").expect("amount_text property");
        let whole_text = properties.get("whole_text").expect("whole_text property");
        let whole_number = properties
            .get("whole_number")
            .expect("whole_number property");
        let ip_address = properties.get("ip_address").expect("ip_address property");
        let json_text = properties.get("json_text").expect("json_text property");
        let lowercase_slug = properties
            .get("lowercase_slug")
            .expect("lowercase_slug property");
        let uppercase_code = properties
            .get("uppercase_code")
            .expect("uppercase_code property");

        assert_eq!(
            amount_text["pattern"],
            serde_json::json!(r"^[+-]?(?:(?:\d+(?:\.\d*)?)|(?:\.\d+))(?:[eE][+-]?\d+)?$")
        );
        assert_eq!(whole_text["pattern"], serde_json::json!(r"^[+-]?\d+$"));
        assert_eq!(
            whole_text["x-foundry-integer-format"],
            serde_json::json!("i64")
        );
        assert_eq!(whole_number["multipleOf"], serde_json::json!(1));
        assert_eq!(
            whole_number["x-foundry-integer-format"],
            serde_json::json!("i64")
        );
        assert_eq!(ip_address["format"], serde_json::json!("ip"));
        assert_eq!(json_text["format"], serde_json::json!("json-string"));
        assert_eq!(
            lowercase_slug["pattern"],
            serde_json::json!(r"^[^\p{Lu}]*$")
        );
        assert_eq!(
            uppercase_code["pattern"],
            serde_json::json!(r"^[^\p{Ll}]*$")
        );
    }

    #[test]
    fn validation_only_rules_are_exposed_as_openapi_vendor_metadata() {
        let schema = ValidationMetadataSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        let audit_note = properties.get("auditNote").expect("auditNote property");
        let password = properties.get("password").expect("password property");
        let repeated_password = properties
            .get("repeatedPassword")
            .expect("repeatedPassword property");
        let starts_at = properties.get("startsAt").expect("startsAt property");
        let email = properties.get("email").expect("email property");
        let exclusive_token = properties
            .get("exclusiveToken")
            .expect("exclusiveToken property");
        let settings = properties.get("settings").expect("settings property");
        let optional_contact = properties
            .get("optionalContact")
            .expect("optionalContact property");

        assert_eq!(
            audit_note["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "required_if",
                    "params": { "other": "enabled", "value": "true" },
                },
                {
                    "code": "required_with_all",
                    "params": { "other": "reviewer, secondReviewer" },
                    "values": ["reviewer", "secondReviewer"],
                },
            ])
        );
        assert_eq!(
            password["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "confirmed",
                    "params": { "other": "passwordConfirmation" },
                },
            ])
        );
        assert_eq!(
            repeated_password["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "same",
                    "params": { "other": "password" },
                },
            ])
        );
        assert_eq!(
            starts_at["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "before_or_equal",
                    "params": { "other": "endsAt" },
                },
            ])
        );
        assert_eq!(
            email["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "unique",
                    "params": { "table": "users", "column": "email" },
                    "serverOnly": true,
                },
            ])
        );
        assert_eq!(
            exclusive_token["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "prohibits",
                    "params": { "other": "legacyToken, overrideToken" },
                    "values": ["legacyToken", "overrideToken"],
                },
            ])
        );
        assert_eq!(
            settings["required"],
            serde_json::json!(["timezone", "locale"])
        );
        assert_eq!(
            settings["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "required_keys",
                    "values": ["timezone", "locale"],
                },
            ])
        );
        assert_eq!(optional_contact["format"], serde_json::json!("email"));
        assert_eq!(
            optional_contact["x-foundry-validation"],
            serde_json::json!([{ "code": "nullable" }, { "code": "bail" }])
        );
    }

    #[test]
    fn custom_validation_rules_are_exposed_as_server_only_openapi_metadata() {
        let schema = CustomRuleSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        let phone = properties.get("phone").expect("phone property");
        let typed_phone = properties.get("typedPhone").expect("typedPhone property");

        assert_eq!(
            schema["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "after",
                    "params": { "hook": "validate_custom_rule_schema" },
                    "serverOnly": true,
                },
            ])
        );
        assert_eq!(
            phone["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "mobile",
                    "params": { "rule": "mobile" },
                    "serverOnly": true,
                },
            ])
        );
        assert_eq!(
            typed_phone["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "mobile",
                    "params": { "rule": "mobile" },
                    "serverOnly": true,
                },
            ])
        );
    }

    #[test]
    fn nested_validation_rules_are_exposed_as_openapi_vendor_metadata() {
        let schema = NestedValidationParentSchema::schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema properties");
        let primary_address = properties
            .get("primaryAddress")
            .expect("primaryAddress property");
        let previous_addresses = properties
            .get("previousAddresses")
            .expect("previousAddresses property");
        let optional_previous_addresses = properties
            .get("optionalPreviousAddresses")
            .expect("optionalPreviousAddresses property");

        assert_eq!(
            primary_address["x-foundry-validation"],
            serde_json::json!([{ "code": "nested" }])
        );
        assert_eq!(
            previous_addresses["items"]["x-foundry-validation"],
            serde_json::json!([{ "code": "nested" }])
        );
        assert_eq!(
            optional_previous_addresses["nullable"],
            serde_json::json!(true)
        );
        assert_eq!(
            optional_previous_addresses["minItems"],
            serde_json::json!(1)
        );
        assert_eq!(
            optional_previous_addresses["items"]["x-foundry-validation"],
            serde_json::json!([{ "code": "nested" }])
        );
    }
}
