pub mod spec;

use serde_json::Value;

/// Trait implemented by types that can generate an OpenAPI JSON Schema.
/// Derive with `#[derive(ApiSchema)]`.
pub trait ApiSchema {
    fn schema() -> Value;
    fn schema_name() -> &'static str;
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
        serde_json::json!({"type": "object"})
    }
    fn schema_name() -> &'static str {
        "JsonValue"
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
        serde_json::json!({"type": "array", "items": T::schema()})
    }
    fn schema_name() -> &'static str {
        "Array"
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

/// Documentation for a single route.
#[derive(Clone, Default)]
pub struct RouteDoc {
    pub(crate) method: Option<String>,
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
        self.method = Some(m.to_lowercase());
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

    pub fn response<T: ApiSchema>(mut self, status: u16) -> Self {
        self.responses.push((status, SchemaRef::of::<T>()));
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
        if self.responses.is_empty() {
            self.responses = defaults.responses.clone();
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
