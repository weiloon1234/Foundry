use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::RouteDoc;
use crate::foundation::{Error, Result};
use crate::support::dotted_ids::{insert_dotted_id, DottedIdTreeNode};
use crate::validation::ValidationRuleDescriptor;

pub struct DocumentedRoute {
    pub method: String,
    pub path: String,
    pub doc: RouteDoc,
}

pub fn generate_openapi_spec(title: &str, version: &str, routes: &[DocumentedRoute]) -> Value {
    try_generate_openapi_spec(title, version, routes)
        .unwrap_or_else(|error| panic!("failed to generate OpenAPI spec: {error}"))
}

pub fn try_generate_openapi_spec(
    title: &str,
    version: &str,
    routes: &[DocumentedRoute],
) -> Result<Value> {
    try_generate_openapi_spec_with_validation_rules(title, version, routes, &[], false)
}

pub(crate) fn try_generate_openapi_spec_with_validation_rules(
    title: &str,
    version: &str,
    routes: &[DocumentedRoute],
    validation_rules: &[ValidationRuleDescriptor],
    validation_rule_manifest_authoritative: bool,
) -> Result<Value> {
    let mut paths: BTreeMap<String, Value> = BTreeMap::new();
    let mut schemas: BTreeMap<String, Value> = BTreeMap::new();
    let mut documented_routes = BTreeSet::new();
    let mut operation_ids = BTreeSet::new();
    let mut route_ids = DottedIdTreeNode::default();

    for route in routes {
        ensure_openapi_route_path(&route.path)?;
        let path = openapi_path_template(&route.path);
        let method = supported_openapi_route_method(&route.method, &path)?;
        if !documented_routes.insert((method.clone(), path.clone())) {
            return Err(Error::message(format!(
                "OpenAPI route `{method} {path}` is documented multiple times; keep one documented route per method/path"
            )));
        }
        ensure_route_document_metadata(&route.doc, &method, &path)?;

        let mut operation = json!({});
        if let Some(ref operation_id) = route.doc.operation_id {
            if !operation_ids.insert(operation_id.clone()) {
                return Err(Error::message(format!(
                    "OpenAPI operationId `{operation_id}` is used by multiple routes; operation ids must be unique"
                )));
            }
            operation["operationId"] = json!(operation_id);
        }
        if let Some(ref route_id) = route.doc.route_id {
            insert_dotted_id(
                &mut route_ids,
                route_id,
                "OpenAPI route id export",
                "route id",
            )?;
            operation["x-foundry-route-id"] = json!(route_id);
        }
        if let Some(ref summary) = route.doc.summary {
            operation["summary"] = json!(summary);
        }
        if let Some(ref desc) = route.doc.description {
            operation["description"] = json!(desc);
        }
        if !route.doc.tags.is_empty() {
            operation["tags"] = json!(route.doc.tags);
        }
        if route.doc.deprecated {
            operation["deprecated"] = json!(true);
        }
        if let Some(policy) = route_policy_extension(&route.doc, &method, &path)? {
            operation["x-foundry-route-policy"] = policy;
        }
        if let Some(auth) = route_auth_extension(&route.doc, &method, &path)? {
            operation["x-foundry-auth"] = auth;
        }
        let path_params = route_path_parameters(&route.path);
        if !path_params.is_empty() {
            operation["parameters"] = json!(path_params);
        }

        if let Some(ref req) = route.doc.request {
            let raw_schema = route_schema_with_registered_validation(
                req.name,
                (req.schema_fn)(),
                validation_rules,
                validation_rule_manifest_authoritative,
            )?;
            crate::http::route_manifest_schema_name(req).map_err(|error| {
                Error::message(format!(
                    "OpenAPI route `{method} {path}` request has invalid schema metadata: {error}"
                ))
            })?;
            let request_transport =
                crate::http::route_request_transport(Some(&method), &raw_schema);
            operation["x-foundry-request-transport"] = json!(request_transport.as_str());

            if request_transport == crate::http::RouteRequestTransport::Query {
                append_query_request_validation_metadata(&mut operation, &raw_schema);
                append_operation_parameters(&mut operation, route_query_parameters(&raw_schema));
            } else {
                let media_type = crate::http::route_request_media_type(&raw_schema);
                operation["x-foundry-request-media-type"] = json!(media_type.as_str());
                let schema =
                    route_request_media_schema(req.name, raw_schema, media_type, &mut schemas)?;
                let mut content = serde_json::Map::new();
                content.insert(media_type.as_str().to_string(), json!({ "schema": schema }));
                operation["requestBody"] = json!({
                    "required": true,
                    "content": content
                });
            }
        }

        ensure_route_has_responses(&method, &path, &route.doc.responses)?;
        ensure_unique_response_statuses(&method, &path, &route.doc.responses)?;
        let mut responses = json!({});
        for (status, schema_ref) in &route.doc.responses {
            let response_schema_name =
                crate::http::route_manifest_schema_name(schema_ref).map_err(|error| {
                    Error::message(format!(
                        "OpenAPI route `{method} {path}` response status `{status}` has invalid schema metadata: {error}"
                    ))
                })?;
            if let Some(media_type) =
                crate::http::route_response_media_type(*status, &response_schema_name)
            {
                let schema =
                    route_media_schema(schema_ref.name, (schema_ref.schema_fn)(), &mut schemas)?;
                let mut content = serde_json::Map::new();
                content.insert(media_type.as_str().to_string(), json!({ "schema": schema }));
                responses[status.to_string()] = json!({
                    "description": "",
                    "x-foundry-response-has-body": true,
                    "x-foundry-response-media-type": media_type.as_str(),
                    "content": content
                });
            } else {
                responses[status.to_string()] = json!({
                    "description": "",
                    "x-foundry-response-has-body": false
                });
            }
        }
        operation["responses"] = responses;

        let path_entry = paths.entry(path).or_insert_with(|| json!({}));
        path_entry[&method] = operation;
    }

    let mut spec = json!({
        "openapi": "3.1.0",
        "info": { "title": title, "version": version },
        "paths": paths,
        "components": { "schemas": schemas }
    });
    if let Some(validation_rule_manifest) =
        openapi_validation_rule_manifest(validation_rules, validation_rule_manifest_authoritative)?
    {
        spec["x-foundry-validation-rules"] = validation_rule_manifest;
    }
    Ok(spec)
}

fn openapi_validation_rule_manifest(
    validation_rules: &[ValidationRuleDescriptor],
    validation_rule_manifest_authoritative: bool,
) -> Result<Option<Value>> {
    if validation_rules.is_empty() && !validation_rule_manifest_authoritative {
        return Ok(None);
    }

    let mut entries = serde_json::Map::new();
    let mut validation_rule_ids = DottedIdTreeNode::default();
    for rule in validation_rules {
        let id = rule.id.as_str();
        if id.trim().is_empty() || id.trim() != id {
            return Err(Error::message(format!(
                "OpenAPI validation rule manifest contains invalid validation rule id `{id}`; validation rule ids must be non-empty and trimmed"
            )));
        }
        insert_dotted_id(
            &mut validation_rule_ids,
            id,
            "OpenAPI validation rule manifest",
            "validation rule id",
        )?;
        entries.insert(
            id.to_string(),
            json!({
                "id": id,
                "serverOnly": true
            }),
        );
    }

    Ok(Some(Value::Object(entries)))
}

fn supported_openapi_route_method(method: &str, path: &str) -> Result<String> {
    let method = method.trim().to_ascii_lowercase();
    if crate::http::route_http_method_is_supported(&method) {
        return Ok(method);
    }

    Err(Error::message(format!(
        "OpenAPI route `{method} {path}` uses unsupported HTTP method `{method}`; supported methods: {}",
        crate::http::route_http_methods_display()
    )))
}

fn ensure_openapi_route_path(path: &str) -> Result<()> {
    if path.trim().is_empty() || path.trim() != path || !path.starts_with('/') {
        return Err(Error::message(format!(
            "OpenAPI route path `{path}` is invalid; route paths must be non-empty, trimmed, and start with `/`"
        )));
    }

    crate::http::ensure_route_path_params_are_valid(path, &format!("OpenAPI route path `{path}`"))
}

fn ensure_route_document_metadata(doc: &RouteDoc, method: &str, path: &str) -> Result<()> {
    let context = format!("OpenAPI route `{method} {path}`");
    ensure_route_document_optional_text(&context, "operationId", doc.operation_id.as_deref())?;
    ensure_route_document_optional_text(&context, "route id", doc.route_id.as_deref())?;
    ensure_route_document_optional_text(&context, "summary", doc.summary.as_deref())?;
    ensure_route_document_optional_text(&context, "description", doc.description.as_deref())?;

    let mut tags = BTreeSet::new();
    for tag in &doc.tags {
        if tag.trim().is_empty() || tag.trim() != tag {
            return Err(Error::message(format!(
                "{context} contains invalid tag `{tag}`; tag metadata must be non-empty and trimmed"
            )));
        }
        if !tags.insert(tag.as_str()) {
            return Err(Error::message(format!(
                "{context} contains duplicate tag `{tag}`; tag metadata must be unique"
            )));
        }
    }

    Ok(())
}

fn ensure_route_document_optional_text(
    context: &str,
    field: &str,
    value: Option<&str>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };

    if !value.trim().is_empty() && value.trim() == value {
        return Ok(());
    }

    Err(Error::message(format!(
        "{context} contains invalid {field} `{value}`; document metadata must be non-empty and trimmed"
    )))
}

fn ensure_route_has_responses(
    method: &str,
    path: &str,
    responses: &[(u16, super::SchemaRef)],
) -> Result<()> {
    if responses.is_empty() {
        return Err(Error::message(format!(
            "OpenAPI route `{method} {path}` has no documented responses; add at least one response::<T>(status) or validation_errors()"
        )));
    }
    Ok(())
}

fn ensure_unique_response_statuses(
    method: &str,
    path: &str,
    responses: &[(u16, super::SchemaRef)],
) -> Result<()> {
    let mut statuses = BTreeSet::new();
    for (status, _) in responses {
        if !crate::http::route_response_status_is_valid(*status) {
            return Err(Error::message(format!(
                "OpenAPI route `{method} {path}` documents invalid response status `{status}`; response statuses must be in {}",
                crate::http::route_response_status_range_display()
            )));
        }
        if !statuses.insert(*status) {
            return Err(Error::message(format!(
                "OpenAPI route `{method} {path}` documents response status `{status}` multiple times; keep one response schema per status"
            )));
        }
    }
    Ok(())
}

fn route_auth_extension(doc: &RouteDoc, method: &str, path: &str) -> Result<Option<Value>> {
    if !doc.auth_required
        && doc.auth_guard.is_none()
        && doc.auth_permissions.is_empty()
        && !doc.auth_allows_mfa_pending_token
        && !doc.auth_has_authorize_callback
    {
        return Ok(None);
    }

    ensure_route_auth_metadata(doc, method, path)?;

    let mut auth = serde_json::Map::new();
    auth.insert("required".to_string(), json!(doc.auth_required));
    auth.insert(
        "allowsMfaPendingToken".to_string(),
        json!(doc.auth_allows_mfa_pending_token),
    );
    auth.insert(
        "hasAuthorizeCallback".to_string(),
        json!(doc.auth_has_authorize_callback),
    );
    if let Some(guard) = &doc.auth_guard {
        auth.insert("guard".to_string(), json!(guard));
    }
    if !doc.auth_permissions.is_empty() {
        auth.insert("permissions".to_string(), json!(doc.auth_permissions));
    }

    Ok(Some(Value::Object(auth)))
}

fn ensure_route_auth_metadata(doc: &RouteDoc, method: &str, path: &str) -> Result<()> {
    let context = route_auth_context(doc, method, path);
    if let Some(guard) = &doc.auth_guard {
        if guard.trim().is_empty() || guard.trim() != guard {
            return Err(Error::message(format!(
                "{context} contains invalid guard `{guard}`; guard metadata must be non-empty and trimmed"
            )));
        }
    }

    let mut permissions = BTreeSet::new();
    for permission in &doc.auth_permissions {
        if permission.trim().is_empty() || permission.trim() != permission {
            return Err(Error::message(format!(
                "{context} contains invalid permission `{permission}`; permission metadata must be non-empty and trimmed"
            )));
        }
        if !permissions.insert(permission.as_str()) {
            return Err(Error::message(format!(
                "{context} contains duplicate permission `{permission}`; permission metadata must be unique"
            )));
        }
    }

    if doc.auth_required {
        return Ok(());
    }

    if doc.auth_guard.is_none()
        && doc.auth_permissions.is_empty()
        && !doc.auth_has_authorize_callback
    {
        return Ok(());
    }

    Err(Error::message(format!(
        "{context} has guard, permission, or authorize callback metadata but auth is not required; guarded route metadata must set auth required to true"
    )))
}

fn route_auth_context(doc: &RouteDoc, method: &str, path: &str) -> String {
    if let Some(route_id) = &doc.route_id {
        format!("OpenAPI route auth for route `{route_id}`")
    } else {
        format!("OpenAPI route auth for `{method} {path}`")
    }
}

fn route_policy_extension(doc: &RouteDoc, method: &str, path: &str) -> Result<Option<Value>> {
    if doc.middleware_group.is_none() && doc.audit_area.is_none() && doc.rate_limits.is_empty() {
        return Ok(None);
    }

    ensure_route_policy_text_metadata(doc, method, path)?;

    let mut policy = serde_json::Map::new();
    if let Some(middleware_group) = &doc.middleware_group {
        policy.insert("middlewareGroup".to_string(), json!(middleware_group));
    }
    if let Some(audit_area) = &doc.audit_area {
        policy.insert("auditArea".to_string(), json!(audit_area));
    }
    if !doc.rate_limits.is_empty() {
        let mut rate_limits = Vec::new();
        for rate_limit in &doc.rate_limits {
            ensure_route_policy_rate_limit(doc, method, path, rate_limit)?;
            rate_limits.push(json!({
                    "maxRequests": rate_limit.max_requests,
                    "windowSeconds": rate_limit.window_seconds,
                    "by": rate_limit.by,
            }));
        }
        policy.insert("rateLimits".to_string(), json!(rate_limits));
    }

    Ok(Some(Value::Object(policy)))
}

fn ensure_route_policy_text_metadata(doc: &RouteDoc, method: &str, path: &str) -> Result<()> {
    let context = route_policy_context(doc, method, path);
    ensure_route_policy_optional_text(
        &context,
        "middlewareGroup",
        doc.middleware_group.as_deref(),
    )?;
    ensure_route_policy_optional_text(&context, "auditArea", doc.audit_area.as_deref())
}

fn ensure_route_policy_optional_text(
    context: &str,
    field: &str,
    value: Option<&str>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };

    if !value.trim().is_empty() && value.trim() == value {
        return Ok(());
    }

    Err(Error::message(format!(
        "{context} contains invalid {field} `{value}`; policy metadata must be non-empty and trimmed"
    )))
}

fn ensure_route_policy_rate_limit(
    doc: &RouteDoc,
    method: &str,
    path: &str,
    rate_limit: &super::RouteDocRateLimit,
) -> Result<()> {
    let context = route_policy_context(doc, method, path);
    let Some(rate_limit_by) = crate::http::middleware::RateLimitBy::from_name(&rate_limit.by)
    else {
        return Err(Error::message(format!(
            "{context} contains invalid rate-limit by `{}`; rate-limit by must be `ip`, `actor`, or `actor_or_ip`",
            rate_limit.by
        )));
    };
    if rate_limit.max_requests == 0 {
        return Err(Error::message(format!(
            "{context} contains rate-limit metadata with maxRequests `0`; rate-limit maxRequests must be greater than 0"
        )));
    }
    if rate_limit.window_seconds == 0 {
        return Err(Error::message(format!(
            "{context} contains rate-limit metadata with windowSeconds `0`; rate-limit windowSeconds must be greater than 0"
        )));
    }
    crate::support::javascript::ensure_safe_integer_u64(
        &format!("{context} rate-limit windowSeconds"),
        rate_limit.window_seconds,
        "JavaScript",
    )?;
    if !doc.auth_required && matches!(rate_limit_by, crate::http::middleware::RateLimitBy::Actor) {
        return Err(Error::message(format!(
            "{context} contains actor rate-limit metadata but auth is not required; actor rate limits only run after authentication"
        )));
    }

    Ok(())
}

fn route_policy_context(doc: &RouteDoc, method: &str, path: &str) -> String {
    if let Some(route_id) = &doc.route_id {
        format!("OpenAPI route policy for route `{route_id}`")
    } else {
        format!("OpenAPI route policy for `{method} {path}`")
    }
}

fn route_path_parameters(path: &str) -> Vec<Value> {
    crate::http::route_path_params(path)
        .into_iter()
        .map(|param| {
            let is_wildcard = crate::http::route_path_param_is_wildcard(path, &param);
            let mut parameter = json!({
                "name": param,
                "in": "path",
                "required": true,
                "schema": { "type": "string" }
            });
            if is_wildcard {
                parameter["description"] = json!("Catch-all path parameter.");
                parameter["x-foundry-catch-all"] = json!(true);
            }
            parameter
        })
        .collect()
}

fn append_operation_parameters(operation: &mut Value, parameters: Vec<Value>) {
    let existing = operation
        .get("parameters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    operation["parameters"] = json!(existing.into_iter().chain(parameters).collect::<Vec<_>>());
}

fn append_query_request_validation_metadata(operation: &mut Value, schema: &Value) {
    if let Some(validation) = schema.get("x-foundry-validation") {
        operation["x-foundry-validation"] = validation.clone();
    }
    if let Some(field_value_kinds) = schema.get("x-foundry-validation-field-value-kinds") {
        operation["x-foundry-validation-field-value-kinds"] = field_value_kinds.clone();
    }
}

fn route_query_parameters(schema: &Value) -> Vec<Value> {
    let Some(object) = schema.as_object() else {
        return Vec::new();
    };
    let Some(properties) = object.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required = object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<std::collections::HashSet<_>>();

    properties
        .iter()
        .map(|(name, property_schema)| {
            let mut parameter = json!({
                "name": name,
                "in": "query",
                "required": required.contains(name.as_str()),
                "schema": property_schema
            });
            if route_query_schema_is_array(property_schema) {
                parameter["style"] = json!("form");
                parameter["explode"] = json!(true);
            }
            parameter
        })
        .collect()
}

fn route_query_schema_is_array(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("array")
}

fn openapi_path_template(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if let Some(param) = segment
                .strip_prefix("{*")
                .and_then(|inner| inner.strip_suffix('}'))
            {
                format!("{{{param}}}")
            } else if let Some(param) = segment.strip_prefix(':') {
                format!("{{{param}}}")
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn route_media_schema(
    name: &str,
    schema: Value,
    components: &mut BTreeMap<String, Value>,
) -> Result<Value> {
    if route_schema_should_inline(name, &schema) {
        Ok(schema)
    } else {
        insert_component_schema(name, schema, components)?;
        Ok(json!({ "$ref": format!("#/components/schemas/{name}") }))
    }
}

fn insert_component_schema(
    name: &str,
    schema: Value,
    components: &mut BTreeMap<String, Value>,
) -> Result<()> {
    ensure_valid_component_schema_name(name)?;

    if let Some(existing) = components.get(name) {
        if existing != &schema {
            return Err(Error::message(format!(
                "OpenAPI component schema `{name}` is generated with conflicting shapes; use unique ApiSchema::schema_name() values or inline one of the schemas"
            )));
        }
        return Ok(());
    }

    components.insert(name.to_string(), schema);
    Ok(())
}

fn ensure_valid_component_schema_name(name: &str) -> Result<()> {
    if is_valid_component_schema_name(name) {
        return Ok(());
    }

    Err(Error::message(format!(
        "OpenAPI component schema name `{name}` is invalid; component names must be non-empty and contain only ASCII letters, digits, dots, hyphens, or underscores"
    )))
}

pub(crate) fn is_valid_component_schema_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
}

fn route_request_media_schema(
    name: &str,
    schema: Value,
    media_type: crate::http::RouteRequestMediaType,
    components: &mut BTreeMap<String, Value>,
) -> Result<Value> {
    if media_type == crate::http::RouteRequestMediaType::Multipart
        && route_schema_is_direct_file_root(name, &schema)
    {
        let required = !route_schema_is_nullable(&schema);
        let mut multipart_schema = json!({
            "type": "object",
            "properties": {
                "file": schema,
            }
        });
        if required {
            multipart_schema["required"] = json!(["file"]);
        }
        return Ok(multipart_schema);
    }

    route_media_schema(name, schema, components)
}

fn route_schema_is_direct_file_root(name: &str, schema: &Value) -> bool {
    if name == "UploadedFile" {
        return route_schema_is_binary_file(schema);
    }

    name == "Array"
        && schema.get("type").and_then(Value::as_str) == Some("array")
        && schema.get("items").is_some_and(route_schema_is_binary_file)
}

fn route_schema_is_binary_file(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("string")
        && schema.get("format").and_then(Value::as_str) == Some("binary")
}

fn route_schema_is_nullable(schema: &Value) -> bool {
    schema
        .get("nullable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn route_schema_should_inline(name: &str, schema: &Value) -> bool {
    name == "Array"
        || name == "Map"
        || schema.get("x-foundry-wrapper-schema").is_some()
        || schema.get("x-foundry-item-schema").is_some()
        || schema.get("x-foundry-additional-schema").is_some()
        || schema
            .get("nullable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn route_schema_with_registered_validation(
    name: &str,
    mut schema: Value,
    validation_rules: &[ValidationRuleDescriptor],
    validation_rule_manifest_authoritative: bool,
) -> Result<Value> {
    apply_registered_validation_schema(
        name,
        &mut schema,
        validation_rules,
        validation_rule_manifest_authoritative,
    )?;
    apply_registered_validation_child_schemas(
        &mut schema,
        validation_rules,
        validation_rule_manifest_authoritative,
    )?;
    Ok(schema)
}

fn apply_registered_validation_child_schemas(
    schema: &mut Value,
    validation_rules: &[ValidationRuleDescriptor],
    validation_rule_manifest_authoritative: bool,
) -> Result<()> {
    if let Some(obj) = schema.as_object_mut() {
        if let Some(name) = obj
            .get("x-foundry-item-schema")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            if let Some(items) = obj.get_mut("items") {
                apply_registered_validation_schema(
                    &name,
                    items,
                    validation_rules,
                    validation_rule_manifest_authoritative,
                )?;
            }
        }

        if let Some(name) = obj
            .get("x-foundry-additional-schema")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            if let Some(additional) = obj.get_mut("additionalProperties") {
                apply_registered_validation_schema(
                    &name,
                    additional,
                    validation_rules,
                    validation_rule_manifest_authoritative,
                )?;
            }
        }
    }

    match schema {
        Value::Array(values) => {
            for value in values {
                apply_registered_validation_child_schemas(
                    value,
                    validation_rules,
                    validation_rule_manifest_authoritative,
                )?;
            }
        }
        Value::Object(obj) => {
            for value in obj.values_mut() {
                apply_registered_validation_child_schemas(
                    value,
                    validation_rules,
                    validation_rule_manifest_authoritative,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn apply_registered_validation_schema(
    name: &str,
    schema: &mut Value,
    validation_rules: &[ValidationRuleDescriptor],
    validation_rule_manifest_authoritative: bool,
) -> Result<()> {
    let Some(validation) = crate::typescript::registered_validation_schema(name)? else {
        return Ok(());
    };
    crate::typescript::ensure_validation_schema_references_registered_rules(
        name,
        &validation,
        validation_rules,
        validation_rule_manifest_authoritative,
    )?;
    let Some(obj) = schema.as_object_mut() else {
        return Ok(());
    };

    apply_openapi_validation_schema_constraints(obj, &validation);
    Ok(())
}

fn apply_openapi_validation_schema_constraints(
    obj: &mut serde_json::Map<String, Value>,
    schema: &crate::typescript::TsValidationSchema,
) {
    apply_openapi_strict_object_constraints(obj, schema);
    apply_openapi_validation_field_value_kind_metadata(obj, schema);
    append_openapi_validation_rules(obj, &schema.rules);
    apply_openapi_validation_field_constraints(obj, &schema.fields);
}

fn apply_openapi_validation_field_value_kind_metadata(
    obj: &mut serde_json::Map<String, Value>,
    schema: &crate::typescript::TsValidationSchema,
) {
    if schema.field_value_kinds.is_empty() {
        return;
    }

    obj.insert(
        "x-foundry-validation-field-value-kinds".into(),
        Value::Array(
            schema
                .field_value_kinds
                .iter()
                .map(|entry| {
                    json!({
                        "field": entry.field.as_str(),
                        "kind": entry.kind.as_str(),
                    })
                })
                .collect(),
        ),
    );
}

fn apply_openapi_strict_object_constraints(
    obj: &mut serde_json::Map<String, Value>,
    schema: &crate::typescript::TsValidationSchema,
) {
    if !schema.deny_unknown_fields || openapi_schema_type(obj) != Some("object") {
        return;
    }

    obj.insert("additionalProperties".into(), json!(false));

    let allowed_fields = if schema.known_fields.is_empty() {
        schema
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>()
    } else {
        schema
            .known_fields
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    };
    if allowed_fields.is_empty() {
        return;
    }

    let properties = obj
        .entry("properties".to_string())
        .or_insert_with(|| json!({}));
    let Some(properties) = properties.as_object_mut() else {
        return;
    };
    for field in allowed_fields {
        properties
            .entry(field.to_string())
            .or_insert_with(|| json!({}));
    }
}

fn apply_openapi_validation_field_constraints(
    obj: &mut serde_json::Map<String, Value>,
    fields: &[crate::typescript::TsValidationField],
) {
    for field in fields {
        if field.rules.iter().any(openapi_validation_rule_is_required) {
            super::insert_json_schema_required_properties(obj, [field.name.clone()]);
        }
    }

    let Some(properties) = obj.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    for field in fields {
        if let Some(property) = properties
            .get_mut(&field.name)
            .and_then(Value::as_object_mut)
        {
            append_openapi_validation_rules(property, &field.rules);
        }
    }
}

fn openapi_validation_rule_is_required(rule: &crate::typescript::TsValidationRule) -> bool {
    !rule.server_only && rule.code == "required"
}

fn append_openapi_validation_rules(
    obj: &mut serde_json::Map<String, Value>,
    rules: &[crate::typescript::TsValidationRule],
) {
    for rule in rules {
        apply_openapi_validation_rule_constraints(obj, rule);
        super::insert_foundry_validation_rule(obj, openapi_validation_rule(rule));
    }
}

fn apply_openapi_validation_rule_constraints(
    obj: &mut serde_json::Map<String, Value>,
    rule: &crate::typescript::TsValidationRule,
) {
    if rule.server_only {
        apply_openapi_server_only_validation_rule_constraints(obj, rule);
        return;
    }

    match rule.code.as_str() {
        "required_keys" => {
            super::insert_json_schema_required_properties(obj, rule.values.iter().cloned());
        }
        "filled" => match openapi_schema_type(obj) {
            Some("array") => {
                let min_items = obj
                    .get("minItems")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .max(1);
                obj.insert("minItems".into(), json!(min_items));
            }
            Some("string") => super::insert_json_schema_pattern(obj, r"\S"),
            _ => {}
        },
        "email" => insert_openapi_format(obj, "email"),
        "url" => insert_openapi_format(obj, "uri"),
        "uuid" => {
            insert_openapi_format(obj, "uuid");
            if let Some(version) = openapi_rule_u8_param(rule, "version") {
                insert_openapi_uuid_version_pattern(obj, version);
            }
        }
        "ulid" => {
            super::insert_json_schema_pattern(obj, "^[0-7][0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{25}$")
        }
        "hex_color" => super::insert_json_schema_pattern(
            obj,
            "^#(?:[0-9A-Fa-f]{3}|[0-9A-Fa-f]{4}|[0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$",
        ),
        "mac_address" => super::insert_json_schema_pattern(
            obj,
            "^(?:(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}|(?:[0-9A-Fa-f]{2}-){5}[0-9A-Fa-f]{2})$",
        ),
        "numeric" if openapi_schema_type(obj) == Some("string") => {
            super::insert_json_schema_pattern(
                obj,
                r"^[+-]?(?:(?:\d+(?:\.\d*)?)|(?:\.\d+))(?:[eE][+-]?\d+)?$",
            );
        }
        "integer" => {
            match openapi_schema_type(obj) {
                Some("string") => super::insert_json_schema_pattern(obj, r"^[+-]?\d+$"),
                Some("number") => {
                    obj.insert("multipleOf".into(), json!(1));
                }
                _ => {}
            }
            obj.insert("x-foundry-integer-format".into(), json!("i64"));
        }
        "boolean" if openapi_schema_type(obj) != Some("boolean") => {
            obj.insert("enum".into(), json!(["true", "false", "1", "0"]));
        }
        "accepted" => {
            if openapi_schema_type(obj) == Some("boolean") {
                obj.insert("enum".into(), json!([true]));
            } else {
                obj.insert("enum".into(), json!(["yes", "on", "1", "true"]));
            }
        }
        "declined" => {
            if openapi_schema_type(obj) == Some("boolean") {
                obj.insert("enum".into(), json!([false]));
            } else {
                obj.insert("enum".into(), json!(["no", "off", "0", "false"]));
            }
        }
        "alpha" => super::insert_json_schema_pattern(obj, r"^[\p{L}\p{M}]*$"),
        "alpha_dash" => super::insert_json_schema_pattern(obj, r"^[\p{L}\p{M}\p{N}_-]*$"),
        "alpha_num" | "alpha_numeric" => {
            super::insert_json_schema_pattern(obj, r"^[\p{L}\p{M}\p{N}]*$");
        }
        "ascii" => super::insert_json_schema_pattern(obj, r"^[\x00-\x7F]*$"),
        "lowercase" => super::insert_json_schema_pattern(obj, r"^[^\p{Lu}]*$"),
        "uppercase" => super::insert_json_schema_pattern(obj, r"^[^\p{Ll}]*$"),
        "regex" => {
            if let Some(pattern) = rule.params.get("pattern") {
                super::insert_json_schema_pattern(obj, pattern.clone());
            }
        }
        "not_regex" => {
            if let Some(pattern) = rule.params.get("pattern") {
                super::insert_json_schema_not_any_pattern(obj, [pattern.clone()]);
            }
        }
        "starts_with" => insert_openapi_string_value_patterns(
            obj,
            openapi_rule_values_or_value_param(rule),
            |value| format!("^{}", super::escape_json_schema_pattern_literal(&value)),
            false,
        ),
        "doesnt_start_with" => insert_openapi_string_value_patterns(
            obj,
            openapi_rule_values_or_value_param(rule),
            |value| format!("^{}", super::escape_json_schema_pattern_literal(&value)),
            true,
        ),
        "ends_with" => insert_openapi_string_value_patterns(
            obj,
            openapi_rule_values_or_value_param(rule),
            |value| format!("{}$", super::escape_json_schema_pattern_literal(&value)),
            false,
        ),
        "doesnt_end_with" => insert_openapi_string_value_patterns(
            obj,
            openapi_rule_values_or_value_param(rule),
            |value| format!("{}$", super::escape_json_schema_pattern_literal(&value)),
            true,
        ),
        "contains" => {
            let values = openapi_rule_values_or_value_param(rule);
            if openapi_schema_type(obj) == Some("array") {
                super::insert_json_schema_array_contains_all(obj, values);
            } else {
                insert_openapi_string_value_patterns(
                    obj,
                    values,
                    |value| super::escape_json_schema_pattern_literal(&value),
                    false,
                );
            }
        }
        "doesnt_contain" => {
            let values = openapi_rule_values_or_value_param(rule);
            if openapi_schema_type(obj) == Some("array") {
                super::insert_json_schema_array_not_contains_any(obj, values);
            } else {
                insert_openapi_string_value_patterns(
                    obj,
                    values,
                    |value| super::escape_json_schema_pattern_literal(&value),
                    true,
                );
            }
        }
        "digits" => super::insert_json_schema_pattern(obj, "^[0-9]*$"),
        "min_digits" => {
            super::insert_json_schema_pattern(obj, "^[0-9]*$");
            insert_openapi_usize_param(obj, "minLength", rule, "min");
        }
        "max_digits" => {
            super::insert_json_schema_pattern(obj, "^[0-9]*$");
            insert_openapi_usize_param(obj, "maxLength", rule, "max");
        }
        "digits_between" => {
            super::insert_json_schema_pattern(obj, "^[0-9]*$");
            insert_openapi_usize_param(obj, "minLength", rule, "min");
            insert_openapi_usize_param(obj, "maxLength", rule, "max");
        }
        "date" => insert_openapi_format(obj, "date"),
        "time" => insert_openapi_format(obj, "time"),
        "datetime" | "local_datetime" => insert_openapi_format(obj, "date-time"),
        "timezone" => insert_openapi_format(obj, "timezone"),
        "ip" => insert_openapi_format(obj, "ip"),
        "ipv4" => insert_openapi_format(obj, "ipv4"),
        "ipv6" => insert_openapi_format(obj, "ipv6"),
        "json" if openapi_schema_type(obj) == Some("string") => {
            insert_openapi_format(obj, "json-string");
        }
        "min" => insert_openapi_usize_param(obj, "minLength", rule, "min"),
        "max" => insert_openapi_usize_param(obj, "maxLength", rule, "max"),
        "size" => apply_openapi_size_rule(obj, rule),
        "size_items" => apply_openapi_exact_items_rule(obj, rule, "size"),
        "min_items" => insert_openapi_usize_param(obj, "minItems", rule, "min"),
        "max_items" => insert_openapi_usize_param(obj, "maxItems", rule, "max"),
        "distinct" => {
            obj.insert("uniqueItems".into(), json!(true));
        }
        "decimal" => {
            if let (Some(min), Some(max)) = (
                openapi_rule_usize_param(rule, "min"),
                openapi_rule_usize_param(rule, "max"),
            ) {
                super::insert_json_schema_pattern(obj, openapi_decimal_pattern(min, max));
            }
        }
        "min_numeric" => insert_openapi_f64_param(obj, "minimum", rule, "min"),
        "max_numeric" => insert_openapi_f64_param(obj, "maximum", rule, "max"),
        "multiple_of" => {
            if let Some(value) = openapi_rule_f64_param(rule, "value") {
                if value > 0.0 {
                    obj.insert("multipleOf".into(), json!(value));
                }
            }
        }
        "between" => {
            insert_openapi_f64_param(obj, "minimum", rule, "min");
            insert_openapi_f64_param(obj, "maximum", rule, "max");
        }
        "gt" => insert_openapi_f64_param(obj, "exclusiveMinimum", rule, "value"),
        "gte" => insert_openapi_f64_param(obj, "minimum", rule, "value"),
        "lt" => insert_openapi_f64_param(obj, "exclusiveMaximum", rule, "value"),
        "lte" => insert_openapi_f64_param(obj, "maximum", rule, "value"),
        "max_file_size" | "allowed_extensions" => {
            apply_openapi_file_validation_rule_constraints(obj, rule);
        }
        "in_list" => {
            obj.insert(
                "enum".into(),
                super::json_schema_enum_values_for_schema(obj, &rule.values),
            );
        }
        "not_in" => {
            let values = super::json_schema_enum_values_for_schema(obj, &rule.values);
            obj.insert("not".into(), json!({ "enum": values }));
        }
        "app_enum" if !obj.contains_key("enum") => {
            obj.insert(
                "enum".into(),
                super::json_schema_enum_values_for_schema(obj, &rule.values),
            );
        }
        "nested" => apply_openapi_nested_rule_constraints(obj, rule),
        "each" => apply_openapi_each_rule_constraints(obj, rule),
        _ => {}
    }
}

fn apply_openapi_server_only_validation_rule_constraints(
    obj: &mut serde_json::Map<String, Value>,
    rule: &crate::typescript::TsValidationRule,
) {
    match rule.code.as_str() {
        "image" | "allowed_mimes" | "max_dimensions" | "min_dimensions" => {
            apply_openapi_file_validation_rule_constraints(obj, rule);
        }
        _ => {}
    }
}

fn apply_openapi_file_validation_rule_constraints(
    obj: &mut serde_json::Map<String, Value>,
    rule: &crate::typescript::TsValidationRule,
) {
    let Some(target) = openapi_file_validation_target_mut(obj) else {
        return;
    };

    match rule.code.as_str() {
        "max_file_size" => {
            if let Some(max) = openapi_rule_usize_param(rule, "max") {
                target.insert("x-foundry-max-file-size-kb".into(), json!(max));
            }
        }
        "allowed_extensions" => {
            target.insert("x-foundry-allowed-extensions".into(), json!(rule.values));
        }
        "image" => super::insert_foundry_server_only_validation(target, "image"),
        "allowed_mimes" => {
            target.insert("x-foundry-allowed-mimes".into(), json!(rule.values));
            super::insert_foundry_server_only_validation(target, "allowed_mimes");
        }
        "max_dimensions" => {
            if let (Some(width), Some(height)) = (
                openapi_rule_u32_param(rule, "width"),
                openapi_rule_u32_param(rule, "height"),
            ) {
                target.insert(
                    "x-foundry-max-dimensions".into(),
                    json!({ "width": width, "height": height }),
                );
                super::insert_foundry_server_only_validation(target, "max_dimensions");
            }
        }
        "min_dimensions" => {
            if let (Some(width), Some(height)) = (
                openapi_rule_u32_param(rule, "width"),
                openapi_rule_u32_param(rule, "height"),
            ) {
                target.insert(
                    "x-foundry-min-dimensions".into(),
                    json!({ "width": width, "height": height }),
                );
                super::insert_foundry_server_only_validation(target, "min_dimensions");
            }
        }
        _ => {}
    }
}

fn apply_openapi_each_rule_constraints(
    obj: &mut serde_json::Map<String, Value>,
    rule: &crate::typescript::TsValidationRule,
) {
    if openapi_schema_type(obj) != Some("array") {
        return;
    }

    let Some(items) = obj.get_mut("items").and_then(Value::as_object_mut) else {
        return;
    };

    append_openapi_validation_rules(items, &rule.rules);
}

fn apply_openapi_nested_rule_constraints(
    obj: &mut serde_json::Map<String, Value>,
    rule: &crate::typescript::TsValidationRule,
) {
    let Some(schema) = &rule.schema else {
        return;
    };

    apply_openapi_validation_schema_constraints(obj, schema);
}

fn apply_openapi_size_rule(
    obj: &mut serde_json::Map<String, Value>,
    rule: &crate::typescript::TsValidationRule,
) {
    if rule.params.get("kind").map(String::as_str) == Some("array") {
        apply_openapi_exact_items_rule(obj, rule, "size");
        return;
    }

    match openapi_schema_type(obj) {
        Some("array") => apply_openapi_exact_items_rule(obj, rule, "size"),
        Some("integer") | Some("number") => {
            if let Some(size) = openapi_rule_f64_param(rule, "size") {
                obj.insert("minimum".into(), json!(size));
                obj.insert("maximum".into(), json!(size));
            }
        }
        Some("string") => {
            if let Some(size) = openapi_rule_usize_param(rule, "size") {
                obj.insert("minLength".into(), json!(size));
                obj.insert("maxLength".into(), json!(size));
            }
        }
        _ => {}
    }
}

fn apply_openapi_exact_items_rule(
    obj: &mut serde_json::Map<String, Value>,
    rule: &crate::typescript::TsValidationRule,
    param: &str,
) {
    if let Some(size) = openapi_rule_usize_param(rule, param) {
        apply_openapi_exact_items_value(obj, size);
    }
}

fn apply_openapi_exact_items_value(obj: &mut serde_json::Map<String, Value>, size: usize) {
    obj.insert("minItems".into(), json!(size));
    obj.insert("maxItems".into(), json!(size));
}

fn insert_openapi_string_value_patterns(
    obj: &mut serde_json::Map<String, Value>,
    values: Vec<String>,
    pattern: impl Fn(String) -> String,
    negate: bool,
) {
    let patterns = values.into_iter().map(pattern).collect::<Vec<_>>();
    if negate {
        super::insert_json_schema_not_any_pattern(obj, patterns);
    } else {
        super::insert_json_schema_any_pattern(obj, patterns);
    }
}

fn insert_openapi_format(obj: &mut serde_json::Map<String, Value>, format: &str) {
    obj.insert("format".into(), json!(format));
}

fn insert_openapi_usize_param(
    obj: &mut serde_json::Map<String, Value>,
    schema_key: &str,
    rule: &crate::typescript::TsValidationRule,
    param: &str,
) {
    if let Some(value) = openapi_rule_usize_param(rule, param) {
        obj.insert(schema_key.into(), json!(value));
    }
}

fn insert_openapi_f64_param(
    obj: &mut serde_json::Map<String, Value>,
    schema_key: &str,
    rule: &crate::typescript::TsValidationRule,
    param: &str,
) {
    if let Some(value) = openapi_rule_f64_param(rule, param) {
        obj.insert(schema_key.into(), json!(value));
    }
}

fn insert_openapi_uuid_version_pattern(obj: &mut serde_json::Map<String, Value>, version: u8) {
    if !(1..=8).contains(&version) {
        return;
    }

    let version = format!("{version:x}");
    let canonical = format!(
        "[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-{version}[0-9a-fA-F]{{3}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}",
    );
    let compact = format!("[0-9a-fA-F]{{12}}{version}[0-9a-fA-F]{{19}}");
    super::insert_json_schema_pattern(
        obj,
        format!("^(?:{compact}|{canonical}|\\{{{canonical}\\}}|urn:uuid:{canonical})$"),
    );
}

fn openapi_decimal_pattern(min: usize, max: usize) -> String {
    if min > max {
        return "a^".to_string();
    }
    if min == max {
        if min == 0 {
            return "^[+-]?[0-9]+\\.$".to_string();
        }
        return format!("^[+-]?(?:[0-9]+\\.[0-9]{{{min}}}|\\.[0-9]{{{min}}})$");
    }
    if min == 0 {
        return format!("^[+-]?(?:[0-9]+\\.[0-9]{{0,{max}}}|\\.[0-9]{{1,{max}}})$");
    }
    format!("^[+-]?(?:[0-9]+\\.[0-9]{{{min},{max}}}|\\.[0-9]{{{min},{max}}})$")
}

fn openapi_rule_values_or_value_param(rule: &crate::typescript::TsValidationRule) -> Vec<String> {
    if !rule.values.is_empty() {
        return rule.values.clone();
    }
    rule.params
        .get("value")
        .map(|value| vec![value.clone()])
        .unwrap_or_default()
}

fn openapi_rule_usize_param(
    rule: &crate::typescript::TsValidationRule,
    param: &str,
) -> Option<usize> {
    rule.params.get(param)?.parse().ok()
}

fn openapi_rule_u32_param(rule: &crate::typescript::TsValidationRule, param: &str) -> Option<u32> {
    rule.params.get(param)?.parse().ok()
}

fn openapi_rule_u8_param(rule: &crate::typescript::TsValidationRule, param: &str) -> Option<u8> {
    rule.params.get(param)?.parse().ok()
}

fn openapi_rule_f64_param(rule: &crate::typescript::TsValidationRule, param: &str) -> Option<f64> {
    let value = rule.params.get(param)?.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn openapi_schema_type(obj: &serde_json::Map<String, Value>) -> Option<&str> {
    obj.get("type").and_then(Value::as_str)
}

fn openapi_file_validation_target_mut(
    obj: &mut serde_json::Map<String, Value>,
) -> Option<&mut serde_json::Map<String, Value>> {
    if openapi_schema_is_file_upload(obj) {
        return Some(obj);
    }

    let items = obj.get_mut("items")?.as_object_mut()?;
    openapi_schema_is_file_upload(items).then_some(items)
}

fn openapi_schema_is_file_upload(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("type").and_then(Value::as_str) == Some("string")
        && obj.get("format").and_then(Value::as_str) == Some("binary")
}

fn openapi_validation_rule(rule: &crate::typescript::TsValidationRule) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("code".to_string(), json!(&rule.code));
    if !rule.params.is_empty() {
        obj.insert("params".to_string(), json!(&rule.params));
    }
    if !rule.values.is_empty() {
        obj.insert("values".to_string(), json!(&rule.values));
    }
    if rule.server_only {
        obj.insert("serverOnly".to_string(), json!(true));
    }
    if !rule.rules.is_empty() {
        obj.insert(
            "rules".to_string(),
            Value::Array(rule.rules.iter().map(openapi_validation_rule).collect()),
        );
    }
    if let Some(schema) = &rule.schema {
        obj.insert(
            "schema".to_string(),
            openapi_validation_schema_metadata(schema),
        );
    }
    Value::Object(obj)
}

fn openapi_validation_schema_metadata(schema: &crate::typescript::TsValidationSchema) -> Value {
    let mut obj = serde_json::Map::new();
    if schema.deny_unknown_fields {
        obj.insert("denyUnknownFields".to_string(), json!(true));
    }
    if !schema.known_fields.is_empty() {
        obj.insert("knownFields".to_string(), json!(&schema.known_fields));
    }
    if !schema.rules.is_empty() {
        obj.insert(
            "rules".to_string(),
            Value::Array(schema.rules.iter().map(openapi_validation_rule).collect()),
        );
    }
    if !schema.fields.is_empty() {
        obj.insert(
            "fields".to_string(),
            Value::Array(
                schema
                    .fields
                    .iter()
                    .map(|field| {
                        json!({
                            "name": &field.name,
                            "rules": field.rules.iter().map(openapi_validation_rule).collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            ),
        );
    }
    if !schema.messages.is_empty() {
        obj.insert(
            "messages".to_string(),
            Value::Array(
                schema
                    .messages
                    .iter()
                    .map(|message| {
                        json!({
                            "field": &message.field,
                            "rule": &message.rule,
                            "message": &message.message,
                        })
                    })
                    .collect(),
            ),
        );
    }
    if !schema.attributes.is_empty() {
        obj.insert(
            "attributes".to_string(),
            Value::Array(
                schema
                    .attributes
                    .iter()
                    .map(|attribute| {
                        json!({
                            "field": &attribute.field,
                            "name": &attribute.name,
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openapi::RouteDoc;

    struct ConflictingComponentA;
    struct ConflictingComponentB;
    struct InvalidComponentName;
    struct ValidPunctuatedComponentName;

    const OPENAPI_CUSTOM_RULE_ID: crate::support::ValidationRuleId =
        crate::support::ValidationRuleId::new("openapi.mobile");

    #[allow(dead_code)]
    #[derive(serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    #[serde(deny_unknown_fields)]
    struct StrictOpenApiRequest {
        name: String,
    }

    #[allow(dead_code)]
    #[derive(
        serde::Deserialize, serde::Serialize, ts_rs::TS, crate::ApiSchema, crate::Validate,
    )]
    struct OpenApiCustomRuleRequest {
        #[validate(rule(OPENAPI_CUSTOM_RULE_ID))]
        phone: String,
    }

    struct OpenApiMismatchedCustomRuleRequest;

    impl crate::openapi::ApiSchema for OpenApiMismatchedCustomRuleRequest {
        fn schema() -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "phone": { "type": "string" }
                }
            })
        }

        fn schema_name() -> &'static str {
            "OpenApiMismatchedCustomRuleRequest"
        }
    }

    impl crate::typescript::TsValidationSchemaProvider for OpenApiMismatchedCustomRuleRequest {
        fn ts_validation_schema() -> crate::typescript::TsValidationSchema {
            crate::typescript::TsValidationSchema::new().field(
                "phone",
                [crate::typescript::TsValidationRule::new("openapi.mobile")
                    .param("rule", "openapi.sms")
                    .server_only()],
            )
        }
    }

    impl crate::openapi::ApiSchema for ConflictingComponentA {
        fn schema() -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "alpha": { "type": "string" }
                }
            })
        }

        fn schema_name() -> &'static str {
            "ConflictingComponent"
        }
    }

    impl crate::openapi::ApiSchema for ConflictingComponentB {
        fn schema() -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "beta": { "type": "integer" }
                }
            })
        }

        fn schema_name() -> &'static str {
            "ConflictingComponent"
        }
    }

    impl crate::openapi::ApiSchema for InvalidComponentName {
        fn schema() -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        fn schema_name() -> &'static str {
            "Invalid/Component"
        }
    }

    impl crate::openapi::ApiSchema for ValidPunctuatedComponentName {
        fn schema() -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        fn schema_name() -> &'static str {
            "Valid.Component_Name-1"
        }
    }

    fn manual_array_schema_without_item_marker() -> Value {
        serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        })
    }

    #[test]
    fn try_generate_openapi_spec_matches_infallible_wrapper() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/health".to_string(),
            doc: RouteDoc::new().get().response::<String>(200),
        }];

        let fallible = try_generate_openapi_spec("Foundry", "1.0.0", &routes).unwrap();
        let infallible = generate_openapi_spec("Foundry", "1.0.0", &routes);

        assert_eq!(fallible, infallible);
    }

    #[test]
    fn app_backed_openapi_spec_allows_registered_custom_validation_rules() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/sms".to_string(),
            doc: RouteDoc::new()
                .post()
                .request::<OpenApiCustomRuleRequest>()
                .response::<()>(202),
        }];
        let validation_rules = [ValidationRuleDescriptor {
            id: OPENAPI_CUSTOM_RULE_ID,
        }];

        let spec = try_generate_openapi_spec_with_validation_rules(
            "Foundry",
            "1.0.0",
            &routes,
            &validation_rules,
            true,
        )
        .expect("registered custom rule metadata should generate OpenAPI");

        assert_eq!(
            spec["components"]["schemas"]["OpenApiCustomRuleRequest"]["properties"]["phone"]
                ["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "openapi.mobile",
                    "params": { "rule": "openapi.mobile" },
                    "serverOnly": true,
                },
            ])
        );
        assert_eq!(
            spec["x-foundry-validation-rules"],
            serde_json::json!({
                "openapi.mobile": {
                    "id": "openapi.mobile",
                    "serverOnly": true
                }
            })
        );
    }

    #[test]
    fn app_backed_openapi_spec_rejects_duplicate_validation_rule_manifest_ids() {
        let validation_rules = [
            ValidationRuleDescriptor {
                id: crate::support::ValidationRuleId::new("tenant.mobile"),
            },
            ValidationRuleDescriptor {
                id: crate::support::ValidationRuleId::new("tenant.mobile"),
            },
        ];

        let error = try_generate_openapi_spec_with_validation_rules(
            "Foundry",
            "1.0.0",
            &[],
            &validation_rules,
            true,
        )
        .expect_err("duplicate validation rule manifest ids should fail");

        assert!(
            error.to_string().contains(
                "OpenAPI validation rule manifest contains duplicate validation rule id `tenant.mobile`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn app_backed_openapi_spec_rejects_validation_rule_manifest_id_grouping_collisions() {
        let validation_rules = [
            ValidationRuleDescriptor {
                id: crate::support::ValidationRuleId::new("tenant.mobile"),
            },
            ValidationRuleDescriptor {
                id: crate::support::ValidationRuleId::new("tenant.mobile.unique"),
            },
        ];

        let error = try_generate_openapi_spec_with_validation_rules(
            "Foundry",
            "1.0.0",
            &[],
            &validation_rules,
            true,
        )
        .expect_err("validation rule manifest id grouping collisions should fail");

        assert!(
            error.to_string().contains(
                "OpenAPI validation rule manifest cannot group validation rule id `tenant.mobile.unique` because `tenant.mobile` is already a validation rule id"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn app_backed_openapi_spec_rejects_empty_validation_rule_manifest_id_segments() {
        let validation_rules = [ValidationRuleDescriptor {
            id: crate::support::ValidationRuleId::new("tenant..mobile"),
        }];

        let error = try_generate_openapi_spec_with_validation_rules(
            "Foundry",
            "1.0.0",
            &[],
            &validation_rules,
            true,
        )
        .expect_err("validation rule manifest id segments should be non-empty");

        assert!(
            error.to_string().contains(
                "OpenAPI validation rule manifest requires non-empty validation rule id segments"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn app_backed_openapi_spec_rejects_camel_case_validation_rule_manifest_id_collisions() {
        let validation_rules = [
            ValidationRuleDescriptor {
                id: crate::support::ValidationRuleId::new("tenant.mobile_phone"),
            },
            ValidationRuleDescriptor {
                id: crate::support::ValidationRuleId::new("tenant.mobile-phone"),
            },
        ];

        let error = try_generate_openapi_spec_with_validation_rules(
            "Foundry",
            "1.0.0",
            &[],
            &validation_rules,
            true,
        )
        .expect_err("validation rule manifest id camelCase collisions should fail");

        assert!(
            error
                .to_string()
                .contains("both normalize to `mobilePhone`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn app_backed_openapi_spec_rejects_unregistered_custom_validation_rules() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/sms".to_string(),
            doc: RouteDoc::new()
                .post()
                .request::<OpenApiCustomRuleRequest>()
                .response::<()>(202),
        }];

        let error =
            try_generate_openapi_spec_with_validation_rules("Foundry", "1.0.0", &routes, &[], true)
                .expect_err("app-backed OpenAPI should reject stale custom validation metadata");

        assert!(
            error
                .to_string()
                .contains("unregistered validation rule `openapi.mobile`")
                && error.to_string().contains("OpenApiCustomRuleRequest"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn openapi_registered_validation_guard_rejects_mismatched_custom_validation_rules() {
        let validation_schema = <OpenApiMismatchedCustomRuleRequest as crate::typescript::TsValidationSchemaProvider>::ts_validation_schema();
        let validation_rules = [ValidationRuleDescriptor {
            id: crate::support::ValidationRuleId::new("openapi.sms"),
        }];

        let error = crate::typescript::ensure_validation_schema_references_registered_rules(
            "OpenApiMismatchedCustomRuleRequest",
            &validation_schema,
            &validation_rules,
            true,
        )
        .expect_err(
            "app-backed OpenAPI guard should reject inconsistent custom validation metadata",
        );

        assert!(
            error.to_string().contains(
                "custom field `phone` rule `openapi.mobile` with inconsistent param `rule` `openapi.sms`"
            ) && error
                .to_string()
                .contains("OpenApiMismatchedCustomRuleRequest"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn standalone_openapi_spec_allows_custom_validation_rules_without_registry() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/sms".to_string(),
            doc: RouteDoc::new()
                .post()
                .request::<OpenApiCustomRuleRequest>()
                .response::<()>(202),
        }];

        let spec = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect("standalone OpenAPI generation should not need a runtime rule registry");

        assert!(
            spec["components"]["schemas"]["OpenApiCustomRuleRequest"]["properties"]["phone"]
                .get("x-foundry-validation")
                .is_some(),
            "standalone OpenAPI should still publish custom validation metadata: {spec}"
        );
        assert!(
            spec.get("x-foundry-validation-rules").is_none(),
            "standalone OpenAPI should not publish an authoritative validation rule manifest: {spec}"
        );
    }

    #[test]
    fn app_backed_openapi_spec_publishes_empty_validation_rule_manifest() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/health".to_string(),
            doc: RouteDoc::new().get().response::<String>(200),
        }];

        let spec =
            try_generate_openapi_spec_with_validation_rules("Foundry", "1.0.0", &routes, &[], true)
                .expect("authoritative app-backed OpenAPI should publish an empty rule manifest");

        assert_eq!(spec["x-foundry-validation-rules"], serde_json::json!({}));
    }

    #[test]
    fn try_generate_openapi_spec_rejects_duplicate_method_path_docs() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users/:id".to_string(),
                doc: RouteDoc::new().get().response::<String>(200),
            },
            DocumentedRoute {
                method: "GET".to_string(),
                path: "/users/{id}".to_string(),
                doc: RouteDoc::new().get().response::<String>(200),
            },
        ];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("duplicate OpenAPI method/path docs should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route `get /users/{id}` is documented multiple times"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_route_paths() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "users".to_string(),
            doc: RouteDoc::new().get().response::<String>(200),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("relative OpenAPI route paths should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route path `users` is invalid"),
            "unexpected error: {error}"
        );

        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: " /users ".to_string(),
            doc: RouteDoc::new().get().response::<String>(200),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("padded OpenAPI route paths should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route path ` /users ` is invalid"),
            "unexpected error: {error}"
        );

        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users/{}".to_string(),
            doc: RouteDoc::new().get().response::<String>(200),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("malformed OpenAPI route path params should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI route path `/users/{}` contains invalid route path parameter segment `{}`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_unsupported_http_methods() {
        let routes = vec![DocumentedRoute {
            method: "fetch".to_string(),
            path: "/users".to_string(),
            doc: RouteDoc::new().method("fetch").response::<String>(200),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("unsupported OpenAPI route methods should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route `fetch /users` uses unsupported HTTP method `fetch`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_response_container_schema_missing_inner_marker() {
        let mut doc = RouteDoc::new().get();
        doc.responses.push((
            200,
            crate::openapi::SchemaRef {
                name: "ManualArray",
                schema_fn: manual_array_schema_without_item_marker,
            },
        ));
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/bulk".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI response array schemas without item markers should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI route `get /bulk` response status `200` has invalid schema metadata: schema `ManualArray` is documented as array but is missing `x-foundry-item-schema`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_request_container_schema_missing_inner_marker() {
        let mut doc = RouteDoc::new().post().response::<String>(200);
        doc.request = Some(crate::openapi::SchemaRef {
            name: "ManualArray",
            schema_fn: manual_array_schema_without_item_marker,
        });
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/bulk".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI request array schemas without item markers should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI route `post /bulk` request has invalid schema metadata: schema `ManualArray` is documented as array but is missing `x-foundry-item-schema`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_allows_strict_object_request_schemas() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/strict".to_string(),
            doc: RouteDoc::new()
                .post()
                .request::<StrictOpenApiRequest>()
                .response::<StrictOpenApiRequest>(200),
        }];

        let spec = try_generate_openapi_spec("Foundry", "1.0.0", &routes).unwrap();
        let schema = &spec["components"]["schemas"]["StrictOpenApiRequest"];

        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            spec["paths"]["/strict"]["post"]["requestBody"]["content"]["application/json"]
                ["schema"]["$ref"],
            serde_json::json!("#/components/schemas/StrictOpenApiRequest")
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_duplicate_operation_ids() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .operation_id("users.index")
                    .response::<String>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/admins".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .operation_id("users.index")
                    .response::<String>(200),
            },
        ];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("duplicate OpenAPI operation ids should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI operationId `users.index` is used by multiple routes"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_duplicate_response_statuses() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc: RouteDoc::new()
                .get()
                .response::<String>(200)
                .response::<u64>(200),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("duplicate OpenAPI response statuses should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI route `get /users` documents response status `200` multiple times"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_duplicate_route_ids() {
        let mut users_doc = RouteDoc::new()
            .get()
            .operation_id("users.index")
            .response::<String>(200);
        users_doc.route_id = Some("admin.users.index".to_string());
        let mut admins_doc = RouteDoc::new()
            .get()
            .operation_id("admins.index")
            .response::<String>(200);
        admins_doc.route_id = Some("admin.users.index".to_string());

        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users".to_string(),
                doc: users_doc,
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/admins".to_string(),
                doc: admins_doc,
            },
        ];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("duplicate OpenAPI route ids should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI route id export contains duplicate route id `admin.users.index`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_route_id_grouping_collisions() {
        let mut users_doc = RouteDoc::new()
            .get()
            .operation_id("users.index")
            .response::<String>(200);
        users_doc.route_id = Some("admin.users".to_string());
        let mut show_doc = RouteDoc::new()
            .get()
            .operation_id("users.show")
            .response::<String>(200);
        show_doc.route_id = Some("admin.users.show".to_string());

        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users".to_string(),
                doc: users_doc,
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users/{id}".to_string(),
                doc: show_doc,
            },
        ];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route id grouping collisions should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI route id export cannot group route id `admin.users.show` because `admin.users` is already a route id"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_empty_route_id_segments() {
        let mut doc = RouteDoc::new()
            .get()
            .operation_id("users.index")
            .response::<String>(200);
        doc.route_id = Some("admin..users".to_string());
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route id segments should be non-empty");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route id export requires non-empty route id segments"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_camel_case_route_id_collisions() {
        let mut snake_doc = RouteDoc::new()
            .get()
            .operation_id("audit.logs.index")
            .response::<String>(200);
        snake_doc.route_id = Some("admin.audit_logs.index".to_string());
        let mut kebab_doc = RouteDoc::new()
            .get()
            .operation_id("audit-logs.index")
            .response::<String>(200);
        kebab_doc.route_id = Some("admin.audit-logs.index".to_string());

        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/audit-logs".to_string(),
                doc: snake_doc,
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/audit/logs".to_string(),
                doc: kebab_doc,
            },
        ];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route id camelCase collisions should fail");
        assert!(
            error.to_string().contains("both normalize to `auditLogs`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_route_document_metadata() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.operation_id = Some(" admin.users.index ".to_string());
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route document metadata should be trimmed");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route `get /users` contains invalid operationId"),
            "unexpected error: {error}"
        );

        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some(" ".to_string());
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route ids should be non-blank");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route `get /users` contains invalid route id"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_duplicate_route_document_tags() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.tags = vec!["admin".to_string(), "admin".to_string()];
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route document tags should be unique");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route `get /users` contains duplicate tag `admin`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_route_auth_metadata() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.auth_required = true;
        doc.auth_guard = Some(" admin ".to_string());
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route auth guard metadata should be trimmed");
        assert!(
            error.to_string().contains(
                "OpenAPI route auth for route `admin.users.index` contains invalid guard ` admin `"
            ),
            "unexpected error: {error}"
        );

        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.auth_required = true;
        doc.auth_permissions = vec!["users.read".to_string(), "users.read".to_string()];
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route auth permission metadata should be unique");
        assert!(
            error.to_string().contains(
                "OpenAPI route auth for route `admin.users.index` contains duplicate permission `users.read`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_guarded_route_auth_metadata_without_required_auth() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.auth_required = false;
        doc.auth_guard = Some("admin".to_string());
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI guarded route auth metadata without required auth should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI route auth for route `admin.users.index` has guard, permission, or authorize callback metadata but auth is not required"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_route_policy_text_metadata() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.middleware_group = Some(" api ".to_string());
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route policy middlewareGroup metadata should be trimmed");
        assert!(
            error.to_string().contains(
                "OpenAPI route policy for route `admin.users.index` contains invalid middlewareGroup ` api `"
            ),
            "unexpected error: {error}"
        );

        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.audit_area = Some(" ".to_string());
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route policy auditArea metadata should be non-blank");
        assert!(
            error.to_string().contains(
                "OpenAPI route policy for route `admin.users.index` contains invalid auditArea"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_route_policy_rate_limit_counts() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.rate_limits = vec![crate::openapi::RouteDocRateLimit {
            max_requests: 0,
            window_seconds: 60,
            by: "ip".to_string(),
        }];
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route policy rate-limit maxRequests `0` should fail");

        assert!(
            error.to_string().contains(
                "OpenAPI route policy for route `admin.users.index` contains rate-limit metadata with maxRequests `0`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_route_policy_window_above_javascript_safe_integer_limit() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.rate_limits = vec![crate::openapi::RouteDocRateLimit {
            max_requests: 60,
            window_seconds: crate::support::javascript::JAVASCRIPT_MAX_SAFE_INTEGER as u64 + 1,
            by: "ip".to_string(),
        }];
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route policy rate-limit windows above JavaScript's safe integer limit should fail");

        assert!(
            error.to_string().contains(
                "OpenAPI route policy for route `admin.users.index` rate-limit windowSeconds"
            ) && error
                .to_string()
                .contains("above JavaScript's safe integer limit"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_route_policy_rate_limit_by() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.auth_required = true;
        doc.rate_limits = vec![crate::openapi::RouteDocRateLimit {
            max_requests: 60,
            window_seconds: 60,
            by: "session".to_string(),
        }];
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route policy rate-limit by values should be known");

        assert!(
            error.to_string().contains(
                "OpenAPI route policy for route `admin.users.index` contains invalid rate-limit by `session`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_public_actor_route_policy_rate_limit() {
        let mut doc = RouteDoc::new().get().response::<String>(200);
        doc.route_id = Some("admin.users.index".to_string());
        doc.auth_required = false;
        doc.rate_limits = vec![crate::openapi::RouteDocRateLimit {
            max_requests: 60,
            window_seconds: 60,
            by: "actor".to_string(),
        }];
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc,
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI route policy actor rate limits on public routes should fail");

        assert!(
            error.to_string().contains(
                "OpenAPI route policy for route `admin.users.index` contains actor rate-limit metadata but auth is not required"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_response_statuses() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users".to_string(),
            doc: RouteDoc::new().get().response::<String>(99),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("invalid OpenAPI response statuses should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route `get /users` documents invalid response status `99`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_routes_without_responses() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/users/search".to_string(),
            doc: RouteDoc::new().post().request::<String>(),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("OpenAPI routes without responses should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI route `post /users/search` has no documented responses"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn generated_operations_keep_route_doc_tags_unique() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/admin/users".to_string(),
            doc: RouteDoc::new()
                .get()
                .tag("admin")
                .tag("admin")
                .tag("users")
                .response::<String>(200),
        }];

        let spec = try_generate_openapi_spec("Foundry", "1.0.0", &routes).unwrap();

        assert_eq!(
            spec["paths"]["/admin/users"]["get"]["tags"],
            serde_json::json!(["admin", "users"])
        );
    }

    #[test]
    fn generated_operations_use_normalized_route_doc_metadata() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/admin/users".to_string(),
            doc: RouteDoc::new()
                .get()
                .operation_id(" admin.users.index ")
                .summary(" List admin users ")
                .description("  Shows admin users.  ")
                .tag(" admin ")
                .tag("admin")
                .response::<String>(200),
        }];

        let spec = try_generate_openapi_spec("Foundry", "1.0.0", &routes).unwrap();
        let operation = &spec["paths"]["/admin/users"]["get"];

        assert_eq!(
            operation["operationId"],
            serde_json::json!("admin.users.index")
        );
        assert_eq!(operation["summary"], serde_json::json!("List admin users"));
        assert_eq!(
            operation["description"],
            serde_json::json!("Shows admin users.")
        );
        assert_eq!(operation["tags"], serde_json::json!(["admin"]));
    }

    #[test]
    fn try_generate_openapi_spec_rejects_invalid_component_schema_names() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/invalid-component".to_string(),
            doc: RouteDoc::new().get().response::<InvalidComponentName>(200),
        }];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("invalid OpenAPI component names should fail");
        assert!(
            error
                .to_string()
                .contains("OpenAPI component schema name `Invalid/Component` is invalid"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn try_generate_openapi_spec_accepts_valid_punctuated_component_names() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/valid-component".to_string(),
            doc: RouteDoc::new()
                .get()
                .response::<ValidPunctuatedComponentName>(200),
        }];

        let spec = try_generate_openapi_spec("Foundry", "1.0.0", &routes).unwrap();

        assert_eq!(
            spec["paths"]["/valid-component"]["get"]["responses"]["200"]["content"]
                ["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/Valid.Component_Name-1")
        );
        assert!(spec["components"]["schemas"]
            .get("Valid.Component_Name-1")
            .is_some());
    }

    #[test]
    fn try_generate_openapi_spec_reuses_identical_component_schemas() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/name".to_string(),
                doc: RouteDoc::new().get().response::<String>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/display-name".to_string(),
                doc: RouteDoc::new().get().response::<String>(200),
            },
        ];

        let spec = try_generate_openapi_spec("Foundry", "1.0.0", &routes).unwrap();

        assert_eq!(
            spec["components"]["schemas"]["String"],
            serde_json::json!({ "type": "string" })
        );
        assert_eq!(
            spec["paths"]["/name"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            serde_json::json!("#/components/schemas/String")
        );
        assert_eq!(
            spec["paths"]["/display-name"]["get"]["responses"]["200"]["content"]
                ["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/String")
        );
    }

    #[test]
    fn try_generate_openapi_spec_rejects_conflicting_component_schemas() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/alpha".to_string(),
                doc: RouteDoc::new().get().response::<ConflictingComponentA>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/beta".to_string(),
                doc: RouteDoc::new().get().response::<ConflictingComponentB>(200),
            },
        ];

        let error = try_generate_openapi_spec("Foundry", "1.0.0", &routes)
            .expect_err("conflicting OpenAPI component schemas should fail");
        assert!(
            error.to_string().contains(
                "OpenAPI component schema `ConflictingComponent` is generated with conflicting shapes"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unit_responses_omit_json_content() {
        let routes = vec![DocumentedRoute {
            method: "delete".to_string(),
            path: "/users/{id}".to_string(),
            doc: RouteDoc::new().delete().response::<()>(204),
        }];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let response = &spec["paths"]["/users/{id}"]["delete"]["responses"]["204"];

        assert_eq!(response["description"], serde_json::json!(""));
        assert_eq!(
            response["x-foundry-response-has-body"],
            serde_json::json!(false)
        );
        assert!(response.get("x-foundry-response-media-type").is_none());
        assert!(response.get("content").is_none());
        assert!(spec["components"]["schemas"].get("Unit").is_none());
    }

    #[test]
    fn json_responses_keep_json_content() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/health".to_string(),
            doc: RouteDoc::new()
                .get()
                .operation_id("health.check")
                .response::<String>(200),
        }];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        assert_eq!(
            spec["paths"]["/health"]["get"]["operationId"],
            serde_json::json!("health.check")
        );
        let response = &spec["paths"]["/health"]["get"]["responses"]["200"];
        let schema = &response["content"]["application/json"]["schema"];

        assert_eq!(
            response["x-foundry-response-has-body"],
            serde_json::json!(true)
        );
        assert_eq!(
            response["x-foundry-response-media-type"],
            serde_json::json!("application/json")
        );
        assert_eq!(
            schema["$ref"],
            serde_json::json!("#/components/schemas/String")
        );
        assert_eq!(
            spec["components"]["schemas"]["String"],
            serde_json::json!({"type": "string"})
        );
    }

    #[test]
    fn file_responses_use_binary_content() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/avatar".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<crate::storage::UploadedFile>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/gallery".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<Vec<crate::storage::UploadedFile>>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/avatar/optional".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<Option<crate::storage::UploadedFile>>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/gallery/optional".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<Option<Vec<crate::storage::UploadedFile>>>(200),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let avatar = &spec["paths"]["/avatar"]["get"]["responses"]["200"];
        let gallery = &spec["paths"]["/gallery"]["get"]["responses"]["200"];
        let optional_avatar = &spec["paths"]["/avatar/optional"]["get"]["responses"]["200"];
        let optional_gallery = &spec["paths"]["/gallery/optional"]["get"]["responses"]["200"];

        assert!(avatar["content"].get("application/json").is_none());
        assert_eq!(
            avatar["x-foundry-response-has-body"],
            serde_json::json!(true)
        );
        assert_eq!(
            avatar["x-foundry-response-media-type"],
            serde_json::json!("application/octet-stream")
        );
        assert_eq!(
            avatar["content"]["application/octet-stream"]["schema"],
            serde_json::json!({"$ref": "#/components/schemas/UploadedFile"})
        );
        assert_eq!(
            spec["components"]["schemas"]["UploadedFile"],
            serde_json::json!({"type": "string", "format": "binary"})
        );
        assert!(gallery["content"].get("application/json").is_none());
        assert_eq!(
            gallery["x-foundry-response-has-body"],
            serde_json::json!(true)
        );
        assert_eq!(
            gallery["x-foundry-response-media-type"],
            serde_json::json!("application/octet-stream")
        );
        assert_eq!(
            gallery["content"]["application/octet-stream"]["schema"],
            serde_json::json!({
                "type": "array",
                "items": { "type": "string", "format": "binary" },
                "x-foundry-item-schema": "UploadedFile"
            })
        );
        assert!(optional_avatar["content"].get("application/json").is_none());
        assert_eq!(
            optional_avatar["content"]["application/octet-stream"]["schema"],
            serde_json::json!({"type": "string", "format": "binary", "nullable": true})
        );
        assert!(optional_gallery["content"]
            .get("application/json")
            .is_none());
        assert_eq!(
            optional_gallery["content"]["application/octet-stream"]["schema"],
            serde_json::json!({
                "type": "array",
                "items": { "type": "string", "format": "binary" },
                "x-foundry-item-schema": "UploadedFile",
                "nullable": true
            })
        );
    }

    #[test]
    fn validation_error_response_schema_documents_required_error_details() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/users".to_string(),
            doc: RouteDoc::new().post().validation_errors(),
        }];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let response_schema = &spec["paths"]["/users"]["post"]["responses"]["422"]["content"]
            ["application/json"]["schema"];
        assert_eq!(
            response_schema["$ref"],
            serde_json::json!("#/components/schemas/ValidationErrorResponse")
        );

        let schema = &spec["components"]["schemas"]["ValidationErrorResponse"];
        assert_eq!(
            schema["required"],
            serde_json::json!(["message", "status", "errors"])
        );
        assert_eq!(
            schema["properties"]["status"]["type"],
            serde_json::json!("integer")
        );
        assert_eq!(
            schema["properties"]["errors"]["items"]["required"],
            serde_json::json!(["field", "code", "message"])
        );
        assert_eq!(
            schema["properties"]["errors"]["items"]["properties"]["field"]["type"],
            serde_json::json!("string")
        );
    }

    #[test]
    fn pagination_response_wrappers_are_inlined_with_item_schema_metadata() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<crate::database::PaginatedResponse<String>>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users/cursor".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<crate::database::CursorPaginated<String>>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users/collection".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<crate::support::Collection<String>>(200),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let paginated = &spec["paths"]["/users"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"];
        assert_eq!(
            paginated["x-foundry-wrapper-schema"],
            serde_json::json!("PaginatedResponse")
        );
        assert_eq!(
            paginated["properties"]["data"]["items"],
            serde_json::json!({"type": "string"})
        );
        assert_eq!(
            paginated["properties"]["meta"]["properties"]["current_page"]["type"],
            serde_json::json!("integer")
        );
        assert!(spec["components"]["schemas"]
            .get("PaginatedResponse")
            .is_none());

        let cursor = &spec["paths"]["/users/cursor"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"];
        assert_eq!(
            cursor["x-foundry-wrapper-schema"],
            serde_json::json!("CursorPaginated")
        );
        assert_eq!(
            cursor["properties"]["data"]["items"],
            serde_json::json!({"type": "string"})
        );
        assert_eq!(
            cursor["properties"]["meta"]["properties"]["has_next"]["type"],
            serde_json::json!("boolean")
        );
        assert!(spec["components"]["schemas"]
            .get("CursorPaginated")
            .is_none());

        let collection = &spec["paths"]["/users/collection"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"];
        assert_eq!(
            collection["x-foundry-wrapper-schema"],
            serde_json::json!("Collection")
        );
        assert_eq!(
            collection["x-foundry-data-schema"],
            serde_json::json!("String")
        );
        assert_eq!(
            collection["properties"]["items"]["items"],
            serde_json::json!({"type": "string"})
        );
        assert!(spec["components"]["schemas"].get("Collection").is_none());
    }

    #[test]
    fn path_params_are_documented_and_framework_paths_normalized() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/files/{*path}/users/:id".to_string(),
            doc: RouteDoc::new().get().response::<String>(200),
        }];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let operation = &spec["paths"]["/files/{path}/users/{id}"]["get"];
        let params = operation["parameters"].as_array().expect("parameters");

        assert!(spec["paths"].get("/files/{*path}/users/:id").is_none());
        assert_eq!(params.len(), 2);
        assert_eq!(params[0]["name"], serde_json::json!("path"));
        assert_eq!(params[0]["in"], serde_json::json!("path"));
        assert_eq!(params[0]["required"], serde_json::json!(true));
        assert_eq!(params[0]["schema"], serde_json::json!({ "type": "string" }));
        assert_eq!(params[0]["x-foundry-catch-all"], serde_json::json!(true));
        assert_eq!(params[1]["name"], serde_json::json!("id"));
        assert_eq!(params[1]["in"], serde_json::json!("path"));
        assert_eq!(params[1]["required"], serde_json::json!(true));
        assert!(params[1].get("x-foundry-catch-all").is_none());
    }

    #[allow(dead_code)]
    #[derive(ts_rs::TS, crate::ApiSchema)]
    struct SearchUsersRequest {
        query: String,
        page: Option<u64>,
        tags: Vec<String>,
        status_ids: Option<Vec<u64>>,
    }

    #[allow(dead_code)]
    #[derive(ts_rs::TS, crate::ApiSchema)]
    struct NestedSearchUsersRequest {
        search: SearchUsersRequest,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, ts_rs::TS, crate::ApiSchema)]
    struct GetUploadRequest {
        avatar: crate::storage::UploadedFile,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    #[serde(rename_all = "camelCase")]
    struct ManualOpenApiNestedValidationAddress {
        street_name: String,
        postal_code: String,
        unit_number: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct ManualOpenApiQueryValidationRequest {
        value: String,
        nickname: Option<String>,
        tags: Vec<String>,
    }

    impl crate::typescript::TsValidationSchemaProvider for ManualOpenApiQueryValidationRequest {
        fn ts_validation_schema() -> crate::typescript::TsValidationSchema {
            crate::typescript::TsValidationSchema::new()
                .field_value_kind("tags", crate::typescript::TsValidationFieldValueKind::Array)
                .rule(crate::typescript::TsValidationRule::after_hook(
                    "validate_manual_openapi_payload",
                ))
                .field(
                    "value",
                    [
                        crate::typescript::TsValidationRule::required(),
                        crate::typescript::TsValidationRule::min(3),
                        crate::typescript::TsValidationRule::max(32),
                        crate::typescript::TsValidationRule::starts_with(["usr."]),
                        crate::typescript::TsValidationRule::doesnt_contain(["legacy."]),
                    ],
                )
                .field(
                    "nickname",
                    [crate::typescript::TsValidationRule::required()],
                )
        }
    }

    ::foundry::inventory::submit! {
        crate::typescript::TsValidation {
            name: "ManualOpenApiQueryValidationRequest",
            schema_fn: || <ManualOpenApiQueryValidationRequest as crate::typescript::TsValidationSchemaProvider>::ts_validation_schema(),
        }
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum ManualOpenApiPriority {
        Low = 1,
        High = 2,
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct ManualOpenApiValidationRequest {
        value: String,
        score: f64,
        tags: Vec<String>,
        status: String,
        level: i32,
        ratio: f64,
        blocked_level: i32,
        priority: i32,
        username: String,
        profile: ManualOpenApiNestedValidationAddress,
        addresses: Vec<ManualOpenApiNestedValidationAddress>,
        reviewer: Option<String>,
        token: String,
    }

    impl crate::typescript::TsValidationSchemaProvider for ManualOpenApiValidationRequest {
        fn ts_validation_schema() -> crate::typescript::TsValidationSchema {
            let address_schema = || {
                crate::typescript::TsValidationSchema::new()
                    .deny_unknown_fields()
                    .known_fields(["streetName", "postalCode", "unitNumber"])
                    .field(
                        "streetName",
                        [
                            crate::typescript::TsValidationRule::required(),
                            crate::typescript::TsValidationRule::min(3),
                        ],
                    )
                    .field(
                        "postalCode",
                        [crate::typescript::TsValidationRule::required()],
                    )
            };

            crate::typescript::TsValidationSchema::new()
                .deny_unknown_fields()
                .field_value_kind("tags", crate::typescript::TsValidationFieldValueKind::Array)
                .field_value_kind(
                    "profile",
                    crate::typescript::TsValidationFieldValueKind::Nested,
                )
                .field_value_kind(
                    "addresses",
                    crate::typescript::TsValidationFieldValueKind::Array,
                )
                .rule(
                    crate::typescript::TsValidationRule::new("after")
                        .param("hook", "validate_manual_openapi_payload")
                        .server_only(),
                )
                .field(
                    "value",
                    [
                        crate::typescript::TsValidationRule::required(),
                        crate::typescript::TsValidationRule::min(3),
                        crate::typescript::TsValidationRule::max(32),
                        crate::typescript::TsValidationRule::starts_with(["usr."]),
                        crate::typescript::TsValidationRule::doesnt_contain(["legacy."]),
                    ],
                )
                .field(
                    "score",
                    [
                        crate::typescript::TsValidationRule::min_numeric(1.5),
                        crate::typescript::TsValidationRule::max_numeric(9.5),
                        crate::typescript::TsValidationRule::multiple_of(0.5),
                    ],
                )
                .field(
                    "tags",
                    [
                        crate::typescript::TsValidationRule::min_items(1),
                        crate::typescript::TsValidationRule::max_items(5),
                        crate::typescript::TsValidationRule::contains_all(["rust", "foundry"]),
                        crate::typescript::TsValidationRule::doesnt_contain_any(["legacy"]),
                        crate::typescript::TsValidationRule::distinct(),
                        crate::typescript::TsValidationRule::each([
                            crate::typescript::TsValidationRule::max(20),
                        ]),
                    ],
                )
                .field(
                    "status",
                    [crate::typescript::TsValidationRule::in_list([
                        "draft",
                        "published",
                    ])],
                )
                .field(
                    "level",
                    [crate::typescript::TsValidationRule::in_list([1, 2])],
                )
                .field(
                    "ratio",
                    [crate::typescript::TsValidationRule::in_list([1.5, 2.5])],
                )
                .field(
                    "blocked_level",
                    [crate::typescript::TsValidationRule::not_in([0, -1])],
                )
                .field(
                    "priority",
                    [crate::typescript::TsValidationRule::app_enum::<
                        ManualOpenApiPriority,
                    >()],
                )
                .field(
                    "username",
                    [crate::typescript::TsValidationRule::not_in([
                        "root", "admin",
                    ])],
                )
                .field(
                    "profile",
                    [crate::typescript::TsValidationRule::nested(address_schema())],
                )
                .field(
                    "addresses",
                    [
                        crate::typescript::TsValidationRule::min_items(1),
                        crate::typescript::TsValidationRule::each([
                            crate::typescript::TsValidationRule::nested(address_schema()),
                        ]),
                    ],
                )
                .field(
                    "reviewer",
                    [crate::typescript::TsValidationRule::required()],
                )
                .field(
                    "token",
                    [crate::typescript::TsValidationRule::new("manual_token").server_only()],
                )
        }
    }

    ::foundry::inventory::submit! {
        crate::typescript::TsValidation {
            name: "ManualOpenApiValidationRequest",
            schema_fn: || <ManualOpenApiValidationRequest as crate::typescript::TsValidationSchemaProvider>::ts_validation_schema(),
        }
    }

    #[allow(dead_code)]
    #[derive(ts_rs::TS, crate::ApiSchema)]
    struct ManualOpenApiFileValidationRequest {
        avatar: crate::storage::UploadedFile,
        photos: Vec<crate::storage::UploadedFile>,
    }

    impl crate::typescript::TsValidationSchemaProvider for ManualOpenApiFileValidationRequest {
        fn ts_validation_schema() -> crate::typescript::TsValidationSchema {
            crate::typescript::TsValidationSchema::new()
                .field(
                    "avatar",
                    [
                        crate::typescript::TsValidationRule::max_file_size(2048),
                        crate::typescript::TsValidationRule::allowed_extensions(["jpg", "png"]),
                        crate::typescript::TsValidationRule::image(),
                        crate::typescript::TsValidationRule::allowed_mimes([
                            "image/jpeg",
                            "image/png",
                        ]),
                        crate::typescript::TsValidationRule::max_dimensions(1024, 768),
                        crate::typescript::TsValidationRule::min_dimensions(128, 128),
                    ],
                )
                .field(
                    "photos",
                    [
                        crate::typescript::TsValidationRule::min_items(1),
                        crate::typescript::TsValidationRule::max_file_size(4096),
                        crate::typescript::TsValidationRule::allowed_extensions(["webp"]),
                        crate::typescript::TsValidationRule::image(),
                    ],
                )
        }
    }

    ::foundry::inventory::submit! {
        crate::typescript::TsValidation {
            name: "ManualOpenApiFileValidationRequest",
            schema_fn: || <ManualOpenApiFileValidationRequest as crate::typescript::TsValidationSchemaProvider>::ts_validation_schema(),
        }
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct ManualOpenApiKeyValidationRequest {
        settings: BTreeMap<String, String>,
    }

    impl crate::typescript::TsValidationSchemaProvider for ManualOpenApiKeyValidationRequest {
        fn ts_validation_schema() -> crate::typescript::TsValidationSchema {
            crate::typescript::TsValidationSchema::new().field(
                "settings",
                [crate::typescript::TsValidationRule::required_keys([
                    "timezone", "locale",
                ])],
            )
        }
    }

    ::foundry::inventory::submit! {
        crate::typescript::TsValidation {
            name: "ManualOpenApiKeyValidationRequest",
            schema_fn: || <ManualOpenApiKeyValidationRequest as crate::typescript::TsValidationSchemaProvider>::ts_validation_schema(),
        }
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct ManualOpenApiResponseOnlySchema {
        value: String,
    }

    impl crate::typescript::TsValidationSchemaProvider for ManualOpenApiResponseOnlySchema {
        fn ts_validation_schema() -> crate::typescript::TsValidationSchema {
            crate::typescript::TsValidationSchema::new().field(
                "value",
                [crate::typescript::TsValidationRule::new("manual_response_only").server_only()],
            )
        }
    }

    ::foundry::inventory::submit! {
        crate::typescript::TsValidation {
            name: "ManualOpenApiResponseOnlySchema",
            schema_fn: || <ManualOpenApiResponseOnlySchema as crate::typescript::TsValidationSchemaProvider>::ts_validation_schema(),
        }
    }

    #[test]
    fn get_request_objects_are_documented_as_query_parameters() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/tenants/{tenant}/users".to_string(),
            doc: RouteDoc::new()
                .get()
                .request::<SearchUsersRequest>()
                .response::<String>(200),
        }];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let operation = &spec["paths"]["/tenants/{tenant}/users"]["get"];
        let params = operation["parameters"].as_array().expect("parameters");

        assert!(operation.get("requestBody").is_none());
        assert_eq!(
            operation["x-foundry-request-transport"],
            serde_json::json!("query")
        );
        assert!(operation.get("x-foundry-request-media-type").is_none());
        assert!(params
            .iter()
            .any(|param| param["name"] == serde_json::json!("tenant")
                && param["in"] == serde_json::json!("path")));

        let query = params
            .iter()
            .find(|param| param["name"] == serde_json::json!("query"))
            .expect("query parameter");
        assert_eq!(query["in"], serde_json::json!("query"));
        assert_eq!(query["required"], serde_json::json!(true));
        assert_eq!(query["schema"], serde_json::json!({"type": "string"}));

        let page = params
            .iter()
            .find(|param| param["name"] == serde_json::json!("page"))
            .expect("page parameter");
        assert_eq!(page["in"], serde_json::json!("query"));
        assert_eq!(page["required"], serde_json::json!(false));
        assert_eq!(page["schema"]["type"], serde_json::json!("integer"));
        assert_eq!(page["schema"]["nullable"], serde_json::json!(true));

        let tags = params
            .iter()
            .find(|param| param["name"] == serde_json::json!("tags"))
            .expect("tags parameter");
        assert_eq!(tags["in"], serde_json::json!("query"));
        assert_eq!(tags["required"], serde_json::json!(true));
        assert_eq!(tags["schema"]["type"], serde_json::json!("array"));
        assert_eq!(tags["schema"]["items"]["type"], serde_json::json!("string"));
        assert_eq!(tags["style"], serde_json::json!("form"));
        assert_eq!(tags["explode"], serde_json::json!(true));

        let status_ids = params
            .iter()
            .find(|param| param["name"] == serde_json::json!("status_ids"))
            .expect("status_ids parameter");
        assert_eq!(status_ids["in"], serde_json::json!("query"));
        assert_eq!(status_ids["required"], serde_json::json!(false));
        assert_eq!(status_ids["schema"]["type"], serde_json::json!("array"));
        assert_eq!(
            status_ids["schema"]["items"]["type"],
            serde_json::json!("integer")
        );
        assert_eq!(status_ids["schema"]["nullable"], serde_json::json!(true));
        assert_eq!(status_ids["style"], serde_json::json!("form"));
        assert_eq!(status_ids["explode"], serde_json::json!(true));
    }

    #[test]
    fn get_wrapper_and_file_request_schemas_stay_body_requests() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users/collection".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .request::<crate::support::Collection<String>>()
                    .response::<String>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users/nested-search".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .request::<NestedSearchUsersRequest>()
                    .response::<String>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/profile/avatar".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .request::<GetUploadRequest>()
                    .response::<String>(200),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let collection = &spec["paths"]["/users/collection"]["get"];
        let collection_schema = &collection["requestBody"]["content"]["application/json"]["schema"];
        assert!(collection.get("parameters").is_none());
        assert_eq!(
            collection["x-foundry-request-transport"],
            serde_json::json!("body")
        );
        assert_eq!(
            collection["x-foundry-request-media-type"],
            serde_json::json!("application/json")
        );
        assert_eq!(
            collection_schema["x-foundry-wrapper-schema"],
            serde_json::json!("Collection")
        );
        assert_eq!(
            collection_schema["properties"]["items"]["type"],
            serde_json::json!("array")
        );

        let nested = &spec["paths"]["/users/nested-search"]["get"];
        let nested_schema = &nested["requestBody"]["content"]["application/json"]["schema"];
        assert!(nested.get("parameters").is_none());
        assert_eq!(
            nested["x-foundry-request-transport"],
            serde_json::json!("body")
        );
        assert_eq!(
            nested["x-foundry-request-media-type"],
            serde_json::json!("application/json")
        );
        assert_eq!(
            nested_schema["$ref"],
            serde_json::json!("#/components/schemas/NestedSearchUsersRequest")
        );

        let upload = &spec["paths"]["/profile/avatar"]["get"];
        let upload_schema = &upload["requestBody"]["content"]["multipart/form-data"]["schema"];
        assert!(upload.get("parameters").is_none());
        assert_eq!(
            upload["x-foundry-request-transport"],
            serde_json::json!("body")
        );
        assert_eq!(
            upload["x-foundry-request-media-type"],
            serde_json::json!("multipart/form-data")
        );
        assert_eq!(
            upload_schema["$ref"],
            serde_json::json!("#/components/schemas/GetUploadRequest")
        );
    }

    #[test]
    fn post_request_objects_still_use_request_body() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/users/search".to_string(),
            doc: RouteDoc::new()
                .post()
                .request::<SearchUsersRequest>()
                .response::<()>(204),
        }];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let operation = &spec["paths"]["/users/search"]["post"];
        let schema = &operation["requestBody"]["content"]["application/json"]["schema"];

        assert!(operation.get("parameters").is_none());
        assert_eq!(
            operation["x-foundry-request-transport"],
            serde_json::json!("body")
        );
        assert_eq!(
            operation["x-foundry-request-media-type"],
            serde_json::json!("application/json")
        );
        assert_eq!(
            schema["$ref"],
            serde_json::json!("#/components/schemas/SearchUsersRequest")
        );
    }

    #[test]
    fn registered_validation_metadata_is_merged_into_openapi_request_schemas() {
        let routes = vec![
            DocumentedRoute {
                method: "post".to_string(),
                path: "/manual-validation".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<ManualOpenApiValidationRequest>()
                    .response::<()>(202),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/manual-validation/search".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .request::<ManualOpenApiQueryValidationRequest>()
                    .response::<String>(200),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/manual-validation/bulk".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<Vec<ManualOpenApiValidationRequest>>()
                    .response::<()>(202),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/manual-validation/keys".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<ManualOpenApiKeyValidationRequest>()
                    .response::<()>(202),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/manual-validation/upload".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<ManualOpenApiFileValidationRequest>()
                    .response::<()>(202),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/manual-validation/response-only".to_string(),
                doc: RouteDoc::new()
                    .get()
                    .response::<ManualOpenApiResponseOnlySchema>(200),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let schema = &spec["components"]["schemas"]["ManualOpenApiValidationRequest"];
        let expected_nested_validation_schema = serde_json::json!({
            "denyUnknownFields": true,
            "knownFields": ["streetName", "postalCode", "unitNumber"],
            "fields": [
                {
                    "name": "streetName",
                    "rules": [
                        { "code": "required" },
                        { "code": "min", "params": { "min": "3" } },
                    ],
                },
                {
                    "name": "postalCode",
                    "rules": [{ "code": "required" }],
                },
            ],
        });
        let expected_value_validation = serde_json::json!([
            { "code": "required" },
            { "code": "min", "params": { "min": "3" } },
            { "code": "max", "params": { "max": "32" } },
            {
                "code": "starts_with",
                "params": { "value": "usr." },
                "values": ["usr."],
            },
            {
                "code": "doesnt_contain",
                "params": { "value": "legacy." },
                "values": ["legacy."],
            },
        ]);
        assert_eq!(
            schema["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "after",
                    "params": { "hook": "validate_manual_openapi_payload" },
                    "serverOnly": true,
                },
            ])
        );
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            schema["x-foundry-validation-field-value-kinds"],
            serde_json::json!([
                { "field": "tags", "kind": "array" },
                { "field": "profile", "kind": "nested" },
                { "field": "addresses", "kind": "array" },
            ])
        );
        let schema_required = schema["required"]
            .as_array()
            .expect("schema required fields");
        assert!(
            schema_required
                .iter()
                .any(|field| field.as_str() == Some("reviewer")),
            "manual required validation should mark optional reviewer as OpenAPI required: {schema}"
        );
        let value = &schema["properties"]["value"];
        assert_eq!(&value["x-foundry-validation"], &expected_value_validation);
        assert_eq!(value["minLength"], serde_json::json!(3));
        assert_eq!(value["maxLength"], serde_json::json!(32));
        assert_eq!(value["pattern"], serde_json::json!("^usr\\."));
        assert_eq!(value["not"], serde_json::json!({ "pattern": "legacy\\." }));

        let score = &schema["properties"]["score"];
        assert_eq!(score["minimum"], serde_json::json!(1.5));
        assert_eq!(score["maximum"], serde_json::json!(9.5));
        assert_eq!(score["multipleOf"], serde_json::json!(0.5));

        let tags = &schema["properties"]["tags"];
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
        assert_eq!(tags["items"]["maxLength"], serde_json::json!(20));
        assert_eq!(
            tags["items"]["x-foundry-validation"],
            serde_json::json!([{ "code": "max", "params": { "max": "20" } }])
        );
        assert_eq!(
            schema["properties"]["status"]["enum"],
            serde_json::json!(["draft", "published"])
        );
        assert_eq!(
            schema["properties"]["level"]["enum"],
            serde_json::json!([1, 2])
        );
        assert_eq!(
            schema["properties"]["level"]["x-foundry-validation"],
            serde_json::json!([{ "code": "in_list", "values": ["1", "2"] }])
        );
        assert_eq!(
            schema["properties"]["ratio"]["enum"],
            serde_json::json!([1.5, 2.5])
        );
        assert_eq!(
            schema["properties"]["blocked_level"]["not"],
            serde_json::json!({ "enum": [0, -1] })
        );
        assert_eq!(
            schema["properties"]["priority"]["enum"],
            serde_json::json!([1, 2])
        );
        assert_eq!(
            schema["properties"]["priority"]["x-foundry-validation"],
            serde_json::json!([{ "code": "app_enum", "values": ["1", "2"] }])
        );
        assert_eq!(
            schema["properties"]["username"]["not"],
            serde_json::json!({ "enum": ["root", "admin"] })
        );
        let profile = &schema["properties"]["profile"];
        assert_eq!(profile["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            profile["properties"]["streetName"]["minLength"],
            serde_json::json!(3)
        );
        assert_eq!(
            profile["properties"]["streetName"]["x-foundry-validation"],
            serde_json::json!([
                { "code": "required" },
                { "code": "min", "params": { "min": "3" } },
            ])
        );
        assert_eq!(
            profile["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "nested",
                    "schema": expected_nested_validation_schema.clone(),
                },
            ])
        );

        let addresses = &schema["properties"]["addresses"];
        assert_eq!(addresses["minItems"], serde_json::json!(1));
        assert_eq!(
            addresses["items"]["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(
            addresses["items"]["properties"]["streetName"]["minLength"],
            serde_json::json!(3)
        );
        assert_eq!(
            addresses["items"]["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "nested",
                    "schema": expected_nested_validation_schema.clone(),
                },
            ])
        );
        assert_eq!(
            schema["properties"]["token"]["x-foundry-validation"],
            serde_json::json!([{ "code": "manual_token", "serverOnly": true }])
        );

        let key_schema = &spec["components"]["schemas"]["ManualOpenApiKeyValidationRequest"];
        assert_eq!(
            key_schema["properties"]["settings"]["required"],
            serde_json::json!(["timezone", "locale"])
        );
        assert_eq!(
            key_schema["properties"]["settings"]["x-foundry-validation"],
            serde_json::json!([
                { "code": "required_keys", "values": ["timezone", "locale"] },
            ])
        );
        assert_eq!(
            spec["paths"]["/manual-validation/keys"]["post"]["requestBody"]["content"]
                ["application/json"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/ManualOpenApiKeyValidationRequest")
        );

        let file_schema = &spec["components"]["schemas"]["ManualOpenApiFileValidationRequest"];
        let avatar = &file_schema["properties"]["avatar"];
        assert_eq!(avatar["type"], serde_json::json!("string"));
        assert_eq!(avatar["format"], serde_json::json!("binary"));
        assert_eq!(
            avatar["x-foundry-max-file-size-kb"],
            serde_json::json!(2048)
        );
        assert_eq!(
            avatar["x-foundry-allowed-extensions"],
            serde_json::json!(["jpg", "png"])
        );
        assert_eq!(
            avatar["x-foundry-allowed-mimes"],
            serde_json::json!(["image/jpeg", "image/png"])
        );
        assert_eq!(
            avatar["x-foundry-server-only-validation"],
            serde_json::json!(["image", "allowed_mimes", "max_dimensions", "min_dimensions"])
        );
        assert_eq!(
            avatar["x-foundry-max-dimensions"],
            serde_json::json!({ "width": 1024, "height": 768 })
        );
        assert_eq!(
            avatar["x-foundry-min-dimensions"],
            serde_json::json!({ "width": 128, "height": 128 })
        );

        let photos = &file_schema["properties"]["photos"];
        assert_eq!(photos["minItems"], serde_json::json!(1));
        assert_eq!(
            photos["items"]["x-foundry-max-file-size-kb"],
            serde_json::json!(4096)
        );
        assert_eq!(
            photos["items"]["x-foundry-allowed-extensions"],
            serde_json::json!(["webp"])
        );
        assert_eq!(
            photos["items"]["x-foundry-server-only-validation"],
            serde_json::json!(["image"])
        );
        assert_eq!(
            spec["paths"]["/manual-validation/upload"]["post"]["requestBody"]["content"]
                ["multipart/form-data"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/ManualOpenApiFileValidationRequest")
        );

        let query_params = spec["paths"]["/manual-validation/search"]["get"]["parameters"]
            .as_array()
            .expect("query parameters");
        assert_eq!(
            spec["paths"]["/manual-validation/search"]["get"]["x-foundry-validation"],
            serde_json::json!([
                {
                    "code": "after",
                    "params": { "hook": "validate_manual_openapi_payload" },
                    "serverOnly": true,
                },
            ])
        );
        assert_eq!(
            spec["paths"]["/manual-validation/search"]["get"]
                ["x-foundry-validation-field-value-kinds"],
            serde_json::json!([{ "field": "tags", "kind": "array" }])
        );
        let value_param = query_params
            .iter()
            .find(|param| param["name"] == serde_json::json!("value"))
            .expect("value query parameter");
        assert_eq!(
            &value_param["schema"]["x-foundry-validation"],
            &expected_value_validation
        );
        assert_eq!(value_param["schema"]["minLength"], serde_json::json!(3));
        assert_eq!(value_param["schema"]["maxLength"], serde_json::json!(32));
        assert_eq!(
            value_param["schema"]["pattern"],
            serde_json::json!("^usr\\.")
        );
        let nickname_param = query_params
            .iter()
            .find(|param| param["name"] == serde_json::json!("nickname"))
            .expect("nickname query parameter");
        assert_eq!(nickname_param["required"], serde_json::json!(true));
        assert_eq!(
            nickname_param["schema"]["nullable"],
            serde_json::json!(true)
        );
        assert_eq!(
            nickname_param["schema"]["x-foundry-validation"],
            serde_json::json!([{ "code": "required" }])
        );
        let tags_param = query_params
            .iter()
            .find(|param| param["name"] == serde_json::json!("tags"))
            .expect("tags query parameter");
        assert_eq!(tags_param["in"], serde_json::json!("query"));
        assert_eq!(tags_param["style"], serde_json::json!("form"));
        assert_eq!(tags_param["explode"], serde_json::json!(true));
        assert_eq!(tags_param["schema"]["type"], serde_json::json!("array"));
        assert_eq!(
            tags_param["schema"]["items"],
            serde_json::json!({ "type": "string" })
        );

        let bulk_schema = &spec["paths"]["/manual-validation/bulk"]["post"]["requestBody"]
            ["content"]["application/json"]["schema"];
        assert_eq!(bulk_schema["type"], serde_json::json!("array"));
        assert_eq!(
            &bulk_schema["items"]["properties"]["value"]["x-foundry-validation"],
            &expected_value_validation
        );
        assert_eq!(
            bulk_schema["items"]["properties"]["tags"]["items"]["maxLength"],
            serde_json::json!(20)
        );
        assert_eq!(
            bulk_schema["items"]["properties"]["profile"]["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(
            bulk_schema["items"]["properties"]["addresses"]["items"]["properties"]["streetName"]
                ["minLength"],
            serde_json::json!(3)
        );

        let response_only = &spec["components"]["schemas"]["ManualOpenApiResponseOnlySchema"];
        assert!(
            response_only["properties"]["value"]
                .get("x-foundry-validation")
                .is_none(),
            "manual RequestValidator metadata should not be merged into response-only schemas: {response_only}"
        );
    }

    #[test]
    fn get_scalar_requests_document_body_transport() {
        let routes = vec![DocumentedRoute {
            method: "get".to_string(),
            path: "/users/search-token".to_string(),
            doc: RouteDoc::new()
                .get()
                .request::<String>()
                .response::<()>(204),
        }];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let operation = &spec["paths"]["/users/search-token"]["get"];

        assert_eq!(
            operation["x-foundry-request-transport"],
            serde_json::json!("body")
        );
        assert_eq!(
            operation["x-foundry-request-media-type"],
            serde_json::json!("application/json")
        );
        assert!(operation.get("parameters").is_none());
        assert!(operation.get("requestBody").is_some());
    }

    #[test]
    fn nullable_route_schemas_inline_without_overwriting_base_components() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/name".to_string(),
                doc: RouteDoc::new().get().response::<String>(200),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/nickname".to_string(),
                doc: RouteDoc::new().get().response::<Option<String>>(200),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let name_schema = &spec["paths"]["/name"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"];
        let nickname_schema = &spec["paths"]["/nickname"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"];

        assert_eq!(
            name_schema["$ref"],
            serde_json::json!("#/components/schemas/String")
        );
        assert_eq!(
            spec["components"]["schemas"]["String"],
            serde_json::json!({"type": "string"})
        );
        assert_eq!(nickname_schema["type"], serde_json::json!("string"));
        assert_eq!(nickname_schema["nullable"], serde_json::json!(true));
        assert!(nickname_schema.get("$ref").is_none());
    }

    #[test]
    fn collection_route_schemas_inline_without_shared_array_components() {
        let routes = vec![
            DocumentedRoute {
                method: "post".to_string(),
                path: "/names".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<Vec<String>>()
                    .response::<()>(204),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/tags".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<std::collections::HashSet<String>>()
                    .response::<()>(204),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/ids".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<Vec<u64>>()
                    .response::<()>(204),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let names_schema = &spec["paths"]["/names"]["post"]["requestBody"]["content"]
            ["application/json"]["schema"];
        let tags_schema =
            &spec["paths"]["/tags"]["post"]["requestBody"]["content"]["application/json"]["schema"];
        let ids_schema =
            &spec["paths"]["/ids"]["post"]["requestBody"]["content"]["application/json"]["schema"];

        assert_eq!(names_schema["type"], serde_json::json!("array"));
        assert_eq!(names_schema["items"]["type"], serde_json::json!("string"));
        assert_eq!(tags_schema["type"], serde_json::json!("array"));
        assert_eq!(tags_schema["items"]["type"], serde_json::json!("string"));
        assert_eq!(ids_schema["type"], serde_json::json!("array"));
        assert_eq!(ids_schema["items"]["type"], serde_json::json!("integer"));
        assert!(names_schema.get("$ref").is_none());
        assert!(tags_schema.get("$ref").is_none());
        assert!(ids_schema.get("$ref").is_none());
        assert!(spec["components"]["schemas"].get("Array").is_none());
    }

    #[allow(dead_code)]
    #[derive(ts_rs::TS, crate::ApiSchema)]
    struct MultipartAvatarRequest {
        avatar: crate::storage::UploadedFile,
        caption: String,
    }

    #[allow(dead_code)]
    #[derive(ts_rs::TS, crate::ApiSchema)]
    struct MultipartGalleryRequest {
        photos: Vec<crate::storage::UploadedFile>,
        caption: Option<String>,
    }

    #[test]
    fn file_bearing_requests_use_multipart_form_data() {
        let routes = vec![
            DocumentedRoute {
                method: "post".to_string(),
                path: "/avatar/direct".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<crate::storage::UploadedFile>()
                    .response::<()>(204),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/gallery/direct".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<Vec<crate::storage::UploadedFile>>()
                    .response::<()>(204),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/avatar/optional".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<Option<crate::storage::UploadedFile>>()
                    .response::<()>(204),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/gallery/optional".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<Option<Vec<crate::storage::UploadedFile>>>()
                    .response::<()>(204),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/avatar".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<MultipartAvatarRequest>()
                    .response::<()>(204),
            },
            DocumentedRoute {
                method: "post".to_string(),
                path: "/gallery".to_string(),
                doc: RouteDoc::new()
                    .post()
                    .request::<MultipartGalleryRequest>()
                    .response::<()>(204),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "1.0.0", &routes);
        let direct_avatar_content =
            &spec["paths"]["/avatar/direct"]["post"]["requestBody"]["content"];
        let direct_gallery_content =
            &spec["paths"]["/gallery/direct"]["post"]["requestBody"]["content"];
        let optional_avatar_schema = &spec["paths"]["/avatar/optional"]["post"]["requestBody"]
            ["content"]["multipart/form-data"]["schema"];
        let optional_gallery_schema = &spec["paths"]["/gallery/optional"]["post"]["requestBody"]
            ["content"]["multipart/form-data"]["schema"];
        let avatar_content = &spec["paths"]["/avatar"]["post"]["requestBody"]["content"];
        let gallery_content = &spec["paths"]["/gallery"]["post"]["requestBody"]["content"];

        assert!(direct_avatar_content.get("application/json").is_none());
        assert_eq!(
            direct_avatar_content["multipart/form-data"]["schema"],
            serde_json::json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string", "format": "binary" }
                },
                "required": ["file"]
            })
        );
        assert!(direct_gallery_content.get("application/json").is_none());
        assert_eq!(
            direct_gallery_content["multipart/form-data"]["schema"],
            serde_json::json!({
                "type": "object",
                "properties": {
                    "file": {
                        "type": "array",
                        "items": { "type": "string", "format": "binary" },
                        "x-foundry-item-schema": "UploadedFile"
                    }
                },
                "required": ["file"]
            })
        );
        assert_eq!(
            optional_avatar_schema["properties"]["file"],
            serde_json::json!({"type": "string", "format": "binary", "nullable": true})
        );
        assert!(optional_avatar_schema.get("required").is_none());
        assert_eq!(
            optional_gallery_schema["properties"]["file"],
            serde_json::json!({
                "type": "array",
                "items": { "type": "string", "format": "binary" },
                "x-foundry-item-schema": "UploadedFile",
                "nullable": true
            })
        );
        assert!(optional_gallery_schema.get("required").is_none());
        assert!(avatar_content.get("application/json").is_none());
        assert_eq!(
            avatar_content["multipart/form-data"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/MultipartAvatarRequest")
        );
        assert_eq!(
            spec["components"]["schemas"]["MultipartAvatarRequest"]["properties"]["avatar"],
            serde_json::json!({"type": "string", "format": "binary"})
        );
        assert!(gallery_content.get("application/json").is_none());
        assert_eq!(
            gallery_content["multipart/form-data"]["schema"]["$ref"],
            serde_json::json!("#/components/schemas/MultipartGalleryRequest")
        );
        assert_eq!(
            spec["components"]["schemas"]["MultipartGalleryRequest"]["properties"]["photos"],
            serde_json::json!({
                "type": "array",
                "items": { "type": "string", "format": "binary" },
                "x-foundry-item-schema": "UploadedFile"
            })
        );
    }
}
