use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::contract::{
    ContractAction, ContractAuth, ContractHttpBody, ContractHttpTransport, ContractManifest,
    ContractPayload, ContractResponse, ContractSchema, ContractTransport,
};
use crate::http::route_path_params;

use super::RouteDoc;

pub struct DocumentedRoute {
    pub method: String,
    pub path: String,
    pub doc: RouteDoc,
}

pub fn generate_openapi_spec(title: &str, version: &str, routes: &[DocumentedRoute]) -> Value {
    let manifest = contract_manifest_from_documented_routes(routes);
    generate_openapi_spec_from_contract(title, version, &manifest)
}

pub fn generate_openapi_spec_from_contract(
    title: &str,
    version: &str,
    manifest: &ContractManifest,
) -> Value {
    let mut paths: BTreeMap<String, Value> = BTreeMap::new();
    let schemas = manifest
        .schemas
        .iter()
        .map(|schema| (schema.name.clone(), schema.schema.clone()))
        .collect::<BTreeMap<_, _>>();

    for action in &manifest.actions {
        let ContractTransport::Http(http) = &action.transport else {
            continue;
        };
        let method = http.method.clone().unwrap_or_else(|| "get".to_string());
        let mut operation = json!({});
        if let Some(ref summary) = action.summary {
            operation["summary"] = json!(summary);
        }
        if let Some(ref description) = action.description {
            operation["description"] = json!(description);
        }
        if !action.tags.is_empty() {
            operation["tags"] = json!(action.tags);
        }
        if action.deprecated {
            operation["deprecated"] = json!(true);
        }
        if let Some(guard) = &action.auth.guard {
            operation["x-foundry-guard"] = json!(guard);
        }
        if !action.auth.permissions.is_empty() {
            operation["x-foundry-permissions"] = json!(action.auth.permissions);
        }

        if let Some(req) = action
            .request
            .as_ref()
            .filter(|_| http.body != ContractHttpBody::None)
        {
            let content_type = match http.body {
                ContractHttpBody::Multipart => "multipart/form-data",
                ContractHttpBody::Json | ContractHttpBody::Unknown => "application/json",
                ContractHttpBody::None => unreachable!("body kind filtered above"),
            };
            operation["requestBody"] = json!({
                "required": true,
                "content": {
                    content_type: {
                        "schema": {
                            "$ref": format!("#/components/schemas/{}", req.schema)
                        }
                    }
                }
            });
        }

        if !action.responses.is_empty() {
            let mut responses = json!({});
            for response in &action.responses {
                responses[response.status.to_string()] = json!({
                    "description": "",
                    "content": {
                        "application/json": {
                            "schema": {
                                "$ref": format!("#/components/schemas/{}", response.schema)
                            }
                        }
                    }
                });
            }
            operation["responses"] = responses;
        }

        let path_entry = paths.entry(http.path.clone()).or_insert_with(|| json!({}));
        path_entry[&method] = operation;
    }

    json!({
        "openapi": "3.1.0",
        "info": { "title": title, "version": version },
        "paths": paths,
        "components": { "schemas": schemas }
    })
}

fn contract_manifest_from_documented_routes(routes: &[DocumentedRoute]) -> ContractManifest {
    let mut schema_map = BTreeMap::<String, Value>::new();
    let mut actions = Vec::new();

    for (index, route) in routes.iter().enumerate() {
        let request = route.doc.request.as_ref().map(|schema| {
            schema_map.insert(schema.name.to_string(), (schema.schema_fn)());
            ContractPayload {
                schema: schema.name.to_string(),
            }
        });

        let responses = route
            .doc
            .responses
            .iter()
            .map(|(status, schema)| {
                schema_map.insert(schema.name.to_string(), (schema.schema_fn)());
                ContractResponse {
                    status: *status,
                    schema: schema.name.to_string(),
                    schema_json: (schema.schema_fn)(),
                }
            })
            .collect::<Vec<_>>();

        actions.push(ContractAction {
            id: format!("{} {}", route.method, route.path),
            action_name: format!("OpenApiRoute{}", index + 1),
            summary: route.doc.summary.clone(),
            description: route.doc.description.clone(),
            tags: route.doc.tags.clone(),
            deprecated: route.doc.deprecated,
            request,
            responses,
            auth: ContractAuth::default(),
            client_export: false,
            validation: None,
            transport: ContractTransport::Http(ContractHttpTransport {
                method: Some(route.method.clone()),
                path: route.path.clone(),
                path_params: route_path_params(&route.path),
                body: if route.doc.request.is_some() {
                    ContractHttpBody::Json
                } else {
                    ContractHttpBody::None
                },
            }),
        });
    }

    let mut manifest = ContractManifest::new()
        .with_schemas(
            schema_map
                .into_iter()
                .map(|(name, schema)| ContractSchema { name, schema })
                .collect(),
        )
        .with_validation_schemas(Vec::new())
        .with_realtime_channels(Vec::new());
    manifest.actions = actions;
    manifest
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn bodyless_contract_request_does_not_emit_request_body() {
        let mut manifest = ContractManifest::new().with_schemas(vec![ContractSchema {
            name: "ShowUserRequest".to_string(),
            schema: json!({ "type": "object" }),
        }]);
        manifest.actions = vec![ContractAction {
            id: "admin.users.show".to_string(),
            action_name: "AdminUsersShow".to_string(),
            summary: None,
            description: None,
            tags: Vec::new(),
            deprecated: false,
            request: Some(ContractPayload {
                schema: "ShowUserRequest".to_string(),
            }),
            responses: Vec::new(),
            auth: ContractAuth::default(),
            client_export: true,
            validation: Some("ShowUserRequest".to_string()),
            transport: ContractTransport::Http(ContractHttpTransport {
                method: Some("get".to_string()),
                path: "/users/{id}".to_string(),
                path_params: vec!["id".to_string()],
                body: ContractHttpBody::None,
            }),
        }];

        let spec = generate_openapi_spec_from_contract("Foundry", "test", &manifest);

        assert!(
            spec["paths"]["/users/{id}"]["get"]
                .get("requestBody")
                .is_none(),
            "bodyless contract action should not emit OpenAPI requestBody: {spec}"
        );
    }
}
