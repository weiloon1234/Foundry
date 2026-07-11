use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

pub mod spec;

use serde_json::Value;

use crate::contract::ContractParameterLocation;

/// Trait implemented by types that can generate an OpenAPI JSON Schema.
/// Derive with `#[derive(ApiSchema)]`.
pub trait ApiSchema: 'static {
    fn schema() -> Value;
    fn schema_name() -> &'static str;
}

/// A compile-time registered API schema for contract and documentation export.
pub struct ApiSchemaDefinition {
    pub name: &'static str,
    pub schema_fn: fn() -> Value,
}

inventory::collect!(ApiSchemaDefinition);

// Built-in impls for common types

impl ApiSchema for String {
    fn schema() -> Value {
        serde_json::json!({"type": "string"})
    }
    fn schema_name() -> &'static str {
        "String"
    }
}

impl ApiSchema for i32 {
    fn schema() -> Value {
        serde_json::json!({"type": "integer", "format": "int32"})
    }
    fn schema_name() -> &'static str {
        "i32"
    }
}

impl ApiSchema for i64 {
    fn schema() -> Value {
        serde_json::json!({"type": "integer", "format": "int64"})
    }
    fn schema_name() -> &'static str {
        "i64"
    }
}

impl ApiSchema for u64 {
    fn schema() -> Value {
        serde_json::json!({"type": "integer", "format": "uint64"})
    }
    fn schema_name() -> &'static str {
        "u64"
    }
}

impl ApiSchema for u32 {
    fn schema() -> Value {
        serde_json::json!({"type": "integer", "format": "uint32"})
    }
    fn schema_name() -> &'static str {
        "u32"
    }
}

impl ApiSchema for u16 {
    fn schema() -> Value {
        serde_json::json!({"type": "integer", "format": "uint16"})
    }
    fn schema_name() -> &'static str {
        "u16"
    }
}

impl ApiSchema for usize {
    fn schema() -> Value {
        serde_json::json!({"type": "integer", "format": "uint64"})
    }
    fn schema_name() -> &'static str {
        "usize"
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

impl ApiSchema for serde_json::Value {
    fn schema() -> Value {
        serde_json::json!({})
    }
    fn schema_name() -> &'static str {
        "JsonValue"
    }
}

impl<T: ApiSchema> ApiSchema for Option<T> {
    fn schema() -> Value {
        nullable_schema(T::schema())
    }
    fn schema_name() -> &'static str {
        structural_schema_name::<Option<T>>("Nullable", T::schema_name())
    }
}

impl<T: ApiSchema> ApiSchema for Vec<T> {
    fn schema() -> Value {
        serde_json::json!({"type": "array", "items": T::schema()})
    }
    fn schema_name() -> &'static str {
        structural_schema_name::<Vec<T>>("ArrayOf", T::schema_name())
    }
}

impl<T: ApiSchema> ApiSchema for BTreeMap<String, T> {
    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": T::schema(),
        })
    }
    fn schema_name() -> &'static str {
        structural_schema_name::<BTreeMap<String, T>>("RecordOf", T::schema_name())
    }
}

/// Wrap a JSON Schema so it accepts either the original value or JSON `null`.
///
/// Foundry emits OpenAPI 3.1, where JSON Schema unions replace the obsolete OpenAPI 3.0
/// `nullable` keyword.
pub fn nullable_schema(schema: Value) -> Value {
    serde_json::json!({
        "anyOf": [schema, {"type": "null"}]
    })
}

pub(crate) fn structural_schema_name<T: 'static>(prefix: &str, inner: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashMap<TypeId, &'static str>>> = OnceLock::new();
    let names = NAMES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut names = names
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(&name) = names.get(&TypeId::of::<T>()) {
        return name;
    }

    let name: &'static str = Box::leak(format!("{prefix}{inner}").into_boxed_str());
    names.insert(TypeId::of::<T>(), name);
    name
}

/// Type-erased schema reference for route documentation.
#[derive(Clone)]
pub struct SchemaRef {
    pub name: &'static str,
    pub schema_fn: fn() -> Value,
}

/// Type-erased schema metadata for one HTTP action parameter.
#[derive(Clone)]
pub(crate) struct ParameterRef {
    pub name: String,
    pub location: ContractParameterLocation,
    pub required: bool,
    pub schema: SchemaRef,
}

/// Type-erased schema metadata for one action-specific error response.
#[derive(Clone)]
pub(crate) struct ErrorRef {
    pub code: String,
    pub status: u16,
    pub schema: Option<SchemaRef>,
}

impl SchemaRef {
    pub fn of<T: ApiSchema>() -> Self {
        Self {
            name: T::schema_name(),
            schema_fn: T::schema,
        }
    }
}

/// Documentation for a single route.
#[derive(Clone, Default)]
pub struct RouteDoc {
    pub(crate) action_name: Option<String>,
    pub(crate) method: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) request: Option<SchemaRef>,
    pub(crate) request_content_type: Option<String>,
    pub(crate) parameters: Vec<ParameterRef>,
    pub(crate) responses: Vec<(u16, SchemaRef)>,
    pub(crate) errors: Vec<ErrorRef>,
    pub(crate) deprecated: bool,
}

impl RouteDoc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn method(mut self, m: &str) -> Self {
        self.method = Some(m.to_lowercase());
        self
    }

    /// Set the business action name used by generated contracts and SDKs.
    ///
    /// This is intentionally independent from the route ID, which remains
    /// transport metadata for URL generation.
    pub fn action_name(mut self, action_name: impl Into<String>) -> Self {
        self.action_name = Some(action_name.into());
        self
    }

    pub fn get(self) -> Self {
        self.method("get")
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

    pub fn summary(mut self, s: &str) -> Self {
        self.summary = Some(s.into());
        self
    }

    pub fn description(mut self, d: &str) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn tag(mut self, t: &str) -> Self {
        self.tags.push(t.into());
        self
    }

    pub fn request<T: ApiSchema>(mut self) -> Self {
        self.request = Some(SchemaRef::of::<T>());
        self
    }

    /// Override the request media type rendered into the OpenAPI contract.
    pub fn request_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.request_content_type = Some(content_type.into());
        self
    }

    pub fn parameter<T: ApiSchema>(
        mut self,
        name: impl Into<String>,
        location: ContractParameterLocation,
        required: bool,
    ) -> Self {
        let name = name.into();
        let parameter = ParameterRef {
            name: name.clone(),
            location,
            required,
            schema: SchemaRef::of::<T>(),
        };
        if let Some(existing) = self
            .parameters
            .iter_mut()
            .find(|existing| existing.name == name && existing.location == location)
        {
            *existing = parameter;
        } else {
            self.parameters.push(parameter);
        }
        self
    }

    pub fn path_parameter<T: ApiSchema>(self, name: impl Into<String>) -> Self {
        self.parameter::<T>(name, ContractParameterLocation::Path, true)
    }

    pub fn query_parameter<T: ApiSchema>(self, name: impl Into<String>, required: bool) -> Self {
        self.parameter::<T>(name, ContractParameterLocation::Query, required)
    }

    pub fn header_parameter<T: ApiSchema>(self, name: impl Into<String>, required: bool) -> Self {
        self.parameter::<T>(name, ContractParameterLocation::Header, required)
    }

    pub fn cookie_parameter<T: ApiSchema>(self, name: impl Into<String>, required: bool) -> Self {
        self.parameter::<T>(name, ContractParameterLocation::Cookie, required)
    }

    pub fn response<T: ApiSchema>(mut self, status: u16) -> Self {
        self.responses.push((status, SchemaRef::of::<T>()));
        self
    }

    pub fn error<T: ApiSchema>(mut self, status: u16, code: impl Into<String>) -> Self {
        self.errors.push(ErrorRef {
            code: code.into(),
            status,
            schema: Some(SchemaRef::of::<T>()),
        });
        self
    }

    pub fn error_without_schema(mut self, status: u16, code: impl Into<String>) -> Self {
        self.errors.push(ErrorRef {
            code: code.into(),
            status,
            schema: None,
        });
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
        if self.description.is_none() {
            self.description = defaults.description.clone();
        }
        if self.request.is_none() {
            self.request = defaults.request.clone();
        }
        if self.request_content_type.is_none() {
            self.request_content_type = defaults.request_content_type.clone();
        }
        if self.responses.is_empty() {
            self.responses = defaults.responses.clone();
        }
        for parameter in &defaults.parameters {
            if !self.parameters.iter().any(|existing| {
                existing.name == parameter.name && existing.location == parameter.location
            }) {
                self.parameters.push(parameter.clone());
            }
        }
        for error in &defaults.errors {
            if !self
                .errors
                .iter()
                .any(|existing| existing.code == error.code && existing.status == error.status)
            {
                self.errors.push(error.clone());
            }
        }
        if defaults.deprecated {
            self.deprecated = true;
        }

        for tag in &defaults.tags {
            if !self.tags.contains(tag) {
                self.tags.push(tag.clone());
            }
        }

        self
    }
}
