use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::RouteDoc;

pub struct DocumentedRoute {
    pub method: String,
    pub path: String,
    pub doc: RouteDoc,
}

pub fn generate_openapi_spec(title: &str, version: &str, routes: &[DocumentedRoute]) -> Value {
    let mut paths: BTreeMap<String, Value> = BTreeMap::new();
    let mut schemas: BTreeMap<String, Value> = BTreeMap::new();

    for route in routes {
        let mut operation = json!({});
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

        if let Some(ref req) = route.doc.request {
            let schema = (req.schema_fn)();
            schemas.insert(req.name.to_string(), schema);
            operation["requestBody"] = json!({
                "required": true,
                "content": {
                    "application/json": {
                        "schema": {
                            "$ref": format!("#/components/schemas/{}", req.name)
                        }
                    }
                }
            });
        }

        if !route.doc.responses.is_empty() {
            let mut responses = json!({});
            for (status, schema_ref) in &route.doc.responses {
                let schema = (schema_ref.schema_fn)();
                schemas.insert(schema_ref.name.to_string(), schema);
                responses[status.to_string()] = json!({
                    "description": "",
                    "content": {
                        "application/json": {
                            "schema": {
                                "$ref": format!("#/components/schemas/{}", schema_ref.name)
                            }
                        }
                    }
                });
            }
            operation["responses"] = responses;
        }

        let path_entry = paths.entry(route.path.clone()).or_insert_with(|| json!({}));
        path_entry[&route.method] = operation;
    }

    json!({
        "openapi": "3.1.0",
        "info": { "title": title, "version": version },
        "paths": paths,
        "components": { "schemas": schemas }
    })
}
